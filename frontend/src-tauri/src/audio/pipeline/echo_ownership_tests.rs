use super::*;
use crate::audio::echo_cancel::EchoCanceller;

const SAMPLE_RATE: u32 = 48_000;

/// One 50 ms window of far-end sound and the echo it leaves in the mic.
fn window(index: usize) -> (Vec<f32>, Vec<f32>) {
    let samples = mixing_window_samples(SAMPLE_RATE);
    let start = index * samples;
    let far: Vec<f32> = (0..samples)
        .map(|offset| {
            let t = (start + offset) as f32 / SAMPLE_RATE as f32;
            (t * 440.0 * std::f32::consts::TAU).sin() * 0.5
        })
        .collect();
    // What comes back through the room: quieter, and later.
    let mic: Vec<f32> = far.iter().map(|s| s * 0.35).collect();
    (mic, far)
}

/// While Windows is cancelling, our canceller must not touch a sample.
///
/// Subtracting an echo that is already gone is not free: it takes some of
/// the owner's voice with it, and worst of all exactly when he talks over
/// the speakers.
#[test]
fn windows_cancelling_leaves_the_microphone_untouched() {
    let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller for 48 kHz");
    let mut left_to_windows = None;

    for index in 0..10 {
        let (mic, far) = window(index);
        let out = cancel_echo_window(
            Some(&mut canceller),
            &mut left_to_windows,
            true,
            mic.clone(),
            &far,
        );
        assert_eq!(out, mic, "window {index} came back changed");
    }
}

/// And when Windows is not, ours is the only one left to do it.
#[test]
fn our_canceller_runs_when_windows_does_not() {
    let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller for 48 kHz");
    let mut left_to_windows = None;
    let mut changed = false;

    for index in 0..10 {
        let (mic, far) = window(index);
        let out = cancel_echo_window(
            Some(&mut canceller),
            &mut left_to_windows,
            false,
            mic.clone(),
            &far,
        );
        changed |= out != mic;
    }

    assert!(changed, "the microphone came back untouched with nobody else cancelling");
}

/// The defect this guards against: the answer used to be frozen when the
/// pipeline was built, and the pipeline is built *before* the capture
/// stream opens. Whatever Windows was doing by the time audio arrived, the
/// frozen answer was always the stale "no" from before the recording.
#[test]
fn the_answer_is_taken_per_window_not_once_at_the_start() {
    let mut canceller = EchoCanceller::new(SAMPLE_RATE).expect("canceller for 48 kHz");
    let mut left_to_windows = None;

    // The stream has not opened yet: nobody else is cancelling.
    for index in 0..4 {
        let (mic, far) = window(index);
        let _ = cancel_echo_window(
            Some(&mut canceller),
            &mut left_to_windows,
            false,
            mic,
            &far,
        );
    }
    assert_eq!(left_to_windows, Some(false));

    // It opens in communications mode, and from here Windows has it.
    for index in 4..8 {
        let (mic, far) = window(index);
        let out = cancel_echo_window(
            Some(&mut canceller),
            &mut left_to_windows,
            true,
            mic.clone(),
            &far,
        );
        assert_eq!(out, mic, "window {index} was cancelled twice");
    }
    assert_eq!(left_to_windows, Some(true));
}
