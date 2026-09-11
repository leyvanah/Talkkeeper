//! Encoding a track while it is being recorded, rather than after.
//!
//! A recording used to be written in 30-second pieces and then, at Stop,
//! decoded and encoded again in one pass over the whole thing — three times
//! over, once per track. On an hour-long recording that is minutes of waiting
//! after the last word, for work that could have been done as it went.
//!
//! Here one encoder runs for the whole recording: samples go in as they are
//! captured, encoded bytes come back out, and Stop only has to close the pipe.
//!
//! ## Why the bytes come back to us instead of FFmpeg writing the file
//!
//! Every byte of the recording passes through one place — [`Sink::write`] —
//! which is where encryption sits (B3), without the encoder, the pipeline or
//! the recording saver knowing about it.
//!
//! Writing to a pipe means the container cannot be revised afterwards, so MP4
//! is written in fragments. That is also what makes an interrupted recording
//! survive: a fragmented file is playable up to its last complete fragment,
//! whereas an ordinary MP4 cut short is not playable at all — its index is
//! written last. This replaces the checkpoint pieces that used to provide
//! crash recovery.
//!
//! ## Publication
//!
//! Bytes go to `<name>.part` and it is renamed to `<name>` only when the
//! encoder has closed cleanly, so nothing ever reads a half-written track.

use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use log::{info, warn};

use super::ffmpeg::find_ffmpeg_path;

/// Extension of a track still being written. Never read.
pub const PARTIAL_EXT: &str = "part";

/// How much of FFmpeg's complaining to keep for the error message.
const STDERR_KEPT_BYTES: usize = 4096;

/// What to encode into, and what to call the result.
#[derive(Clone, Copy)]
pub struct EncodeFormat {
    /// Codec arguments, as passed to FFmpeg.
    pub codec_args: &'static [&'static str],
    /// Muxer name for `-f`.
    pub container: &'static str,
}

/// The delivery format: AAC-LC in MP4, played wherever the app shows audio.
///
/// Fragmented, because it is written to a pipe and because an interrupted
/// recording has to remain playable. `empty_moov` starts the file with an
/// empty index and `default_base_moof` makes each fragment self-describing.
///
/// **`frag_duration` is what closes the fragments, and it is not optional.**
/// A fragment is otherwise cut at a video keyframe, and a recording has no
/// video: FFmpeg then holds the whole thing open, and past about a minute it
/// gives up and writes a fragment header with a length of zero. The file that
/// comes out has the right size and the right duration and almost none of it
/// decodes — which is exactly what happened to the first real recording made
/// this way. Two seconds also bounds what an interrupted recording loses and
/// what the encoder holds in memory while it runs.
pub const MP4_AAC: EncodeFormat = EncodeFormat {
    codec_args: &[
        "-c:a",
        "aac",
        "-b:a",
        "192k",
        "-profile:a",
        "aac_low",
        "-movflags",
        "empty_moov+default_base_moof",
        "-frag_duration",
        "2000000",
    ],
    container: "mp4",
};

/// The working-track format: lossless, and self-framing, so a truncated file
/// still decodes up to the cut.
pub const FLAC: EncodeFormat = EncodeFormat {
    codec_args: &["-c:a", "flac", "-sample_fmt", "s16"],
    container: "flac",
};

/// Where a track's encoded bytes go.
///
/// In the application that is a file, encrypted or not; a trait so that the
/// encryption sits in front of the file without anything above knowing, and so
/// tests can hold a track in memory.
pub trait Sink: Send {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()>;
    fn finish(&mut self) -> std::io::Result<()>;
}

/// The file a track is written to, encrypted when the archive has a key (B3).
/// Which it is, is decided in one place — see
/// [`super::encrypted_audio::AudioSink`] — so nothing here has to know.
impl Sink for super::encrypted_audio::AudioSink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        std::io::Write::write_all(self, bytes)
    }

    fn finish(&mut self) -> std::io::Result<()> {
        super::encrypted_audio::AudioSink::finish(self)
    }
}

/// One track being encoded as it is recorded.
pub struct StreamingEncoder {
    label: String,
    child: Child,
    stdin: Option<ChildStdin>,
    /// Moves encoded bytes from FFmpeg to the sink. Must keep running: if
    /// nothing drains the pipe, FFmpeg stops writing and then stops reading,
    /// and the recording stalls behind it.
    output: Option<JoinHandle<std::io::Result<u64>>>,
    /// Drains FFmpeg's diagnostics for the same reason, keeping the tail for
    /// the error message.
    errors: Option<JoinHandle<String>>,
    partial_path: PathBuf,
    final_path: PathBuf,
    samples_written: u64,
    sample_rate: u32,
}

impl StreamingEncoder {
    /// Start encoding into `final_path`, which is written under a temporary
    /// name until [`StreamingEncoder::finish`] succeeds.
    pub fn start(
        label: impl Into<String>,
        final_path: PathBuf,
        sample_rate: u32,
        channels: u16,
        format: EncodeFormat,
    ) -> Result<Self> {
        let partial_path = partial_path_for(&final_path);
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = File::create(&partial_path)?;
        Self::start_into(
            label,
            Box::new(super::encrypted_audio::AudioSink::into_file(file)?),
            partial_path,
            final_path,
            sample_rate,
            channels,
            format,
        )
    }

    fn start_into(
        label: impl Into<String>,
        mut sink: Box<dyn Sink>,
        partial_path: PathBuf,
        final_path: PathBuf,
        sample_rate: u32,
        channels: u16,
        format: EncodeFormat,
    ) -> Result<Self> {
        let label = label.into();
        let ffmpeg_path = find_ffmpeg_path()
            .ok_or_else(|| anyhow!("FFmpeg not found — cannot record audio"))?;

        let mut command = Command::new(ffmpeg_path);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "f32le",
                "-ar",
                &sample_rate.to_string(),
                "-ac",
                &channels.to_string(),
                "-i",
                "pipe:0",
            ])
            .args(format.codec_args)
            .args(["-f", format.container, "pipe:1"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Hide the console window on Windows, as everywhere else FFmpeg is spawned.
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("Failed to open the FFmpeg input pipe"))?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("Failed to open the FFmpeg output pipe"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("Failed to open the FFmpeg error pipe"))?;

        let output = std::thread::spawn(move || -> std::io::Result<u64> {
            let mut buffer = vec![0u8; 1 << 16];
            let mut total = 0u64;
            loop {
                match stdout.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        sink.write(&buffer[..read])?;
                        total += read as u64;
                    }
                    Err(error) => return Err(error),
                }
            }
            sink.finish()?;
            Ok(total)
        });

        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let mut buffer = vec![0u8; 4096];
            while let Ok(read) = stderr.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                text.push_str(&String::from_utf8_lossy(&buffer[..read]));
                if text.len() > STDERR_KEPT_BYTES * 2 {
                    text = text.split_off(text.len() - STDERR_KEPT_BYTES);
                }
            }
            text
        });

        Ok(Self {
            label,
            child,
            stdin: Some(stdin),
            output: Some(output),
            errors: Some(errors),
            partial_path,
            final_path,
            samples_written: 0,
            sample_rate,
        })
    }

    /// Hand one block of samples to the encoder.
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("Encoder for {} is already closed", self.label))?;
        stdin.write_all(bytemuck::cast_slice(samples))?;
        self.samples_written += samples.len() as u64;
        Ok(())
    }

    /// Where the track will be published, once it is finished.
    pub fn final_path(&self) -> &Path {
        &self.final_path
    }

    /// Seconds of audio handed over so far, per channel.
    pub fn seconds_written(&self, channels: u16) -> f64 {
        self.samples_written as f64 / (self.sample_rate as f64 * channels.max(1) as f64)
    }

    /// Close the encoder and publish the track under its final name.
    ///
    /// On failure whatever was encoded is left under the temporary name and
    /// the caller decides: a recording is worth publishing in part
    /// ([`publish_partial`]), a derived track is not ([`discard_partial`]).
    pub fn finish(mut self) -> Result<PathBuf> {
        let complaints = self.close()?;
        let status = self.child.wait()?;
        if !status.success() {
            return Err(anyhow!(
                "FFmpeg failed to encode the {} track: {}",
                self.label,
                complaints.trim()
            ));
        }
        if self.final_path.exists() {
            std::fs::remove_file(&self.final_path)?;
        }
        std::fs::rename(&self.partial_path, &self.final_path)?;
        Ok(self.final_path.clone())
    }

    /// Give up on this track, leaving nothing for a later pass to read.
    pub fn abandon(mut self) {
        let _ = self.close();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.partial_path);
    }

    /// Close the input and wait for everything already sent to be written out.
    fn close(&mut self) -> Result<String> {
        drop(self.stdin.take());
        if let Some(output) = self.output.take() {
            output
                .join()
                .map_err(|_| anyhow!("The {} output thread panicked", self.label))??;
        }
        Ok(self
            .errors
            .take()
            .and_then(|errors| errors.join().ok())
            .unwrap_or_default())
    }
}

/// Give an unfinished track its final name, keeping what was encoded.
///
/// For a recording this is nearly always right: fifty minutes of a session is
/// worth incomparably more than nothing, and the fragmented container means
/// what was written plays.
pub fn publish_partial(final_path: &Path) -> Result<PathBuf> {
    let partial = partial_path_for(final_path);
    let written = std::fs::metadata(&partial).map(|m| m.len()).unwrap_or(0);
    if written == 0 {
        return Err(anyhow!("Nothing was encoded to {}", partial.display()));
    }
    if final_path.exists() {
        std::fs::remove_file(final_path)?;
    }
    std::fs::rename(&partial, final_path)?;
    Ok(final_path.to_path_buf())
}

/// Throw an unfinished track away. For audio derived from a recording, where
/// a partial copy would be read as if it were the whole of it.
pub fn discard_partial(final_path: &Path) {
    let partial = partial_path_for(final_path);
    if partial.exists() {
        if let Err(error) = std::fs::remove_file(&partial) {
            warn!("Could not discard {}: {error}", partial.display());
        }
    }
}

/// The name a track is written under while it is unfinished.
pub fn partial_path_for(final_path: &Path) -> PathBuf {
    let mut name = final_path.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(PARTIAL_EXT);
    final_path.with_file_name(name)
}

/// Publish tracks left unfinished by a recording that never closed.
///
/// The fragmented container means what was written is playable, so unlike the
/// checkpoint pieces it replaces, recovery is a rename. Returns the tracks
/// recovered.
pub fn recover_partial_tracks(folder: &Path) -> Vec<PathBuf> {
    let mut recovered = Vec::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return recovered;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.extension().and_then(|ext| ext.to_str()) != Some(PARTIAL_EXT) {
            continue;
        }
        let Some(final_path) = path.file_stem().map(|stem| path.with_file_name(stem)) else {
            continue;
        };
        if final_path.exists() {
            // The finished track is the better copy; the leftover is not.
            let _ = std::fs::remove_file(&path);
            continue;
        }
        // An encoder that never wrote a fragment leaves nothing playable.
        if std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) == 0 {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        match std::fs::rename(&path, &final_path) {
            Ok(()) => {
                info!("Recovered interrupted track {}", final_path.display());
                recovered.push(final_path);
            }
            Err(error) => warn!("Could not recover {}: {error}", path.display()),
        }
    }
    recovered
}

/// What FFmpeg complains about while decoding every frame of a file.
///
/// A track can carry the right length and still hold broken audio: that is
/// what a player refuses and what a duration check does not see.
#[cfg(test)]
pub(crate) fn decode_complaints(path: &Path) -> Vec<String> {
    let Some(ffmpeg) = find_ffmpeg_path() else {
        return Vec::new();
    };
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "warning", "-i"])
        .arg(path)
        .args(["-f", "null", "-"])
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let Ok(output) = command.output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .filter(|line| {
            let line = line.to_ascii_lowercase();
            line.contains("invalid") || line.contains("error") || line.contains("[aac")
        })
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::decoder::decode_audio_file;

    fn ffmpeg_missing() -> bool {
        if find_ffmpeg_path().is_none() {
            eprintln!("skipping: FFmpeg is not installed");
            return true;
        }
        false
    }

    fn tone(sample_rate: u32, seconds: f32) -> Vec<f32> {
        let total = (sample_rate as f32 * seconds) as usize;
        (0..total)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.3
            })
            .collect()
    }

    /// Feed `seconds` of tone in 50 ms blocks, as the pipeline does.
    fn record(encoder: &mut StreamingEncoder, sample_rate: u32, seconds: f32) {
        let block = (sample_rate / 20) as usize;
        for chunk in tone(sample_rate, seconds).chunks(block) {
            encoder.write(chunk).expect("write");
        }
    }

    #[test]
    fn a_recorded_track_is_readable_by_what_reads_recordings() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let mut encoder =
            StreamingEncoder::start("mixed", path.clone(), 48000, 1, MP4_AAC).expect("start");
        record(&mut encoder, 48000, 2.0);
        assert_eq!(encoder.finish().expect("finish"), path);

        // decode_audio_file is the reader used by retranscription and import;
        // it must take the file as written, without a conversion step.
        let decoded = decode_audio_file(&path).expect("decode");
        assert_eq!(decoded.sample_rate, 48000);
        assert_eq!(decoded.channels, 1);
        assert!(
            (decoded.duration_seconds - 2.0).abs() < 0.15,
            "expected about two seconds, got {:.3}s",
            decoded.duration_seconds
        );
    }

    /// The same recording, written with a key: what lands on disk must not be
    /// an MP4 at all, and must come back as one byte for byte once decrypted.
    #[test]
    fn a_recorded_track_is_encrypted_when_the_archive_has_a_key() {
        use crate::audio::encrypted_audio::{AudioSink, AudioSource};
        use crate::security::envelope::generate_dek;
        use crate::security::stream::looks_encrypted;
        use std::io::Read as _;

        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let key = generate_dek();

        // `start_into` is what `start` calls; the sink is keyed explicitly here
        // because a process-wide key installed by a test would follow the whole
        // test run around.
        let partial = partial_path_for(&path);
        let sink = AudioSink::create_with_key(&partial, key.as_ref()).expect("sink");
        let mut encoder = StreamingEncoder::start_into(
            "mixed",
            Box::new(sink),
            partial,
            path.clone(),
            48000,
            1,
            MP4_AAC,
        )
        .expect("start");
        record(&mut encoder, 48000, 2.0);
        assert_eq!(encoder.finish().expect("finish"), path);

        let on_disk = std::fs::read(&path).unwrap();
        assert!(looks_encrypted(&on_disk), "the track was written in the clear");
        assert!(
            !on_disk.windows(4).any(|w| w == b"ftyp"),
            "an MP4 header is visible in the encrypted track"
        );

        // Decrypted, it is the same recording the plaintext test decodes.
        let mut source = AudioSource::open_with_key(&path, key.as_ref()).expect("open");
        let mut decrypted = Vec::new();
        source.read_to_end(&mut decrypted).unwrap();
        let plain = dir.path().join("plain.mp4");
        std::fs::write(&plain, &decrypted).unwrap();

        let decoded = decode_audio_file(&plain).expect("decode");
        assert_eq!(decoded.sample_rate, 48000);
        assert_eq!(decoded.channels, 1);
        assert!(
            (decoded.duration_seconds - 2.0).abs() < 0.15,
            "expected about two seconds, got {:.3}s",
            decoded.duration_seconds
        );
    }

    /// The recording has to hold sound, not just a length. Written the way the
    /// pipeline writes it: one window at a time, for the whole recording.
    ///
    /// **The length matters.** The defect this guards against does not appear
    /// until a recording runs past about a minute, which is why the first
    /// version of this test — twenty seconds — passed while the first real
    /// recording came out undecodable.
    #[test]
    fn a_recorded_track_holds_undamaged_audio() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let mut encoder =
            StreamingEncoder::start("mixed", path.clone(), 48000, 1, MP4_AAC).expect("start");
        record(&mut encoder, 48000, 100.0);
        encoder.finish().expect("finish");

        let complaints = decode_complaints(&path);
        assert!(
            complaints.is_empty(),
            "the recording decodes with {} complaints, first: {}",
            complaints.len(),
            complaints.first().map(String::as_str).unwrap_or("")
        );
    }

    #[test]
    fn a_lossless_track_is_readable_too() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.flac");
        let mut encoder =
            StreamingEncoder::start("microphone", path.clone(), 16000, 1, FLAC).expect("start");
        record(&mut encoder, 16000, 1.0);
        encoder.finish().expect("finish");

        let decoded = decode_audio_file(&path).expect("decode");
        assert_eq!(decoded.sample_rate, 16000);
        assert!((decoded.duration_seconds - 1.0).abs() < 0.05);
    }

    #[test]
    fn nothing_is_published_under_the_final_name_until_it_is_finished() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let mut encoder =
            StreamingEncoder::start("mixed", path.clone(), 48000, 1, MP4_AAC).expect("start");
        record(&mut encoder, 48000, 0.5);

        assert!(!path.exists(), "an unfinished track must not be readable");
        assert!(partial_path_for(&path).exists());

        encoder.finish().expect("finish");
        assert!(path.exists());
        assert!(!partial_path_for(&path).exists());
    }

    #[test]
    fn an_abandoned_track_leaves_nothing_behind() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.mp4");
        let mut encoder =
            StreamingEncoder::start("microphone", path.clone(), 48000, 1, MP4_AAC).expect("start");
        record(&mut encoder, 48000, 0.3);
        encoder.abandon();

        assert!(!path.exists());
        assert!(!partial_path_for(&path).exists());
    }

    /// What a crash leaves: the encoder never closed, so the track is still
    /// under its temporary name. Recovery has to hand back playable audio.
    #[test]
    fn an_interrupted_recording_is_recovered_and_still_plays() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let mut encoder =
            StreamingEncoder::start("mixed", path.clone(), 48000, 1, MP4_AAC).expect("start");
        record(&mut encoder, 48000, 2.0);

        // Close the input the way a killed process would: the samples already
        // sent are encoded and written, but nothing is published.
        let complaints = encoder.close().expect("close");
        assert!(complaints.trim().is_empty(), "FFmpeg complained: {complaints}");
        assert!(encoder.child.wait().expect("wait").success());
        drop(encoder);

        let recovered = recover_partial_tracks(dir.path());
        assert_eq!(recovered, vec![path.clone()]);

        let decoded = decode_audio_file(&path).expect("the recovered track must decode");
        assert!(
            decoded.duration_seconds > 1.5,
            "recovered only {:.3}s of two",
            decoded.duration_seconds
        );
    }

    /// What Stop costs now against what it cost before: closing a pipe over a
    /// recording that is already encoded, against encoding it from the start.
    /// Opt-in because the numbers only mean something in a release build:
    ///   cargo test --release --lib streaming_encoder -- --ignored --nocapture
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn stop_cost_against_re_encoding_the_recording() {
        if ffmpeg_missing() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let sample_rate = 48000u32;
        let minutes = 10.0;
        let audio = tone(sample_rate, 60.0 * minutes);

        // As it happens now: encoded during the recording, so Stop is the tail.
        let streamed = dir.path().join("streamed.mp4");
        let mut encoder =
            StreamingEncoder::start("mixed", streamed, sample_rate, 1, MP4_AAC).expect("start");
        let block = (sample_rate / 20) as usize;
        for chunk in audio.chunks(block) {
            encoder.write(chunk).expect("write");
        }
        let stop = std::time::Instant::now();
        encoder.finish().expect("finish");
        let closing = stop.elapsed().as_secs_f64();

        // As it happened before: one pass over the whole recording at Stop.
        // (The old path also decoded the pieces first, which this leaves out,
        // so the comparison is generous to it.)
        let reference = dir.path().join("reference.mp4");
        let started = std::time::Instant::now();
        crate::audio::encode::encode_single_audio(
            bytemuck::cast_slice(&audio),
            sample_rate,
            1,
            &reference,
        )
        .expect("encode");
        let re_encoding = started.elapsed().as_secs_f64();

        println!(
            "stop cost for {minutes:.0} minutes of one track: {closing:.2} s closing the encoder, \
             against {re_encoding:.2} s to encode it again ({:.0}x)",
            re_encoding / closing.max(0.001)
        );
    }

    #[test]
    fn recovery_keeps_the_finished_track_over_the_leftover() {
        let dir = tempfile::tempdir().unwrap();
        let finished = dir.path().join("audio.mp4");
        std::fs::write(&finished, b"the whole recording").unwrap();
        std::fs::write(partial_path_for(&finished), b"a leftover").unwrap();

        assert!(recover_partial_tracks(dir.path()).is_empty());
        assert_eq!(std::fs::read(&finished).unwrap(), b"the whole recording");
        assert!(!partial_path_for(&finished).exists());
    }

    #[test]
    fn recovery_ignores_a_track_that_holds_no_audio() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mic.mp4");
        std::fs::write(partial_path_for(&path), b"").unwrap();

        assert!(recover_partial_tracks(dir.path()).is_empty());
        assert!(!path.exists());
        assert!(!partial_path_for(&path).exists());
    }
}
