//! Finding the speakers' echo in a recording that nobody judged while it ran.
//!
//! The detector in [`super::own_speech`] answers during the recording and
//! writes down what it heard. Recordings made with it switched off — which is
//! the default, where Windows cancels the echo on its own — have no such
//! record, and if that cancellation was not enough, the microphone track holds
//! the other person's voice. Every later pass then reads it as the owner
//! speaking: the text is recognised twice, and the labels say two people spoke
//! at once through the whole conversation.
//!
//! Both tracks were written by one pipeline off one clock, so the answer is
//! available after the fact. Echo is the system track again: quieter, a little
//! later, and shaped the same. This measures that.
//!
//! **What it will not do is cut the owner out.** The owner talking over the
//! other person is exactly the case that must survive, so a window is only
//! called echo when the microphone is no louder than the echo of that moment
//! usually is. When the two tracks do not line up at all — a recording where
//! nothing was played, or one where the tracks are unrelated — it says so by
//! finding nothing, and every later pass behaves as it did before.

/// How much of the recording one measurement covers.
///
/// Short enough to place a span inside a sentence, long enough that a single
/// consonant does not decide anything.
pub const WINDOW_MS: f64 = 25.0;

/// The furthest the echo can lag behind what was played: the sound card, the
/// air, and the microphone's own buffering. Beyond this it is not echo.
const MAX_LAG_MS: f64 = 500.0;

/// Below this the tracks are not telling the same story, and nothing is cut.
const MIN_CORRELATION: f32 = 0.35;

/// A run shorter than this is not worth silencing: it is inside a word, and
/// removing it would leave a click where speech used to be.
const MIN_SPAN_MS: f64 = 200.0;

/// Windows quieter than this carry no speech at all, on either track.
const SILENCE_RMS: f32 = 0.002;

/// How much louder than the usual echo the microphone may be before the window
/// is read as the owner speaking rather than the speakers coming back.
const OWN_VOICE_MARGIN: f32 = 2.5;

/// The loudness of each window of a track, which is all this needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub window_ms: f64,
    pub rms: Vec<f32>,
}

impl Envelope {
    /// Measure a track. Samples are mono at `sample_rate`.
    pub fn measure(samples: &[f32], sample_rate: u32, window_ms: f64) -> Self {
        let per_window = ((window_ms / 1000.0) * sample_rate as f64).round().max(1.0) as usize;
        let mut rms = Vec::with_capacity(samples.len() / per_window + 1);
        for window in samples.chunks(per_window) {
            let sum: f64 = window.iter().map(|s| (*s as f64) * (*s as f64)).sum();
            rms.push((sum / window.len() as f64).sqrt() as f32);
        }
        Self { window_ms, rms }
    }

    fn len(&self) -> usize {
        self.rms.len()
    }
}

/// How well the microphone follows the system track when delayed by `lag`
/// windows, counted only where something was actually played.
fn correlation_at(mic: &Envelope, system: &Envelope, lag: usize) -> f32 {
    let mut pairs = 0usize;
    let mut mic_sum = 0.0f64;
    let mut sys_sum = 0.0f64;
    let mut mic_sq = 0.0f64;
    let mut sys_sq = 0.0f64;
    let mut cross = 0.0f64;

    for index in lag..mic.len().min(system.len() + lag) {
        let played = system.rms[index - lag] as f64;
        let heard = mic.rms[index] as f64;
        if played < SILENCE_RMS as f64 && heard < SILENCE_RMS as f64 {
            continue;
        }
        pairs += 1;
        mic_sum += heard;
        sys_sum += played;
        mic_sq += heard * heard;
        sys_sq += played * played;
        cross += heard * played;
    }

    if pairs < 8 {
        return 0.0;
    }
    let count = pairs as f64;
    let covariance = cross / count - (mic_sum / count) * (sys_sum / count);
    let mic_variance = (mic_sq / count - (mic_sum / count).powi(2)).max(0.0);
    let sys_variance = (sys_sq / count - (sys_sum / count).powi(2)).max(0.0);
    let denominator = (mic_variance * sys_variance).sqrt();
    if denominator <= f64::EPSILON {
        return 0.0;
    }
    (covariance / denominator) as f32
}

/// The delay that best explains the microphone by the system track, and how
/// well it explains it.
pub fn best_lag(mic: &Envelope, system: &Envelope) -> (usize, f32) {
    let max_lag = (MAX_LAG_MS / mic.window_ms).round() as usize;
    let mut best = (0usize, f32::MIN);
    for lag in 0..=max_lag {
        let score = correlation_at(mic, system, lag);
        if score > best.1 {
            best = (lag, score);
        }
    }
    best
}

/// The stretches of the microphone track that are nothing but the speakers.
///
/// Returned as `(from_ms, to_ms)` on the microphone's own clock, the same
/// shape [`super::own_speech::echo_spans`] returns, so the passes that silence
/// them do not care which of the two answered.
pub fn echo_spans(mic: &Envelope, system: &Envelope) -> Vec<(f64, f64)> {
    let (lag, correlation) = best_lag(mic, system);
    if correlation < MIN_CORRELATION {
        return Vec::new();
    }

    // What the echo of this room usually sounds like, relative to what was
    // played. Taken from the recording itself: a laptop with its speakers up
    // and a headset are different rooms, and neither is a constant we could
    // have written down.
    let mut ratios: Vec<f32> = Vec::new();
    for index in lag..mic.len().min(system.len() + lag) {
        let played = system.rms[index - lag];
        if played < SILENCE_RMS {
            continue;
        }
        ratios.push(mic.rms[index] / played);
    }
    if ratios.len() < 8 {
        return Vec::new();
    }
    ratios.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let typical = ratios[ratios.len() / 2];
    let ceiling = typical * OWN_VOICE_MARGIN;

    let mut marked = vec![false; mic.len()];
    for index in lag..mic.len().min(system.len() + lag) {
        let played = system.rms[index - lag];
        let heard = mic.rms[index];
        if played < SILENCE_RMS {
            continue;
        }
        // Loud enough to be his own voice over the echo: left alone.
        if heard > played * ceiling.max(1.0) || heard > played * OWN_VOICE_MARGIN.max(ceiling) {
            continue;
        }
        marked[index] = true;
    }

    spans_of(&marked, mic.window_ms)
}

/// Runs of marked windows, as milliseconds, with the short ones dropped.
fn spans_of(marked: &[bool], window_ms: f64) -> Vec<(f64, f64)> {
    let mut spans = Vec::new();
    let mut start: Option<usize> = None;
    for (index, &is_echo) in marked.iter().enumerate() {
        match (is_echo, start) {
            (true, None) => start = Some(index),
            (false, Some(from)) => {
                push_span(&mut spans, from, index, window_ms);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(from) = start {
        push_span(&mut spans, from, marked.len(), window_ms);
    }
    spans
}

fn push_span(spans: &mut Vec<(f64, f64)>, from: usize, to: usize, window_ms: f64) {
    let length_ms = (to - from) as f64 * window_ms;
    if length_ms < MIN_SPAN_MS {
        return;
    }
    spans.push((from as f64 * window_ms, to as f64 * window_ms));
}

/// How much of the track the spans cover, for the log.
pub fn covered_ms(spans: &[(f64, f64)]) -> f64 {
    spans.iter().map(|(from, to)| to - from).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// A burst of noise, which is as much like speech as this needs.
    fn burst(samples: &mut [f32], from_ms: f64, to_ms: f64, level: f32, seed: &mut u32) {
        let first = ((from_ms / 1000.0) * RATE as f64) as usize;
        let last = (((to_ms / 1000.0) * RATE as f64) as usize).min(samples.len());
        for sample in &mut samples[first..last] {
            // A small deterministic generator: the test must not depend on
            // which random crate is in the tree.
            *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = ((*seed >> 8) as f32 / 8_388_608.0) - 1.0;
            *sample += noise * level;
        }
    }

    fn silence(seconds: f64) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f64) as usize]
    }

    /// The room this exists for: the other person out of the speakers, the
    /// microphone hearing them a little later and much quieter.
    fn a_call_with_echo() -> (Vec<f32>, Vec<f32>) {
        let mut seed = 7;
        let mut system = silence(10.0);
        let mut mic = silence(10.0);

        // The other person speaks twice.
        burst(&mut system, 1000.0, 3000.0, 0.30, &mut seed);
        burst(&mut system, 6000.0, 8000.0, 0.30, &mut seed);

        // The same sound reaches the microphone 120 ms later, five times
        // quieter.
        let mut echo_seed = 7;
        burst(&mut mic, 1120.0, 3120.0, 0.06, &mut echo_seed);
        burst(&mut mic, 6120.0, 8120.0, 0.06, &mut echo_seed);

        (mic, system)
    }

    fn envelopes(mic: &[f32], system: &[f32]) -> (Envelope, Envelope) {
        (
            Envelope::measure(mic, RATE, WINDOW_MS),
            Envelope::measure(system, RATE, WINDOW_MS),
        )
    }

    #[test]
    fn the_echo_is_found_where_the_speakers_played() {
        let (mic, system) = a_call_with_echo();
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let spans = echo_spans(&mic_envelope, &system_envelope);

        assert!(!spans.is_empty(), "the echo was not found at all");
        let covered = covered_ms(&spans);
        assert!(
            covered > 3_000.0,
            "only {covered:.0} ms of about 4000 ms of echo was found"
        );
        // Nothing is cut where neither track carried anything.
        assert!(
            spans.iter().all(|(from, _)| *from > 800.0),
            "silence before the first word was cut: {spans:?}"
        );
    }

    #[test]
    fn the_owner_talking_over_the_other_person_is_left_alone() {
        let (mut mic, system) = a_call_with_echo();
        // He answers over the second stretch, loudly, into his own microphone.
        let mut seed = 99;
        burst(&mut mic, 6500.0, 7500.0, 0.40, &mut seed);
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let spans = echo_spans(&mic_envelope, &system_envelope);

        let cut_while_he_spoke = spans
            .iter()
            .any(|(from, to)| *from < 7_400.0 && *to > 6_600.0);
        assert!(
            !cut_while_he_spoke,
            "his own answer was silenced as echo: {spans:?}"
        );
    }

    #[test]
    fn two_unrelated_tracks_are_left_alone() {
        // A recording where the speakers played nothing: the microphone has
        // his voice and the system track has silence. There is nothing to
        // correlate, and nothing may be cut.
        let mut seed = 3;
        let mut mic = silence(10.0);
        burst(&mut mic, 1000.0, 4000.0, 0.30, &mut seed);
        let system = silence(10.0);
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        assert!(echo_spans(&mic_envelope, &system_envelope).is_empty());
    }

    #[test]
    fn the_delay_between_the_tracks_is_measured() {
        let (mic, system) = a_call_with_echo();
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let (lag, correlation) = best_lag(&mic_envelope, &system_envelope);

        let lag_ms = lag as f64 * WINDOW_MS;
        assert!(
            (lag_ms - 120.0).abs() <= 50.0,
            "measured a delay of {lag_ms:.0} ms instead of about 120 ms"
        );
        assert!(correlation > MIN_CORRELATION, "correlation {correlation}");
    }

    #[test]
    fn a_short_flicker_is_not_worth_a_span() {
        let marked = [false, true, true, false];
        assert!(spans_of(&marked, WINDOW_MS).is_empty());

        let long = vec![true; 20];
        assert_eq!(spans_of(&long, WINDOW_MS), vec![(0.0, 500.0)]);
    }
}

/// Measure two real tracks and report the numbers, without reading a word of
/// what was said.
///
/// Synthetic noise proves the arithmetic; only a real room proves the
/// thresholds. Ignored by default because it needs a recording:
///
/// ```text
/// TK_MIC=...\.work\mic.flac TK_SYSTEM=...\.work\system.flac \
///   cargo test --lib echo_offline::real_tracks -- --ignored --nocapture
/// ```
#[cfg(test)]
mod real_tracks {
    use super::*;

    #[test]
    #[ignore = "needs a recording; see the module docs"]
    fn measure_a_recording() {
        let mic_path = std::env::var("TK_MIC").expect("TK_MIC");
        let system_path = std::env::var("TK_SYSTEM").expect("TK_SYSTEM");

        let measure = |path: &str| {
            let decoded = crate::audio::decoder::decode_audio_file(std::path::Path::new(path))
                .expect("the track could not be decoded (a track of a protected archive is encrypted: only the app holds the key)");
            let seconds = decoded.duration_seconds;
            let samples = decoded.to_whisper_format();
            let rate = 16_000;
            (Envelope::measure(&samples, rate, WINDOW_MS), seconds)
        };

        let (mic, mic_seconds) = measure(&mic_path);
        let (system, system_seconds) = measure(&system_path);
        let (lag, correlation) = best_lag(&mic, &system);
        let spans = echo_spans(&mic, &system);
        let covered = covered_ms(&spans);

        println!("microphone: {mic_seconds:.0}s, system: {system_seconds:.0}s");
        println!(
            "delay: {:.0} ms, correlation: {correlation:.2}",
            lag as f64 * WINDOW_MS
        );
        println!(
            "would silence {:.0}s in {} stretches ({:.0}% of the microphone track)",
            covered / 1000.0,
            spans.len(),
            covered / (mic_seconds * 10.0)
        );
    }
}
