//! tasks#140: where the samples go between the devices and the tracks.
//!
//! Synthetic input only. Each test counts what goes in and what comes out of
//! one stage, under the same stop sequence `stop_streams_and_force_flush` runs.
use super::*;
use std::time::Instant;
use crate::audio::device_detection::InputDeviceKind;
use crate::audio::stream::DspWorker;
use crate::audio::devices::configuration::DeviceType as EndpointKind;

const RATE: u32 = 48_000;
const BLOCK: usize = RATE as usize / 100; // 10 ms, as a capture callback delivers

/// One source's blocks covering `[from, to)` seconds of the recording clock,
/// each stamped with the clock at its end (as `process_block` stamps them)
/// plus `late(index)` for a handler that is held up.
fn blocks(
    device_type: DeviceType,
    from: f64,
    to: f64,
    value: f32,
    late: impl Fn(usize) -> f64,
) -> Vec<AudioChunk> {
    let first = (from * 100.0).round() as usize;
    let last = (to * 100.0).round() as usize;
    (first..last)
        .map(|index| AudioChunk {
            data: vec![value; BLOCK],
            sample_rate: RATE,
            timestamp: (index + 1) as f64 / 100.0 + late(index),
            chunk_id: index as u64,
            device_type: device_type.clone(),
        })
        .collect()
}

struct Tracks {
    mixed: Vec<f32>,
    mic: Vec<f32>,
    system: Vec<f32>,
}

/// The pipeline from its input channel to the recording channel, stopped the
/// way a recording is stopped: flush signal, three more, then the channel
/// closes.
async fn through_the_pipeline(mut chunks: Vec<AudioChunk>) -> Tracks {
    let state = RecordingState::new();
    state.start_recording().expect("recording state");
    let (audio_sender, audio_receiver) = mpsc::unbounded_channel::<AudioChunk>();
    let (transcription_sender, _transcription) = mpsc::unbounded_channel();
    let (recording_sender, mut recording) = mpsc::unbounded_channel::<AudioChunk>();
    let mut pipeline = AudioPipeline::new(
        audio_receiver,
        transcription_sender,
        state.clone(),
        0,
        RATE,
        "Test microphone".to_string(),
        InputDeviceKind::Unknown,
        "Test system audio".to_string(),
        InputDeviceKind::Unknown,
    );
    pipeline.recording_sender_for_mixed = Some(recording_sender);
    let running = tokio::spawn(async move { pipeline.run().await });

    // Arrival order is clock order, both sources interleaved.
    chunks.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap());
    for chunk in chunks {
        audio_sender.send(chunk).unwrap();
    }

    state.stop_recording();
    for i in 0..4u64 {
        let _ = audio_sender.send(AudioChunk {
            data: vec![],
            sample_rate: 16000,
            timestamp: 0.0,
            chunk_id: u64::MAX - i.saturating_sub(1),
            device_type: DeviceType::Microphone,
        });
    }
    drop(audio_sender);
    running.await.expect("pipeline task").expect("pipeline run");

    let mut tracks = Tracks { mixed: vec![], mic: vec![], system: vec![] };
    while let Ok(chunk) = recording.try_recv() {
        match chunk.device_type {
            DeviceType::Mixed => tracks.mixed.extend(chunk.data),
            DeviceType::Microphone => tracks.mic.extend(chunk.data),
            DeviceType::System => tracks.system.extend(chunk.data),
        }
    }
    tracks
}

fn seconds(samples: usize) -> f64 {
    samples as f64 / RATE as f64
}

fn first_sound(samples: &[f32]) -> f64 {
    seconds(samples.iter().position(|s| s.abs() > 1e-6).unwrap_or(samples.len()))
}

fn report(case: &str, delivered_mic: f64, tracks: &Tracks) {
    eprintln!(
        "[#140] {case}: mic delivered {delivered_mic:.3}s -> audio {:.3}s, mic {:.3}s, system {:.3}s; \
         system sound starts at {:.3}s",
        seconds(tracks.mixed.len()),
        seconds(tracks.mic.len()),
        seconds(tracks.system.len()),
        first_sound(&tracks.system),
    );
}

const WINDOW: f64 = 0.05;

#[tokio::test]
async fn both_sources_from_the_first_block_to_the_last() {
    let mut chunks = blocks(DeviceType::Microphone, 0.0, 10.0, 0.1, |_| 0.0);
    chunks.extend(blocks(DeviceType::System, 0.0, 10.0, 0.1, |_| 0.0));
    let tracks = through_the_pipeline(chunks).await;
    report("both continuous", 10.0, &tracks);
    assert!((seconds(tracks.mixed.len()) - 10.0).abs() <= WINDOW);
}

/// What the dev logs of 30.09–01.10 show: the loopback stream opens about
/// three seconds after the microphone, because finding the output device
/// takes that long.
#[tokio::test]
async fn system_audio_opening_three_seconds_late() {
    let mut chunks = blocks(DeviceType::Microphone, 0.0, 10.0, 0.1, |_| 0.0);
    chunks.extend(blocks(DeviceType::System, 3.0, 10.0, 0.1, |_| 0.0));
    let tracks = through_the_pipeline(chunks).await;
    report("system opens 3s late", 10.0, &tracks);
    assert!((seconds(tracks.mixed.len()) - 10.0).abs() <= WINDOW);
}

/// Loopback with nothing playing hands over no blocks at all.
#[tokio::test]
async fn system_audio_that_never_delivers() {
    let chunks = blocks(DeviceType::Microphone, 0.0, 10.0, 0.1, |_| 0.0);
    let tracks = through_the_pipeline(chunks).await;
    report("system never delivers", 10.0, &tracks);
    assert!((seconds(tracks.mixed.len()) - 10.0).abs() <= WINDOW);
}

/// Loopback that goes quiet in the middle, and a microphone handler that
/// falls 350 ms behind and catches up in a burst, as measured in September.
#[tokio::test]
async fn jitter_and_a_silent_stretch() {
    let late = |index: usize| if (300..600).contains(&index) { 0.35 } else { 0.0 };
    let mut chunks = blocks(DeviceType::Microphone, 0.0, 10.0, 0.1, late);
    chunks.extend(blocks(DeviceType::System, 0.0, 2.0, 0.1, |_| 0.0));
    chunks.extend(blocks(DeviceType::System, 6.0, 10.0, 0.1, |_| 0.0));
    let tracks = through_the_pipeline(chunks).await;
    report("jitter + silent loopback 2-6s", 10.0, &tracks);
    assert!((seconds(tracks.mixed.len()) - 10.0).abs() <= WINDOW);
}

fn microphone_capture(state: Arc<RecordingState>) -> AudioCapture {
    AudioCapture::new(
        Arc::new(AudioDevice::new("Test microphone".to_string(), EndpointKind::Input)),
        state,
        RATE,
        1,
        DeviceType::Microphone,
        None,
    )
}

fn delivered(receiver: &mut mpsc::UnboundedReceiver<AudioChunk>) -> f64 {
    let mut samples = 0;
    while let Ok(chunk) = receiver.try_recv() {
        samples += chunk.data.len();
    }
    seconds(samples)
}

/// Stop in the order `stop_streams_and_force_flush` runs it: the device side
/// stops (its sender goes), the processing thread works through what was
/// admitted, and only then does the recording state take the pipeline's
/// sender away. Before, the state went first, and whatever was still queued
/// at the click - the last words - was processed into nothing.
#[test]
fn blocks_queued_when_stop_is_pressed_still_reach_the_pipeline() {
    let state = RecordingState::new();
    state.start_recording().unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let capture = microphone_capture(state.clone());

    // A whole second admitted and still queued at the click: the worst case,
    // a processing thread that has not got to any of it yet.
    let (blocks, queue) = crossbeam::channel::unbounded();
    for _ in 0..100 {
        let muted = capture.admit().expect("recording");
        blocks.send((vec![0.1; BLOCK], muted, Instant::now())).unwrap();
    }
    drop(blocks); // the device side stops
    let worker = DspWorker::spawn(capture, queue, "Microphone".to_string(), || {}).unwrap();
    worker.wait();
    state.stop_recording();

    let got = delivered(&mut receiver);
    eprintln!("[#140] admitted 1.000s before Stop, reached the pipeline {got:.3}s");
    assert!((got - 1.0).abs() < 1e-9, "reached the pipeline: {got:.3}s");
}

/// Pause is decided when a block is captured. A block admitted before the
/// click and processed after it is the end of the segment and is kept; a
/// block arriving during the pause is not admitted at all.
#[test]
fn pause_cuts_at_the_click_not_when_the_block_is_processed() {
    let state = RecordingState::new();
    state.start_recording().unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let capture = microphone_capture(state.clone());

    let admitted: Vec<bool> = (0..30).map(|_| capture.admit().expect("recording")).collect();
    state.pause_recording().unwrap();
    for muted in admitted {
        capture.process_block(&vec![0.1; BLOCK], muted);
    }
    assert_eq!(capture.admit(), None, "a block arriving during the pause");
    let got = delivered(&mut receiver);
    eprintln!("[#140] 0.300s captured before Pause, processed after it -> {got:.3}s kept");
    assert!((got - 0.3).abs() < 1e-9, "kept: {got:.3}s");
}

/// A 16 kHz device is resampled in fixed chunks, and the last partial chunk
/// used to stay in the resampler when the stream ended (9.968 s of 10).
/// `finish` hands it on.
#[test]
fn the_resampler_tail_is_handed_on_when_the_stream_ends() {
    let state = RecordingState::new();
    state.start_recording().unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let rate = 16_000u32;
    let capture = AudioCapture::new(
        Arc::new(AudioDevice::new("Test microphone".to_string(), EndpointKind::Input)),
        state.clone(),
        rate,
        1,
        DeviceType::Microphone,
        None,
    );

    let block = rate as usize / 100;
    for index in 0..1_000 {
        let samples: Vec<f32> = (0..block)
            .map(|i| ((index * block + i) as f32 * 0.05).sin() * 0.2)
            .collect();
        capture.process_block(&samples, false);
    }
    let before = delivered(&mut receiver);
    capture.finish();
    let got = before + delivered(&mut receiver);
    eprintln!("[#140] 10.000s at 16 kHz: {before:.3}s before finish, {got:.3}s after");
    assert!((got - 10.0).abs() < 0.002, "handed on: {got:.3}s");
}

/// How far ahead of the device the microphone chain runs in this (debug)
/// build: the backlog that Stop can throw away only exists when it does not.
#[test]
fn microphone_chain_real_time_factor() {
    for rate in [48_000u32, 44_100, 16_000] {
        let state = RecordingState::new();
        state.start_recording().unwrap();
        let (sender, mut receiver) = mpsc::unbounded_channel();
        state.set_audio_sender(sender);
        let capture = AudioCapture::new(
            Arc::new(AudioDevice::new("Test microphone".to_string(), EndpointKind::Input)),
            state.clone(),
            rate,
            1,
            DeviceType::Microphone,
            None,
        );
        let block = rate as usize / 100;
        let seconds_in = 10usize;
        let started = std::time::Instant::now();
        for index in 0..seconds_in * 100 {
            let samples: Vec<f32> = (0..block)
                .map(|i| ((index * block + i) as f32 * 0.05).sin() * 0.2)
                .collect();
            capture.process_block(&samples, false);
        }
        let took = started.elapsed().as_secs_f64();
        let out = delivered(&mut receiver);
        eprintln!(
            "[#140] mic chain at {rate} Hz: {seconds_in}s of audio in {took:.3}s ({:.1}x real time), \
             {out:.3}s handed on",
            seconds_in as f64 / took
        );
    }
}
