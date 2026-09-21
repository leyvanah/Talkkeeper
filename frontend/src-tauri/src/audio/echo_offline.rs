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
//! available afterwards — but not by asking whether the microphone *looks
//! like* the system track. That was the first attempt, and on a real call it
//! found nothing: half of a dialogue is the owner talking alone, which pulls
//! any correlation towards zero, and what Windows leaves of the echo is too
//! mangled to keep its shape.
//!
//! What survives any canceller is loudness. The owner's own voice arrives at
//! his microphone at the level he speaks at; the other person arrives there
//! only as what the room and the canceller let through, well below that. So
//! the owner's speaking level is measured from the stretches where he talks
//! alone, and while the other person is playing, a microphone well below that
//! level is echo.
//!
//! **What it will not do is cut the owner out.** Talking over the other person
//! he is at his own level and is left alone. His solo speech is never even a
//! candidate: only windows where the speakers were playing are. A recording
//! where he never speaks alone gives no level to measure against, and then
//! nothing is cut at all.

/// How much of the recording one measurement covers.
///
/// Short enough to place a span inside a sentence, long enough that a single
/// consonant does not decide anything.
pub const WINDOW_MS: f64 = 25.0;

/// How long after the speakers played the microphone may still be hearing
/// them: the sound card, the air, and the microphone's own buffering.
const ECHO_REACH_MS: f64 = 500.0;

/// Below this a window carries nothing, on either track.
const SILENCE_RMS: f32 = 0.002;

/// Below this the microphone is not the owner speaking, for the purpose of
/// learning how loud he speaks.
const VOICE_FLOOR_RMS: f32 = 0.01;

/// A microphone quieter than this share of the owner's own speaking level,
/// while the speakers play, is the speakers. About −9 dB: far below anyone
/// talking into their own microphone, well above what a canceller lets by.
const ECHO_LEVEL: f32 = 0.35;

/// How much solo speech it takes to trust the owner's level: two seconds.
const MIN_SOLO_WINDOWS: usize = 80;

/// A run shorter than this is not worth silencing: it is inside a word, and
/// removing it would leave a click where speech used to be.
const MIN_SPAN_MS: f64 = 200.0;

/// The furthest the echo is looked for when reporting the delay.
const MAX_LAG_MS: f64 = 500.0;

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

/// What a measurement found, with the numbers worth a line in the log.
#[derive(Debug, Clone, Default)]
pub struct Measurement {
    /// Stretches of the microphone that are only the speakers, in ms.
    pub spans: Vec<(f64, f64)>,
    /// How loud the owner speaks into his own microphone, when he could be
    /// measured alone.
    pub own_voice_rms: Option<f32>,
    /// The typical microphone level while the speakers played, as a share of
    /// his own voice. Low means echo that a canceller mostly handled.
    pub echo_share: Option<f32>,
    /// The delay at which the microphone best follows the speakers, and how
    /// well — reported, not relied on.
    pub lag_ms: f64,
    pub correlation: f32,
}

impl Measurement {
    pub fn covered_ms(&self) -> f64 {
        covered_ms(&self.spans)
    }
}

/// Whether the speakers played at any point in the `reach` windows up to and
/// including `index` — the stretch whose echo could be arriving now.
fn speakers_reach(system: &Envelope, index: usize, reach: usize) -> bool {
    let from = index.saturating_sub(reach);
    let to = index.min(system.len().saturating_sub(1));
    from <= to && system.rms[from..=to].iter().any(|&level| level >= SILENCE_RMS)
}

fn median(values: &mut [f32]) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Some(values[values.len() / 2])
}

/// How well the microphone follows the speakers when delayed by `lag`
/// windows, counted only while the speakers were playing: the owner talking
/// alone says nothing about echo either way.
fn correlation_at(mic: &Envelope, system: &Envelope, lag: usize) -> f32 {
    let mut pairs = 0usize;
    let (mut mic_sum, mut sys_sum, mut mic_sq, mut sys_sq, mut cross) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
    for index in lag..mic.len().min(system.len() + lag) {
        let played = system.rms[index - lag] as f64;
        if played < SILENCE_RMS as f64 {
            continue;
        }
        let heard = mic.rms[index] as f64;
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

/// The delay that best explains the microphone by the speakers, and how well.
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

/// Measure the echo in a pair of tracks.
pub fn measure(mic: &Envelope, system: &Envelope) -> Measurement {
    let (lag, correlation) = best_lag(mic, system);
    let lag_ms = lag as f64 * mic.window_ms;
    let reach = (ECHO_REACH_MS / mic.window_ms).round() as usize;

    // How loud he speaks: the microphone where the speakers were silent for
    // the whole reach of an echo, and something was being said.
    let mut solo: Vec<f32> = (0..mic.len())
        .filter(|&index| !speakers_reach(system, index, reach))
        .map(|index| mic.rms[index])
        .filter(|&level| level >= VOICE_FLOOR_RMS)
        .collect();
    if solo.len() < MIN_SOLO_WINDOWS {
        return Measurement {
            lag_ms,
            correlation,
            ..Measurement::default()
        };
    }
    let own_voice = median(&mut solo).unwrap_or(0.0);
    let ceiling = own_voice * ECHO_LEVEL;

    let mut while_playing: Vec<f32> = Vec::new();
    let mut marked = vec![false; mic.len()];
    for index in 0..mic.len() {
        if !speakers_reach(system, index, reach) {
            continue;
        }
        let heard = mic.rms[index];
        if heard >= SILENCE_RMS {
            while_playing.push(heard);
        }
        // At his own level, he is talking over them: left alone.
        if heard < ceiling {
            marked[index] = true;
        }
    }

    let echo_share = median(&mut while_playing).map(|level| level / own_voice.max(f32::EPSILON));

    Measurement {
        spans: spans_of(&marked, mic.window_ms),
        own_voice_rms: Some(own_voice),
        echo_share,
        lag_ms,
        correlation,
    }
}

/// The stretches of the microphone track that are nothing but the speakers.
///
/// Returned as `(from_ms, to_ms)` on the microphone's own clock, the same
/// shape [`super::own_speech::echo_spans`] returns, so the passes that silence
/// them do not care which of the two answered.
pub fn echo_spans(mic: &Envelope, system: &Envelope) -> Vec<(f64, f64)> {
    measure(mic, system).spans
}

/// One line for the log: what was measured and what it decided.
pub fn describe(measurement: &Measurement) -> String {
    match (measurement.own_voice_rms, measurement.echo_share) {
        (Some(own), share) => format!(
            "own voice {:.3} RMS, microphone while the speakers played at {} of it, \
             delay {:.0} ms (correlation {:.2}), {:.1}s marked as echo in {} stretches",
            own,
            share.map_or("—".to_string(), |share| format!("{:.0}%", share * 100.0)),
            measurement.lag_ms,
            measurement.correlation,
            measurement.covered_ms() / 1000.0,
            measurement.spans.len()
        ),
        (None, _) => format!(
            "the owner never spoke alone long enough to learn his level; nothing marked \
             (delay {:.0} ms, correlation {:.2})",
            measurement.lag_ms, measurement.correlation
        ),
    }
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

    /// The room this exists for, as a dialogue: the other person out of the
    /// speakers twice, reaching the microphone a little later and much
    /// quieter; the owner answering in between, at his own level.
    fn a_call_with_echo() -> (Vec<f32>, Vec<f32>) {
        let mut system = silence(12.0);
        let mut mic = silence(12.0);

        let mut seed = 7;
        burst(&mut system, 1000.0, 3000.0, 0.30, &mut seed);
        burst(&mut system, 7000.0, 9000.0, 0.30, &mut seed);

        let mut echo_seed = 7;
        burst(&mut mic, 1120.0, 3120.0, 0.06, &mut echo_seed);
        burst(&mut mic, 7120.0, 9120.0, 0.06, &mut echo_seed);

        // He answers alone, between the two.
        let mut own_seed = 42;
        burst(&mut mic, 4000.0, 6500.0, 0.30, &mut own_seed);

        (mic, system)
    }

    fn envelopes(mic: &[f32], system: &[f32]) -> (Envelope, Envelope) {
        (
            Envelope::measure(mic, RATE, WINDOW_MS),
            Envelope::measure(system, RATE, WINDOW_MS),
        )
    }

    fn overlaps(spans: &[(f64, f64)], from_ms: f64, to_ms: f64) -> bool {
        spans.iter().any(|(from, to)| *from < to_ms && *to > from_ms)
    }

    #[test]
    fn the_echo_is_found_where_the_speakers_played() {
        let (mic, system) = a_call_with_echo();
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let measured = measure(&mic_envelope, &system_envelope);

        assert!(measured.own_voice_rms.is_some(), "his level was not learned");
        let covered = measured.covered_ms();
        assert!(
            covered > 3_500.0,
            "only {covered:.0} ms of about 4000 ms of echo was found: {}",
            describe(&measured)
        );
        assert!(overlaps(&measured.spans, 1_500.0, 2_500.0));
        assert!(overlaps(&measured.spans, 7_500.0, 8_500.0));
    }

    #[test]
    fn his_own_speech_is_never_cut() {
        let (mic, system) = a_call_with_echo();
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let spans = echo_spans(&mic_envelope, &system_envelope);

        assert!(
            !overlaps(&spans, 4_000.0, 6_500.0),
            "his answer was silenced as echo: {spans:?}"
        );
    }

    #[test]
    fn the_owner_talking_over_the_other_person_is_left_alone() {
        let (mut mic, system) = a_call_with_echo();
        // He answers over the second stretch, into his own microphone.
        let mut seed = 99;
        burst(&mut mic, 7500.0, 8500.0, 0.30, &mut seed);
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let spans = echo_spans(&mic_envelope, &system_envelope);

        assert!(
            !overlaps(&spans, 7_600.0, 8_400.0),
            "his own answer was silenced as echo: {spans:?}"
        );
    }

    #[test]
    fn a_canceller_that_mangled_the_echo_does_not_hide_it() {
        // What Windows leaves: not a quieter copy, but something that only
        // happens at the same time. No shape to correlate with — and still
        // far below his voice.
        let mut system = silence(12.0);
        let mut mic = silence(12.0);
        let mut seed = 5;
        burst(&mut system, 1000.0, 3000.0, 0.30, &mut seed);
        let mut unrelated = 1234;
        burst(&mut mic, 1100.0, 3100.0, 0.05, &mut unrelated);
        let mut own_seed = 42;
        burst(&mut mic, 4000.0, 6500.0, 0.30, &mut own_seed);
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let measured = measure(&mic_envelope, &system_envelope);

        assert!(
            measured.covered_ms() > 1_500.0,
            "the mangled echo was missed: {}",
            describe(&measured)
        );
        assert!(!overlaps(&measured.spans, 4_000.0, 6_500.0));
    }

    #[test]
    fn a_recording_where_he_never_speaks_alone_is_left_alone() {
        let mut system = silence(10.0);
        let mut mic = silence(10.0);
        let mut seed = 5;
        burst(&mut system, 1000.0, 9000.0, 0.30, &mut seed);
        let mut echo_seed = 5;
        burst(&mut mic, 1100.0, 9100.0, 0.06, &mut echo_seed);
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let measured = measure(&mic_envelope, &system_envelope);

        assert!(measured.own_voice_rms.is_none());
        assert!(measured.spans.is_empty());
    }

    #[test]
    fn with_nothing_played_nothing_is_cut() {
        let mut seed = 3;
        let mut mic = silence(10.0);
        burst(&mut mic, 1000.0, 4000.0, 0.30, &mut seed);
        let system = silence(10.0);
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        assert!(echo_spans(&mic_envelope, &system_envelope).is_empty());
    }

    #[test]
    fn the_delay_between_the_tracks_is_reported() {
        let (mic, system) = a_call_with_echo();
        let (mic_envelope, system_envelope) = envelopes(&mic, &system);

        let measured = measure(&mic_envelope, &system_envelope);

        assert!(
            (measured.lag_ms - 120.0).abs() <= 50.0,
            "reported a delay of {:.0} ms instead of about 120 ms",
            measured.lag_ms
        );
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
/// thresholds. Ignored by default because it needs a recording, and a track
/// of a protected archive is encrypted — only the app holds its key:
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

        let envelope_of = |path: &str| {
            let decoded = crate::audio::decoder::decode_audio_file(std::path::Path::new(path))
                .expect("the track could not be decoded (a track of a protected archive is encrypted)");
            let samples = decoded.to_whisper_format();
            Envelope::measure(&samples, 16_000, WINDOW_MS)
        };

        let measured = measure(&envelope_of(&mic_path), &envelope_of(&system_path));
        println!("{}", describe(&measured));
    }
}
