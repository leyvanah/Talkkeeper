use super::*;
use crate::audio::own_speech::{is_only_the_speakers, WindowTimeline};

const WINDOW_MS: f64 = MIXING_WINDOW_MS as f64;

/// Build the two timelines a stretch of recording would leave behind.
/// Each entry covers one 50 ms window.
fn timelines(
    own: &[Option<bool>],
    far: &[Option<bool>],
) -> (WindowTimeline, WindowTimeline) {
    let mut own_timeline = WindowTimeline::new(WINDOW_MS);
    let mut far_timeline = WindowTimeline::new(WINDOW_MS);
    for value in own {
        own_timeline.push(*value);
    }
    for value in far {
        far_timeline.push(*value);
    }
    (own_timeline, far_timeline)
}

fn verdict(own: &[Option<bool>], far: &[Option<bool>]) -> bool {
    let (own_timeline, far_timeline) = timelines(own, far);
    let span_ms = own.len().max(far.len()) as f64 * WINDOW_MS;
    let (own_ms, far_ms) = measure_segment(&own_timeline, &far_timeline, 0.0, span_ms);
    is_only_the_speakers(own_ms, far_ms)
}

/// The complaint: a video played through the speakers, came back into the
/// microphone, and was written down as the owner talking.
#[test]
fn four_seconds_of_speakers_with_a_silent_detector_is_echo() {
    let windows = 80;
    assert!(verdict(&vec![Some(false); windows], &vec![Some(true); windows]));
}

/// And the case that made the detector necessary: he says a short word
/// while the other person is still talking. The driver's canceller takes
/// his voice down, but the detector still heard him, and the segment stays.
#[test]
fn a_word_over_the_speakers_is_his_and_is_kept() {
    let windows = 80;
    let mut own = vec![Some(false); windows];
    for slot in own.iter_mut().skip(20).take(6) {
        *slot = Some(true);
    }
    assert!(!verdict(&own, &vec![Some(true); windows]));
}

/// With nothing playing there is no echo to mistake anything for, however
/// quiet the detector was. This is the failure that would cost him words
/// in an ordinary session, where the speakers are silent the whole hour.
#[test]
fn speech_in_a_silent_room_is_kept_however_quiet_the_detector_was() {
    let windows = 80;
    assert!(!verdict(&vec![Some(false); windows], &vec![Some(false); windows]));
}

/// A detector that fell behind must not cost him a segment either.
#[test]
fn a_segment_the_detector_never_saw_is_kept() {
    let windows = 80;
    assert!(!verdict(&vec![None; windows], &vec![Some(true); windows]));
}

/// Nor may a gap in what the speakers were doing turn into a drop: an
/// unobserved window says the speakers were silent, and silence keeps.
#[test]
fn a_gap_in_the_far_end_timeline_keeps_the_segment() {
    let windows = 80;
    assert!(!verdict(&vec![Some(false); windows], &vec![None; windows]));
}

/// A cough or a chair with the speakers quiet is kept — it is his room.
#[test]
fn a_short_noise_with_the_speakers_quiet_is_kept() {
    assert!(!verdict(&vec![Some(false); 8], &vec![Some(false); 8]));
}

#[test]
fn a_window_of_playing_audio_is_seen_and_a_silent_one_is_not() {
    let samples = mixing_window_samples(48_000);
    let playing: Vec<f32> = (0..samples)
        .map(|index| (index as f32 / 48_000.0 * 220.0 * std::f32::consts::TAU).sin() * 0.3)
        .collect();

    assert!(far_end_is_playing(&playing));
    assert!(!far_end_is_playing(&vec![0.0; samples]));
    assert!(!far_end_is_playing(&[]));
}
