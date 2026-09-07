//! The 16 kHz working track: the audio every later pass actually listens to.
//!
//! Recognition, voice activity detection and diarization all work at 16 kHz
//! mono. The delivery tracks are 48 kHz AAC, so every pass over a finished
//! recording used to begin by decoding them and resampling the result — on an
//! hour-long session that is minutes of work before the first word is
//! recognized, repeated for every retranscription.
//!
//! The live pipeline already has to produce that 16 kHz stream to run VAD.
//! Writing it down as it goes costs almost nothing during the recording and
//! removes the whole decode-and-resample stage afterwards.
//!
//! ## Guarantees the rest of the code relies on
//!
//! * The file is **published by rename**: samples go to `<track>.flac.part`
//!   and it becomes `<track>.flac` only when the recording closed cleanly. A
//!   crash or an encoder failure therefore leaves no working track at all
//!   rather than half of one, and readers fall back to the delivery track.
//! * The content is the same audio as the delivery track of the same name —
//!   the same windows, after echo cancellation and system gain — at 16 kHz.
//! * FLAC, so the track is lossless: what is stored is what the live pass
//!   heard, and a retranscription cannot be worse for having read it.

use std::path::{Path, PathBuf};

use anyhow::Result;
use log::{info, warn};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};

use super::streaming_encoder::{self, StreamingEncoder};

/// Every pass over stored audio runs at this rate; so does the working track.
pub const WORKING_SAMPLE_RATE: u32 = 16000;

/// Subfolder of the meeting folder holding derived audio. Named like
/// `.checkpoints`: not part of the recording, safe to delete, rebuildable.
const WORKING_DIR: &str = ".work";

/// Extension of a finished working track.
const WORKING_EXT: &str = "flac";

/// Extension of one still being written. Never read.
const PARTIAL_EXT: &str = "flac.part";

/// FLAC at the working rate, written as the recording runs.
const WORKING_FORMAT: streaming_encoder::EncodeFormat = streaming_encoder::FLAC;

/// Where the working track for `track` ("mic" / "system") lives.
pub fn working_track_path(meeting_folder: &Path, track: &str) -> PathBuf {
    meeting_folder
        .join(WORKING_DIR)
        .join(format!("{track}.{WORKING_EXT}"))
}

/// The working track for `track`, if this recording finished writing one.
pub fn find_working_track(meeting_folder: &Path, track: &str) -> Option<PathBuf> {
    let path = working_track_path(meeting_folder, track);
    path.is_file().then_some(path)
}

/// Throw away tracks left half-written by a recording that never closed.
///
/// Nothing reads them — they never got their final name — so all they would do
/// is hold an hour of unaccounted-for speech on disk. Called when a recording
/// is recovered, which is the moment we know one was interrupted.
pub fn discard_partial_tracks(meeting_folder: &Path) {
    let Ok(entries) = std::fs::read_dir(meeting_folder.join(WORKING_DIR)) else {
        return;
    };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.to_string_lossy().ends_with(PARTIAL_EXT) {
            match std::fs::remove_file(&path) {
                Ok(()) => info!("Discarded the unfinished working track {}", path.display()),
                Err(error) => warn!("Could not discard {}: {error}", path.display()),
            }
        }
    }
}

/// Streaming conversion of the capture rate to the working rate.
///
/// The resampler is persistent and fed fixed-size chunks: creating one per
/// window would restart its filter state at every window and leave a step in
/// the audio at each boundary. The parameters match the one-shot resampler
/// used on stored files (`audio_processing::resample`), so a retranscription
/// reads the same audio it would otherwise have computed for itself.
pub struct WorkingRateResampler {
    /// `None` when the input is already at the working rate.
    resampler: Option<SincFixedIn<f32>>,
    input_rate: u32,
    /// Fixed number of input samples per call into the resampler.
    input_chunk: usize,
    /// Input samples not yet forming a whole chunk.
    pending: Vec<f32>,
}

impl WorkingRateResampler {
    pub fn new(input_rate: u32, input_chunk: usize) -> Result<Self> {
        let input_chunk = input_chunk.max(1);
        let resampler = if input_rate == WORKING_SAMPLE_RATE {
            None
        } else {
            let ratio = WORKING_SAMPLE_RATE as f64 / input_rate as f64;
            // The same shape as the anti-aliased downsampling branch of
            // `audio_processing::resample`: a long sinc for the stopband,
            // cubic interpolation between table entries, Blackman-Harris.
            let params = SincInterpolationParameters {
                sinc_len: 512,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Cubic,
                oversampling_factor: 512,
                window: WindowFunction::BlackmanHarris2,
            };
            Some(SincFixedIn::<f32>::new(ratio, 2.0, params, input_chunk, 1)?)
        };

        Ok(Self {
            resampler,
            input_rate,
            input_chunk,
            pending: Vec::with_capacity(input_chunk * 2),
        })
    }

    /// How many working-rate samples `input_len` capture samples come to.
    fn output_len_for(&self, input_len: usize) -> usize {
        (input_len as u64 * WORKING_SAMPLE_RATE as u64 / self.input_rate as u64) as usize
    }

    /// Convert everything that forms a whole chunk. A remainder shorter than
    /// one chunk waits for the next window instead of being resampled on its
    /// own, which is what keeps the stream continuous.
    pub fn push(&mut self, samples: &[f32]) -> Vec<f32> {
        if self.resampler.is_none() {
            return samples.to_vec();
        }

        self.pending.extend_from_slice(samples);
        let mut output = Vec::with_capacity(self.output_len_for(self.pending.len()) + 16);
        let Some(resampler) = self.resampler.as_mut() else {
            return output;
        };
        while self.pending.len() >= self.input_chunk {
            let chunk: Vec<f32> = self.pending.drain(..self.input_chunk).collect();
            match resampler.process(&[chunk], None) {
                Ok(mut converted) => {
                    if let Some(channel) = converted.pop() {
                        output.extend_from_slice(&channel);
                    }
                }
                Err(error) => warn!("Working-rate resampler failed on a window: {error}"),
            }
        }
        output
    }

    /// Convert the remainder at the end of a recording. The chunk is completed
    /// with silence because the resampler takes fixed-size input; the silence
    /// is then cut back off, so the track ends where the recording does.
    pub fn drain(&mut self) -> Vec<f32> {
        if self.pending.is_empty() {
            return Vec::new();
        }
        let real_len = self.pending.len();
        let keep = self.output_len_for(real_len);
        let padding = self.input_chunk - (real_len % self.input_chunk);
        self.pending.resize(real_len + padding, 0.0);
        let mut output = self.push(&[]);
        self.pending.clear();
        output.truncate(keep);
        output
    }
}

/// One capture source, converted to the working rate and — when the recording
/// is being kept — written down as it goes.
pub struct WorkingTrack {
    label: &'static str,
    resampler: WorkingRateResampler,
    /// `None` when nothing is being saved, or once writing has failed.
    writer: Option<StreamingEncoder>,
    samples_written: u64,
}

impl WorkingTrack {
    /// `path` is `None` when the recording is not being saved — the conversion
    /// still runs, because VAD needs it either way.
    pub fn new(
        label: &'static str,
        input_rate: u32,
        input_chunk: usize,
        path: Option<PathBuf>,
    ) -> Result<Self> {
        let resampler = WorkingRateResampler::new(input_rate, input_chunk)?;
        let writer = match path {
            Some(path) => match StreamingEncoder::start(
                label,
                path,
                WORKING_SAMPLE_RATE,
                1,
                WORKING_FORMAT,
            ) {
                Ok(writer) => {
                    info!("📝 Working track for {label} opened at {WORKING_SAMPLE_RATE} Hz");
                    Some(writer)
                }
                // Not fatal: later passes fall back to the delivery track.
                Err(error) => {
                    warn!("Could not open the {label} working track: {error}");
                    None
                }
            },
            None => None,
        };

        Ok(Self {
            label,
            resampler,
            writer,
            samples_written: 0,
        })
    }

    /// Convert one capture window, store it, and hand it back for VAD.
    pub fn push(&mut self, window: &[f32]) -> Vec<f32> {
        let converted = self.resampler.push(window);
        self.store(&converted);
        converted
    }

    fn store(&mut self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        if let Err(error) = writer.write(samples) {
            warn!(
                "Stopped writing the {} working track after {:.1}s: {error}",
                self.label,
                self.samples_written as f64 / WORKING_SAMPLE_RATE as f64
            );
            if let Some(writer) = self.writer.take() {
                writer.abandon();
            }
            return;
        }
        self.samples_written += samples.len() as u64;
    }

    /// Flush the tail and publish the track. Whatever is still held in the
    /// resampler belongs to the recording, so it is converted first.
    pub fn finish(&mut self) {
        let tail = self.resampler.drain();
        self.store(&tail);

        if let Some(writer) = self.writer.take() {
            let seconds = self.samples_written as f64 / WORKING_SAMPLE_RATE as f64;
            let path = writer.final_path().to_path_buf();
            match writer.finish() {
                // The size is logged because it is the one cost of keeping the
                // track, and an hour of real speech is the only honest measure
                // of it.
                Ok(path) => info!(
                    "✅ Working track for {}: {:.1}s, {:.1} MB → {}",
                    self.label,
                    seconds,
                    std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0,
                    path.display()
                ),
                // Unlike a recording, a partial working track is worse than
                // none: it would be read as the whole session and quietly
                // transcribe only part of it.
                Err(error) => {
                    warn!("Could not finish the {} working track: {error}", self.label);
                    streaming_encoder::discard_partial(&path);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::ffmpeg::find_ffmpeg_path;

    /// A 440 Hz tone — well inside the band both rates keep.
    fn tone(sample_rate: u32, seconds: f32) -> Vec<f32> {
        let total = (sample_rate as f32 * seconds) as usize;
        (0..total)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
            })
            .collect()
    }

    #[test]
    fn conversion_does_not_drift_over_a_long_recording() {
        let window = 2400; // 50 ms at 48 kHz: the mixing window of the pipeline
        let mut resampler = WorkingRateResampler::new(48000, window).unwrap();
        let audio = tone(48000, 60.0);

        let mut produced = 0usize;
        for chunk in audio.chunks(window) {
            produced += resampler.push(chunk).len();
        }
        produced += resampler.drain().len();

        // A minute of audio has to come to a minute of audio: the startup delay
        // of the filter is the only shortfall allowed, and it is milliseconds.
        let expected = 60 * WORKING_SAMPLE_RATE as usize;
        let shortfall = expected.saturating_sub(produced);
        assert!(
            shortfall < WORKING_SAMPLE_RATE as usize / 100,
            "expected about {expected} samples, got {produced}"
        );
        assert!(produced <= expected + 16, "produced more audio than existed");
    }

    #[test]
    fn conversion_preserves_the_signal() {
        let window = 2400;
        let mut resampler = WorkingRateResampler::new(48000, window).unwrap();
        let audio = tone(48000, 2.0);

        let mut converted = Vec::new();
        for chunk in audio.chunks(window) {
            converted.extend(resampler.push(chunk));
        }
        converted.extend(resampler.drain());

        // Skip the startup of the filter, then compare energy with the source.
        let rms = |samples: &[f32]| {
            (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
        };
        let source_rms = rms(&audio[4800..]);
        let converted_rms = rms(&converted[1600..]);
        assert!(
            (converted_rms - source_rms).abs() < 0.02,
            "energy changed: {source_rms} to {converted_rms}"
        );
    }

    #[test]
    fn already_at_the_working_rate_is_passed_through() {
        let mut resampler = WorkingRateResampler::new(WORKING_SAMPLE_RATE, 800).unwrap();
        let audio = tone(WORKING_SAMPLE_RATE, 0.1);
        let converted = resampler.push(&audio);
        assert_eq!(converted, audio);
        assert!(resampler.drain().is_empty());
    }

    #[test]
    fn a_track_is_only_named_once_it_is_complete() {
        let dir = tempfile::tempdir().unwrap();
        assert!(find_working_track(dir.path(), "mic").is_none());

        let path = working_track_path(dir.path(), "mic");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path.with_extension(PARTIAL_EXT), b"half a recording").unwrap();
        assert!(
            find_working_track(dir.path(), "mic").is_none(),
            "an unfinished track must not be readable"
        );

        std::fs::write(&path, b"a whole recording").unwrap();
        assert_eq!(find_working_track(dir.path(), "mic"), Some(path));
    }

    #[test]
    fn an_interrupted_recording_leaves_nothing_behind() {
        let dir = tempfile::tempdir().unwrap();
        let finished = working_track_path(dir.path(), "mic");
        std::fs::create_dir_all(finished.parent().unwrap()).unwrap();
        let unfinished = working_track_path(dir.path(), "system").with_extension(PARTIAL_EXT);
        std::fs::write(&finished, b"a whole recording").unwrap();
        std::fs::write(&unfinished, b"half a recording").unwrap();

        discard_partial_tracks(dir.path());

        assert!(!unfinished.exists(), "the half-written track was kept");
        assert!(finished.exists(), "a finished track must survive");
        // A folder that never had one must not be a problem either.
        discard_partial_tracks(tempfile::tempdir().unwrap().path());
    }

    /// Needs FFmpeg, like every other encoding path in the app; skipped when it
    /// is not installed so the suite still runs on a bare machine.
    #[test]
    fn a_finished_track_is_a_readable_16khz_file() {
        if find_ffmpeg_path().is_none() {
            eprintln!("skipping: FFmpeg is not installed");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = working_track_path(dir.path(), "mic");
        let mut track = WorkingTrack::new("microphone", 48000, 2400, Some(path.clone())).unwrap();

        for chunk in tone(48000, 1.0).chunks(2400) {
            track.push(chunk);
        }
        track.finish();

        let decoded = super::super::decoder::decode_audio_file(&path).unwrap();
        assert_eq!(decoded.sample_rate, WORKING_SAMPLE_RATE);
        assert_eq!(decoded.channels, 1);
        assert!(
            (decoded.duration_seconds - 1.0).abs() < 0.05,
            "expected about a second, got {:.3}s",
            decoded.duration_seconds
        );
    }

    /// The point of the track is that a later pass can read it instead of
    /// converting the delivery track for itself. That only holds if the two
    /// conversions agree, so this compares the streaming one against the
    /// one-shot resampler every stored file goes through.
    #[test]
    fn matches_the_conversion_done_on_stored_files() {
        let window = 2400;
        let audio = tone(48000, 2.0);
        let one_shot = super::super::audio_processing::resample_audio(&audio, 48000, 16000);

        let mut resampler = WorkingRateResampler::new(48000, window).unwrap();
        let mut streamed = Vec::new();
        for chunk in audio.chunks(window) {
            streamed.extend(resampler.push(chunk));
        }
        streamed.extend(resampler.drain());

        // Both carry the delay of the same filter, so they line up sample for
        // sample; compare the part that is past the startup of both.
        let compared = one_shot.len().min(streamed.len());
        assert!(compared > 16000, "too little output to compare");
        let worst = one_shot[1600..compared]
            .iter()
            .zip(&streamed[1600..compared])
            .fold(0.0f32, |worst, (a, b)| worst.max((a - b).abs()));
        assert!(
            worst < 0.01,
            "streaming conversion differs from the one-shot by {worst}"
        );
    }

    /// What keeping the working track costs the live pipeline. Opt-in because
    /// the number only means something in a release build:
    ///   cargo test --release --lib working_track -- --ignored --nocapture
    #[test]
    #[ignore = "timing measurement, run in release"]
    fn cost_relative_to_realtime() {
        let window = 2400; // 50 ms at 48 kHz
        let windows = 1200; // one minute of audio, per source
        let audio = tone(48000, 0.05);
        let mut resampler = WorkingRateResampler::new(48000, window).unwrap();

        let started = std::time::Instant::now();
        let mut produced = 0usize;
        for _ in 0..windows {
            produced += resampler.push(&audio).len();
        }
        let elapsed = started.elapsed().as_secs_f64();
        let audio_seconds = windows as f64 * 0.05;

        println!(
            "working-track conversion: {:.3} s of work for {:.0} s of audio \
             ({:.2}% of realtime, {:.3} ms per 50 ms window, {produced} samples out)",
            elapsed,
            audio_seconds,
            elapsed / audio_seconds * 100.0,
            elapsed / windows as f64 * 1000.0
        );
    }

    #[test]
    fn a_track_with_nowhere_to_go_still_converts_for_vad() {
        let mut track = WorkingTrack::new("microphone", 48000, 2400, None).unwrap();
        let converted = track.push(&tone(48000, 0.05));
        assert!(!converted.is_empty());
        track.finish();
    }
}
