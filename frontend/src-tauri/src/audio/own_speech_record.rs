//! What the detector heard, written down beside the audio it judged.
//!
//! ## Why the live answer has to be kept
//!
//! The hybrid in [`super::own_speech`] drops a microphone segment when the
//! speakers were playing under it and the owner was not talking. Only the
//! second microphone stream can tell those apart, and that stream is never
//! recorded — by design, since it carries his voice and nothing else worth
//! storing. What lands on disk is the ordinary microphone, echo and all.
//!
//! So a later pass over the stored audio — re-transcription, above all — used
//! to reach a different verdict than the live one on the same recording: it
//! has the file, but not the two timelines the decision was made from, and the
//! echo the canceller left came back as words in the owner's channel. One
//! session, read twice, said two different things, and the second reading was
//! the worse one.
//!
//! This module closes that gap by storing the timelines themselves, one entry
//! per mixing window, so the later pass can ask what the detector answered
//! instead of guessing. What it does with the answer is not quite what the
//! live pass does: it silences those stretches of the track before voice
//! detection runs, rather than dropping whole segments afterwards. The reason
//! is in [`super::own_speech::echo_spans`] — offline segments are far coarser
//! than live ones, and one of them routinely holds the echo *and* the answer
//! he gave over it.
//!
//! What this cannot remove: the echo arrives about 120 ms after the sound that
//! caused it, so the tail that follows each pause in the far end outlives the
//! window the record marked. Measured on the bench, that leaves under a fifth
//! of the far end in the track, and voice detection finds no speech in it.
//!
//! ## The clock, which is the whole difficulty
//!
//! The record is laid out in **working-track time, not recording time**: slot
//! `i` covers `[i * window_ms, (i + 1) * window_ms)` of the stored track.
//!
//! That is deliberate, and it is what keeps the two clocks from drifting
//! apart. A window is observed in the same step of the pipeline that hands it
//! to the tracks, so there is exactly one entry for every window that reached
//! the file. The in-memory timelines of the live pass are laid out in
//! *recording* time instead, where a break in capture leaves an unanswered
//! gap; that gap has no samples behind it and so no place in the file, and
//! copying it here would push everything after it out of step.
//!
//! What remains is a sub-window offset: the resampler holds part of a window
//! back before the working track sees it. It is bounded by one window (50 ms)
//! against a floor of 300 ms, and the rule it feeds is lenient by
//! construction — but it is a real offset, which is why the check that settles
//! this is the end-to-end one in the pipeline bench, where a written track is
//! decoded and silenced and measured, and not the round-trip tests below.
//!
//! ## What it is not
//!
//! Not a transcript and not audio, but still a record of who was speaking
//! when, which is the same thing at a coarser grain. It goes through the same
//! sink as the recordings, so it is encrypted with the archive key exactly as
//! they are, and a locked archive yields no record rather than a plaintext
//! one.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use log::{debug, info, warn};

use super::encrypted_audio::{AudioSink, AudioSource};
use super::own_speech::WindowTimeline;
use super::working_track::working_dir;

/// Name of the record inside the working directory.
pub const RECORD_NAME: &str = "own-speech.tkgate";

/// Name of one still being written. Never read.
const PARTIAL_NAME: &str = "own-speech.tkgate.part";

/// First bytes of the file, so nothing else is ever mistaken for it. The last
/// byte is the layout version: raise it when the layout changes, and an older
/// build declines the file instead of misreading it.
const MAGIC: &[u8; 8] = b"TKGATE\n\x01";

/// The magic, the window length, the number of windows.
const HEADER_LEN: usize = 8 + 4 + 4;

/// An hour of 50 ms windows is 72 000 entries. This only stops a damaged file
/// from asking for an unbounded allocation: 16 M windows is over nine days.
const MAX_WINDOWS: u32 = 16 << 20;

/// One window from one detector.
const UNKNOWN: u8 = 0;
const QUIET: u8 = 1;
const ACTIVE: u8 = 2;

/// Where the record for a recording lives.
pub fn record_path(meeting_folder: &Path) -> PathBuf {
    working_dir(meeting_folder).join(RECORD_NAME)
}

fn flag_to_code(flag: Option<bool>) -> u8 {
    match flag {
        None => UNKNOWN,
        Some(false) => QUIET,
        Some(true) => ACTIVE,
    }
}

fn code_to_flag(code: u8) -> Option<bool> {
    match code {
        QUIET => Some(false),
        ACTIVE => Some(true),
        // Anything else is a value this version does not know, and "do not
        // know" is the reading that can only keep a segment.
        _ => None,
    }
}

/// The two timelines in the form they take on disk.
///
/// One byte per window: the low nibble is the owner, the high nibble the
/// speakers. An hour costs 72 KB, nothing against the audio it describes, and
/// the fixed width keeps a slot index an offset.
#[derive(Debug, Clone, PartialEq)]
pub struct GateRecord {
    window_ms: f32,
    slots: Vec<u8>,
}

impl GateRecord {
    pub fn new(window_ms: f32) -> Self {
        Self {
            window_ms,
            slots: Vec::new(),
        }
    }

    /// Write down what this window held, for both detectors.
    pub fn observe(&mut self, own_speech: Option<bool>, far_end: Option<bool>) {
        self.slots
            .push(flag_to_code(own_speech) | (flag_to_code(far_end) << 4));
    }

    pub fn windows(&self) -> usize {
        self.slots.len()
    }

    /// Whether the detector answered any window at all.
    ///
    /// A recording made with it switched off has nothing to say here, and a
    /// record of nothing but "do not know" would only cost a file.
    pub fn detector_answered(&self) -> bool {
        self.slots.iter().any(|slot| slot & 0x0f != UNKNOWN)
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER_LEN + self.slots.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.window_ms.to_le_bytes());
        bytes.extend_from_slice(&(self.slots.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&self.slots);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(anyhow!("too short to be a detector record"));
        }
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err(anyhow!(
                "not a detector record, or a layout this build does not read"
            ));
        }
        let window_ms = f32::from_le_bytes(bytes[8..12].try_into()?);
        if !window_ms.is_finite() || window_ms <= 0.0 {
            return Err(anyhow!("the record claims a window of {window_ms} ms"));
        }
        let count = u32::from_le_bytes(bytes[12..16].try_into()?);
        if count > MAX_WINDOWS {
            return Err(anyhow!("the record claims {count} windows"));
        }
        let slots = &bytes[HEADER_LEN..];
        if slots.len() != count as usize {
            // Truncation means a crash mid-write, which publishing by rename
            // is supposed to prevent. Refuse it rather than judge a whole
            // session by a record of the first part of it.
            return Err(anyhow!(
                "the record holds {} windows but claims {count}",
                slots.len()
            ));
        }
        Ok(Self {
            window_ms,
            slots: slots.to_vec(),
        })
    }

    /// The two timelines this record stands for: the owner, then the speakers.
    pub fn timelines(&self) -> (WindowTimeline, WindowTimeline) {
        let own = self
            .slots
            .iter()
            .map(|slot| code_to_flag(slot & 0x0f))
            .collect();
        let far = self
            .slots
            .iter()
            .map(|slot| code_to_flag(slot >> 4))
            .collect();
        (
            WindowTimeline::from_slots(self.window_ms as f64, own),
            WindowTimeline::from_slots(self.window_ms as f64, far),
        )
    }
}

/// The record being built while a recording runs.
///
/// Published by rename like the working tracks: a recording that never closed
/// leaves no record at all, which reads as "no second opinion" and keeps every
/// segment — rather than half a record, which would silently stop gating
/// partway through the session.
pub struct GateRecorder {
    record: GateRecord,
    /// `None` until the recording has a folder, and it may never get one.
    path: Option<PathBuf>,
}

impl GateRecorder {
    pub fn new(window_ms: f32) -> Self {
        Self {
            record: GateRecord::new(window_ms),
            path: None,
        }
    }

    /// Point the record at a meeting folder. Without this it is kept in memory
    /// and thrown away, exactly like the working tracks.
    pub fn open_in(&mut self, meeting_folder: &Path) {
        self.path = Some(record_path(meeting_folder));
    }

    pub fn observe(&mut self, own_speech: Option<bool>, far_end: Option<bool>) {
        self.record.observe(own_speech, far_end);
    }

    /// Write the record out, if there is one worth writing.
    pub fn finish(&mut self) {
        let Some(path) = self.path.take() else {
            return;
        };
        if !self.record.detector_answered() {
            debug!("No own-speech record: the detector answered no window of this recording");
            return;
        }
        if let Err(error) = self.write(&path) {
            // The recording is unaffected. A later pass simply has no second
            // opinion and keeps everything, which is what it did before.
            warn!("Could not write the own-speech record: {error}");
        }
    }

    fn write(&self, path: &Path) -> Result<()> {
        let partial = path.with_file_name(PARTIAL_NAME);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&partial);

        let mut sink = AudioSink::create(&partial)?;
        let encrypted = sink.is_encrypted();
        sink.write_all(&self.record.encode())?;
        sink.finish()?;
        drop(sink);

        std::fs::rename(&partial, path)?;
        info!(
            "🔇 Own-speech record: {} windows of {:.0} ms{} → {}",
            self.record.windows(),
            self.record.window_ms,
            if encrypted { ", encrypted" } else { "" },
            path.display()
        );
        Ok(())
    }
}

/// The timelines this recording wrote down, if it wrote any.
///
/// `None` covers every ordinary reason there is nothing to read — made before
/// this existed, made with the detector off, interrupted, or encrypted while
/// the archive is locked — and all of them mean the same thing to the caller:
/// no second opinion, so keep what the microphone heard.
pub fn read_timelines(meeting_folder: &Path) -> Option<(WindowTimeline, WindowTimeline)> {
    let path = record_path(meeting_folder);
    if !path.is_file() {
        return None;
    }
    let record = match read_record(&path) {
        Ok(record) => record,
        Err(error) => {
            warn!(
                "Ignoring the own-speech record at {}: {error}",
                path.display()
            );
            return None;
        }
    };
    info!(
        "🔇 Read the own-speech record: {} windows of {:.0} ms",
        record.windows(),
        record.window_ms
    );
    Some(record.timelines())
}

fn read_record(path: &Path) -> Result<GateRecord> {
    let mut source = AudioSource::open(path)?;
    let mut bytes = Vec::new();
    source.read_to_end(&mut bytes)?;
    GateRecord::decode(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::own_speech::{is_only_the_speakers, measure_segment};

    const WINDOW_MS: f32 = 50.0;

    fn recorded(windows: &[(Option<bool>, Option<bool>)]) -> GateRecord {
        let mut record = GateRecord::new(WINDOW_MS);
        for (own, far) in windows {
            record.observe(*own, *far);
        }
        record
    }

    #[test]
    fn a_record_reads_back_as_the_windows_it_was_given() {
        let windows = [
            (Some(true), Some(false)),
            (Some(false), Some(true)),
            (None, Some(true)),
            (None, None),
        ];
        let record = recorded(&windows);
        let read = GateRecord::decode(&record.encode()).expect("decodes");
        assert_eq!(read, record);

        let (own, far) = read.timelines();
        assert_eq!(own.active_ms_between(0.0, 50.0, false), 50.0);
        assert_eq!(far.active_ms_between(0.0, 50.0, false), 0.0);
        assert_eq!(far.active_ms_between(50.0, 150.0, false), 100.0);
        // An unanswered window is still unanswered, and still falls whichever
        // way the caller says.
        assert_eq!(own.active_ms_between(100.0, 150.0, true), 50.0);
        assert_eq!(own.active_ms_between(100.0, 150.0, false), 0.0);
    }

    /// The point of keeping the record at all: the rule reaches the same
    /// verdict from the file as it did from the live timelines.
    #[test]
    fn the_verdict_survives_the_round_trip() {
        // A second of the speakers alone, then a second of him over them.
        let mut windows = vec![(Some(false), Some(true)); 20];
        windows.extend(vec![(Some(true), Some(true)); 20]);
        let record = recorded(&windows);

        let (live_own, live_far) = record.timelines();
        let read = GateRecord::decode(&record.encode()).expect("decodes");
        let (own, far) = read.timelines();

        for (from, to, expected) in [(0.0, 1000.0, true), (1000.0, 2000.0, false)] {
            let live = {
                let (own_ms, far_ms) = measure_segment(&live_own, &live_far, from, to);
                is_only_the_speakers(own_ms, far_ms)
            };
            let stored = {
                let (own_ms, far_ms) = measure_segment(&own, &far, from, to);
                is_only_the_speakers(own_ms, far_ms)
            };
            assert_eq!(live, expected);
            assert_eq!(
                stored, live,
                "the stored record disagreed about {from}-{to} ms"
            );
        }
    }

    #[test]
    fn a_truncated_record_is_refused_rather_than_half_believed() {
        let record = recorded(&[(Some(true), Some(true)); 8]);
        let mut bytes = record.encode();
        bytes.truncate(bytes.len() - 3);
        assert!(GateRecord::decode(&bytes).is_err());
    }

    #[test]
    fn something_that_is_not_a_record_is_refused() {
        assert!(GateRecord::decode(b"RIFF....WAVEfmt ").is_err());
        assert!(GateRecord::decode(&[]).is_err());
    }

    #[test]
    fn a_recording_with_the_detector_off_leaves_no_file() {
        let folder = tempfile::tempdir().expect("temp dir");
        let mut recorder = GateRecorder::new(WINDOW_MS);
        recorder.open_in(folder.path());
        for _ in 0..10 {
            recorder.observe(None, Some(false));
        }
        recorder.finish();
        assert!(!record_path(folder.path()).exists());
        assert!(read_timelines(folder.path()).is_none());
    }

    #[test]
    fn a_written_record_is_read_back_by_the_next_pass() {
        let folder = tempfile::tempdir().expect("temp dir");
        let mut recorder = GateRecorder::new(WINDOW_MS);
        recorder.open_in(folder.path());
        for _ in 0..20 {
            recorder.observe(Some(false), Some(true));
        }
        for _ in 0..20 {
            recorder.observe(Some(true), Some(true));
        }
        recorder.finish();

        let (own, far) = read_timelines(folder.path()).expect("the record is there");
        let (own_ms, far_ms) = measure_segment(&own, &far, 0.0, 1000.0);
        assert!(is_only_the_speakers(own_ms, far_ms));
        let (own_ms, far_ms) = measure_segment(&own, &far, 1000.0, 2000.0);
        assert!(!is_only_the_speakers(own_ms, far_ms));

        // Nothing is left behind under the partial name.
        assert!(!working_dir(folder.path()).join(PARTIAL_NAME).exists());
    }

    #[test]
    fn no_record_means_no_second_opinion() {
        let folder = tempfile::tempdir().expect("temp dir");
        assert!(read_timelines(folder.path()).is_none());
    }
}
