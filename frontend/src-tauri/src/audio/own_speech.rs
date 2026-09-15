//! Telling the owner's own speech apart from the speakers' echo, using a
//! second microphone stream that is never recorded.
//!
//! ## The problem this solves
//!
//! Letting Windows cancel the echo works — the far end disappears completely,
//! which nothing we can do from inside an application matches. It has one
//! cost, and it is the one case the owner cares about most: when he speaks
//! *while* the other person is speaking, the driver's canceller takes his
//! voice down along with the echo. Talking over someone is not an edge case in
//! a conversation, and there is no setting to ask the driver not to.
//!
//! ## The way around it
//!
//! The microphone can be opened twice at once — measured on the owner's
//! machine: with the speakers playing, the ordinary stream carried the echo at
//! energy 36.6 while the communications stream carried 0.0. So the two streams
//! answer two different questions, and each answers its own well:
//!
//! * the ordinary stream is what gets recorded and transcribed, because the
//!   owner's voice is never suppressed in it;
//! * the communications stream is read only for *whether he was talking* — by
//!   construction it holds no trace of the far end, so when it is loud, the
//!   sound is his.
//!
//! A microphone segment is then dropped as leftover echo only when both things
//! are true: the speakers really were playing under it, and this stream stayed
//! quiet throughout. When the speakers were silent there is no echo to mistake
//! anything for, and the segment is kept whatever this stream says.
//!
//! ## What is deliberately not done here
//!
//! No sample from the communications stream reaches the recording, the
//! transcript or the disk. It is a detector. Mixing the two streams would mean
//! choosing between them sample by sample, which is a cross-fade problem with
//! no good answer and a new way to lose a word.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Loudness a window has to reach before it counts as the owner speaking.
///
/// The communications stream makes this easy: the same mode that removes the
/// echo also removes room noise, so quiet really is zero rather than a hiss to
/// be thresholded against. -40 dBFS sits well above what is left and well
/// below speech.
const SPEAKING_RMS: f32 = 0.01;

/// How long the owner has to be audible inside a microphone segment for it to
/// count as his.
///
/// Short because it is the *minimum* utterance worth keeping: an "ага" in the
/// middle of the other person's sentence is exactly the thing the whole
/// arrangement exists to save.
const OWN_SPEECH_MIN_MS: f64 = 250.0;

/// How long the speakers have to have been playing under a segment before the
/// segment is a candidate for being their echo at all.
const FAR_END_MIN_MS: f64 = 300.0;

/// How far the detector is allowed to fall behind the recording before the
/// oldest samples are let go.
///
/// Both streams come off the same device and the same crystal, so they should
/// not drift apart; this bounds the damage if one of them stutters. Letting
/// the oldest go keeps the detector answering about *now*, which is the only
/// thing it is asked about.
const MAX_PENDING_SECONDS: f64 = 2.0;

/// A flag per mixing window, laid out along the recording's own clock.
///
/// Windows arrive in order and each covers a fixed span, so the slot index is
/// the timestamp. `None` means nobody looked — the detector had not caught up,
/// or the clock jumped over a break — and callers are expected to read it as
/// "do not know", never as "no".
#[derive(Debug, Clone)]
pub struct WindowTimeline {
    slot_ms: f64,
    slots: Vec<Option<bool>>,
}

impl WindowTimeline {
    pub fn new(slot_ms: f64) -> Self {
        Self {
            slot_ms: slot_ms.max(1.0),
            slots: Vec::new(),
        }
    }

    /// Rebuild a timeline that was written down during the recording.
    ///
    /// The slots are the same windows in the same order; what was `None` then
    /// is `None` now, so an unobserved window keeps reading as "do not know"
    /// rather than as silence.
    pub fn from_slots(slot_ms: f64, slots: Vec<Option<bool>>) -> Self {
        Self {
            slot_ms: slot_ms.max(1.0),
            slots,
        }
    }

    /// Record what the next window held.
    pub fn push(&mut self, value: Option<bool>) {
        self.slots.push(value);
    }

    /// Jump the clock forward over a break in the recording, leaving the
    /// skipped span unanswered rather than pretending it was silent.
    pub fn skip_to(&mut self, timestamp_ms: f64) {
        let wanted = (timestamp_ms / self.slot_ms).floor().max(0.0) as usize;
        while self.slots.len() < wanted {
            self.slots.push(None);
        }
    }

    /// Milliseconds inside `[from_ms, to_ms)` that were flagged.
    ///
    /// `unknown_counts` decides which way an unanswered window falls, so each
    /// caller can choose the answer that is safe for it.
    pub fn active_ms_between(&self, from_ms: f64, to_ms: f64, unknown_counts: bool) -> f64 {
        if to_ms <= from_ms {
            return 0.0;
        }
        let first = (from_ms / self.slot_ms).floor().max(0.0) as usize;
        let last = (to_ms / self.slot_ms).ceil().max(0.0) as usize;
        let mut total = 0.0;
        for index in first..last.min(self.slots.len()) {
            let counts = match self.slots[index] {
                Some(flag) => flag,
                None => unknown_counts,
            };
            if counts {
                // Only the part of the slot that falls inside the span counts.
                let slot_from = index as f64 * self.slot_ms;
                let slot_to = slot_from + self.slot_ms;
                total += slot_to.min(to_ms) - slot_from.max(from_ms);
            }
        }
        total.max(0.0)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots.len()
    }
}

/// How long the owner was audible under a microphone segment, and how long the
/// speakers were playing under it.
///
/// The two unanswered-window rules are opposites on purpose, and both lean the
/// same way in the end: an unobserved window counts as the owner speaking and
/// as the speakers being silent, so a gap in either timeline can only keep a
/// segment, never lose one. Losing a word he actually said is the one failure
/// this whole arrangement is not allowed to have.
///
/// Live and offline ask this the same way. The pipeline asks it of the
/// timelines it is filling; a later pass over the stored audio asks it of the
/// same timelines read back from the recording folder.
pub fn measure_segment(
    own_speech: &WindowTimeline,
    far_end: &WindowTimeline,
    from_ms: f64,
    to_ms: f64,
) -> (f64, f64) {
    (
        own_speech.active_ms_between(from_ms, to_ms, true),
        far_end.active_ms_between(from_ms, to_ms, false),
    )
}

/// The stretches of a recording that were nothing but the speakers.
///
/// The live pass asks the question of one VAD segment at a time, and that
/// works there because its segments are short: a pause of 800 ms ends one. A
/// later pass over the stored track segments it far more coarsely — two
/// seconds of silence is treated as one breath inside a sentence rather than
/// the end of a turn — so a single segment routinely holds the echo *and* the
/// answer he gave over it, and no verdict on the whole of it can be right.
///
/// So offline the question is asked of the windows themselves: the speakers
/// playing, and the owner not. A run of such windows has to last
/// [`FAR_END_MIN_MS`] before it counts, which is what keeps the gaps between
/// his own words from being cut, and by construction it holds none of his
/// speech at all — an unanswered window is never a candidate, so a detector
/// that fell behind silences nothing.
pub fn echo_spans(own_speech: &WindowTimeline, far_end: &WindowTimeline) -> Vec<(f64, f64)> {
    debug_assert_eq!(own_speech.slot_ms, far_end.slot_ms);
    let slot_ms = far_end.slot_ms;
    let count = far_end.slots.len().max(own_speech.slots.len());
    let is_candidate = |index: usize| {
        far_end.slots.get(index).copied().flatten() == Some(true)
            && own_speech.slots.get(index).copied().flatten() == Some(false)
    };

    let mut spans = Vec::new();
    let mut run_start: Option<usize> = None;
    // One past the end, to close a run that reaches it.
    for index in 0..=count {
        if index < count && is_candidate(index) {
            run_start.get_or_insert(index);
            continue;
        }
        if let Some(start) = run_start.take() {
            let from = start as f64 * slot_ms;
            let to = index as f64 * slot_ms;
            if to - from >= FAR_END_MIN_MS {
                spans.push((from, to));
            }
        }
    }
    spans
}

/// Whether a microphone segment is nothing but the speakers coming back.
///
/// Both halves matter. Without the first, a segment recorded in a silent room
/// could be thrown away because the detector missed it; without the second,
/// every word said over the other person would be thrown away with the echo.
pub fn is_only_the_speakers(own_speech_ms: f64, far_end_ms: f64) -> bool {
    far_end_ms >= FAR_END_MIN_MS && own_speech_ms < OWN_SPEECH_MIN_MS
}

#[derive(Debug, Default)]
struct GateInner {
    /// Samples the detector stream has delivered and no window has claimed.
    pending: VecDeque<f32>,
    /// The rate Windows chose for the detector stream. Zero until it opens.
    sample_rate: u32,
    /// Samples let go because the detector ran ahead of the recording.
    dropped: u64,
}

/// The second microphone stream, read only for who is talking.
///
/// Cloning gives another handle to the same detector: the capture thread holds
/// one to push samples into, the pipeline holds one to ask questions of.
#[derive(Clone, Debug, Default)]
pub struct OwnSpeechGate {
    inner: Arc<Mutex<GateInner>>,
}

impl OwnSpeechGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// The one detector this process records through.
    ///
    /// A handle rather than a global read: the capture thread pushes into it
    /// and the pipeline asks it questions, and only one recording runs at a
    /// time. Tests build their own with `new` and touch nothing shared.
    pub fn shared() -> Self {
        static SHARED: std::sync::OnceLock<OwnSpeechGate> = std::sync::OnceLock::new();
        SHARED.get_or_init(OwnSpeechGate::new).clone()
    }

    /// Say the detector stream has opened, and at what rate.
    ///
    /// Windows picks the rate for this mode itself, and it is not promised to
    /// match the one the recording runs at — which is why windows are claimed
    /// by duration below rather than by sample count.
    pub fn opened(&self, sample_rate: u32) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.pending.clear();
            inner.sample_rate = sample_rate;
            inner.dropped = 0;
        }
    }

    /// Say the detector stream has stopped. Questions from here on go
    /// unanswered rather than answered from stale samples.
    pub fn closed(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.pending.clear();
            inner.sample_rate = 0;
        }
    }

    /// Whether the detector stream is running.
    pub fn is_open(&self) -> bool {
        self.inner
            .lock()
            .map(|inner| inner.sample_rate > 0)
            .unwrap_or(false)
    }

    /// Hand over what the detector stream just delivered.
    pub fn push(&self, samples: &[f32]) {
        let Ok(mut inner) = self.inner.lock() else {
            return;
        };
        if inner.sample_rate == 0 {
            return;
        }
        inner.pending.extend(samples.iter().copied());

        let cap = (inner.sample_rate as f64 * MAX_PENDING_SECONDS) as usize;
        if inner.pending.len() > cap {
            let excess = inner.pending.len() - cap;
            inner.pending.drain(..excess);
            inner.dropped += excess as u64;
        }
    }

    /// Claim the detector's answer for the next `duration_ms` of recording.
    ///
    /// `None` when the stream is not open or has not delivered that much yet:
    /// the detector is behind, and a segment is never dropped on a window it
    /// did not see.
    pub fn take_window(&self, duration_ms: f64) -> Option<bool> {
        let mut inner = self.inner.lock().ok()?;
        if inner.sample_rate == 0 {
            return None;
        }
        let wanted = (duration_ms * inner.sample_rate as f64 / 1000.0).round() as usize;
        if wanted == 0 || inner.pending.len() < wanted {
            return None;
        }

        let mut sum_squares = 0.0f64;
        for _ in 0..wanted {
            let sample = inner.pending.pop_front().unwrap_or(0.0);
            sum_squares += (sample as f64) * (sample as f64);
        }
        let rms = (sum_squares / wanted as f64).sqrt() as f32;
        Some(rms >= SPEAKING_RMS)
    }

    /// How many samples were let go because the detector ran ahead. Zero in a
    /// healthy recording; worth saying out loud when it is not.
    pub fn dropped_samples(&self) -> u64 {
        self.inner.lock().map(|inner| inner.dropped).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;
    const WINDOW_MS: f64 = 50.0;

    fn loud(samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|index| {
                let t = index as f32 / RATE as f32;
                (t * 300.0 * std::f32::consts::TAU).sin() * 0.4
            })
            .collect()
    }

    fn quiet(samples: usize) -> Vec<f32> {
        vec![0.0; samples]
    }

    fn window_samples() -> usize {
        (WINDOW_MS * RATE as f64 / 1000.0) as usize
    }

    #[test]
    fn a_loud_window_is_the_owner_and_a_silent_one_is_not() {
        let gate = OwnSpeechGate::new();
        gate.opened(RATE);

        gate.push(&loud(window_samples()));
        assert_eq!(gate.take_window(WINDOW_MS), Some(true));

        gate.push(&quiet(window_samples()));
        assert_eq!(gate.take_window(WINDOW_MS), Some(false));
    }

    /// The detector running behind must never read as "he was not speaking".
    #[test]
    fn a_detector_that_has_not_caught_up_says_nothing() {
        let gate = OwnSpeechGate::new();
        gate.opened(RATE);

        gate.push(&loud(window_samples() / 2));
        assert_eq!(gate.take_window(WINDOW_MS), None);

        gate.push(&loud(window_samples() / 2));
        assert_eq!(gate.take_window(WINDOW_MS), Some(true));
    }

    #[test]
    fn a_gate_that_never_opened_says_nothing() {
        let gate = OwnSpeechGate::new();
        gate.push(&loud(window_samples()));
        assert_eq!(gate.take_window(WINDOW_MS), None);
        assert!(!gate.is_open());
    }

    /// A detector that runs ahead is trimmed rather than left to grow for the
    /// length of a session.
    #[test]
    fn a_detector_that_runs_ahead_lets_the_oldest_go() {
        let gate = OwnSpeechGate::new();
        gate.opened(RATE);

        // Five seconds delivered, two seconds kept.
        for _ in 0..100 {
            gate.push(&loud(window_samples()));
        }

        assert_eq!(gate.dropped_samples(), (RATE as u64 * 3));
    }

    #[test]
    fn windows_claimed_at_the_rate_windows_chose() {
        let gate = OwnSpeechGate::new();
        gate.opened(16_000);

        // 50ms at 16 kHz is 800 samples, not the 2400 the recording uses.
        gate.push(&vec![0.4; 800]);
        assert_eq!(gate.take_window(WINDOW_MS), Some(true));
        assert_eq!(gate.take_window(WINDOW_MS), None);
    }

    #[test]
    fn a_timeline_measures_the_part_of_a_span_that_was_flagged() {
        let mut timeline = WindowTimeline::new(WINDOW_MS);
        for _ in 0..10 {
            timeline.push(Some(true));
        }
        for _ in 0..10 {
            timeline.push(Some(false));
        }

        assert_eq!(timeline.active_ms_between(0.0, 500.0, false), 500.0);
        assert_eq!(timeline.active_ms_between(500.0, 1000.0, false), 0.0);
        assert_eq!(timeline.active_ms_between(250.0, 750.0, false), 250.0);
    }

    /// Past the end of what was observed the answer is unknown, and unknown
    /// falls whichever way the caller asked for.
    #[test]
    fn a_span_past_the_end_of_the_timeline_is_unknown() {
        let mut timeline = WindowTimeline::new(WINDOW_MS);
        timeline.push(Some(true));

        assert_eq!(timeline.active_ms_between(0.0, 1000.0, false), 50.0);
        assert_eq!(timeline.active_ms_between(1000.0, 2000.0, true), 0.0);
    }

    #[test]
    fn a_break_in_the_recording_leaves_the_skipped_span_unanswered() {
        let mut timeline = WindowTimeline::new(WINDOW_MS);
        timeline.push(Some(true));
        timeline.skip_to(1_000.0);
        timeline.push(Some(true));

        assert_eq!(timeline.len(), 21);
        // The skipped second answers "no" to one caller and "yes" to the other.
        assert_eq!(timeline.active_ms_between(50.0, 1_000.0, false), 0.0);
        assert_eq!(timeline.active_ms_between(50.0, 1_000.0, true), 950.0);
    }

    #[test]
    fn a_segment_over_playing_speakers_with_no_own_speech_is_echo() {
        assert!(is_only_the_speakers(0.0, 4_000.0));
        assert!(is_only_the_speakers(100.0, 4_000.0));
    }

    /// The case the whole arrangement exists for: a short word said over the
    /// other person is his own, not an echo.
    #[test]
    fn a_word_said_over_the_speakers_is_kept() {
        assert!(!is_only_the_speakers(300.0, 4_000.0));
    }

    /// And with nothing playing there is no echo to mistake it for, however
    /// quiet the detector was.
    #[test]
    fn a_segment_recorded_in_a_silent_room_is_kept() {
        assert!(!is_only_the_speakers(0.0, 0.0));
        assert!(!is_only_the_speakers(0.0, 200.0));
    }

    /// Windows as a later pass reads them back: `own` is what the detector
    /// answered, `far` whether the speakers were playing.
    fn two_timelines(
        windows: &[(Option<bool>, Option<bool>)],
    ) -> (WindowTimeline, WindowTimeline) {
        let own = windows.iter().map(|(own, _)| *own).collect();
        let far = windows.iter().map(|(_, far)| *far).collect();
        (
            WindowTimeline::from_slots(WINDOW_MS, own),
            WindowTimeline::from_slots(WINDOW_MS, far),
        )
    }

    #[test]
    fn a_stretch_of_speakers_with_him_silent_is_an_echo_span() {
        let (own, far) = two_timelines(&[(Some(false), Some(true)); 20]);
        assert_eq!(echo_spans(&own, &far), vec![(0.0, 1_000.0)]);
    }

    /// A gap between his words is not a stretch to cut out of the recording.
    #[test]
    fn a_stretch_shorter_than_the_floor_is_left_alone() {
        let mut windows = vec![(Some(true), Some(true)); 4];
        windows.extend(vec![(Some(false), Some(true)); 5]); // 250 ms
        windows.extend(vec![(Some(true), Some(true)); 4]);
        let (own, far) = two_timelines(&windows);
        assert!(echo_spans(&own, &far).is_empty());
    }

    #[test]
    fn a_span_ends_where_he_starts_speaking() {
        let mut windows = vec![(Some(false), Some(true)); 20];
        windows.extend(vec![(Some(true), Some(true)); 20]);
        let (own, far) = two_timelines(&windows);
        assert_eq!(echo_spans(&own, &far), vec![(0.0, 1_000.0)]);
    }

    #[test]
    fn nothing_is_cut_where_the_speakers_were_silent() {
        let (own, far) = two_timelines(&[(Some(false), Some(false)); 40]);
        assert!(echo_spans(&own, &far).is_empty());
    }

    /// A detector that fell behind silences nothing: its windows are
    /// unanswered, and an unanswered window is never a candidate.
    #[test]
    fn a_stretch_the_detector_never_saw_is_left_alone() {
        let (own, far) = two_timelines(&[(None, Some(true)); 40]);
        assert!(echo_spans(&own, &far).is_empty());
    }

    /// One answered window in the middle is enough to break a stretch in two,
    /// and what is left either side has to stand on its own length.
    #[test]
    fn a_word_in_the_middle_breaks_the_stretch() {
        let mut windows = vec![(Some(false), Some(true)); 8];
        windows.push((Some(true), Some(true)));
        windows.extend(vec![(Some(false), Some(true)); 4]);
        let (own, far) = two_timelines(&windows);
        assert_eq!(echo_spans(&own, &far), vec![(0.0, 400.0)]);
    }
}
