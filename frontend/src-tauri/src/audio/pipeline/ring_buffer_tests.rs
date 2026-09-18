use super::*;
use crate::audio::devices::DeviceType as AudioDeviceType;

/// What the microphone actually offers, so the capture format can be
/// compared with what other apps get:
///   cargo test --lib pipeline -- --ignored --nocapture
#[test]
#[ignore = "prints the host's audio configuration"]
fn prints_input_device_configurations() {
    use cpal::traits::{DeviceTrait, HostTrait};
    let host = cpal::default_host();
    for device in host.input_devices().expect("input devices") {
        let name = device.name().unwrap_or_else(|_| "<unnamed>".to_string());
        println!("device: {}", name);
        match device.default_input_config() {
            Ok(config) => println!(
                "  default: {} ch, {} Hz, {:?}",
                config.channels(),
                config.sample_rate().0,
                config.sample_format()
            ),
            Err(error) => println!("  default: unavailable ({})", error),
        }
        if let Ok(configs) = device.supported_input_configs() {
            for config in configs {
                println!(
                    "  supported: {} ch, {}-{} Hz, {:?}",
                    config.channels(),
                    config.min_sample_rate().0,
                    config.max_sample_rate().0,
                    config.sample_format()
                );
            }
        }
    }
}

/// Capture callbacks arrive a few milliseconds early or late. The samples
/// themselves are continuous, so the stream must come out of the buffer
/// exactly as it went in - no silence spliced in, nothing trimmed away.
#[test]
fn arrival_jitter_does_not_cut_into_the_stream() {
    let sample_rate = 48_000u32;
    let mut buffer = AudioMixerRingBuffer::new(sample_rate, true, false);

    let block = sample_rate as usize / 100; // 10 ms, as the device delivers
    let blocks = 200; // two seconds
    let mut sent = Vec::with_capacity(block * blocks);
    let mut received: Vec<f32> = Vec::with_capacity(block * blocks);
    let mut clock = 0.0f64;

    for index in 0..blocks {
        // The value of each sample says where it came from, so a gap or a
        // trim shows up as a break in the sequence.
        let samples: Vec<f32> = (0..block)
            .map(|i| (index * block + i) as f32)
            .collect();
        sent.extend_from_slice(&samples);

        clock += block as f64 / sample_rate as f64;
        // Arrival wanders by up to ±4 ms around the true time
        let jitter = ((index % 9) as f64 - 4.0) * 0.001;
        buffer.add_samples(DeviceType::Microphone, samples, clock + jitter);

        // The pipeline drains as it goes; holding everything would hit the
        // buffer's own overflow guard and prove nothing about splicing.
        while let Some((mic, _)) = buffer.extract_window() {
            received.extend_from_slice(&mic);
        }
    }

    while let Some((mic, _)) = buffer.extract_remaining() {
        received.extend_from_slice(&mic);
    }

    assert_eq!(
        received.len(),
        sent.len(),
        "stream length changed: {} sent, {} received",
        sent.len(),
        received.len()
    );
    assert_eq!(received, sent, "the samples came back re-cut");
}

/// Feed `blocks` 10 ms blocks of ones, the first of them arriving
/// `late_by` seconds after where the clock says the stream stands, and
/// return every sample the buffer gives back.
fn run_blocks(
    buffer: &mut AudioMixerRingBuffer,
    sample_rate: u32,
    clock: &mut f64,
    blocks: usize,
    late_by: f64,
) -> Vec<f32> {
    let block = sample_rate as usize / 100;
    let mut received = Vec::new();
    for index in 0..blocks {
        *clock += block as f64 / sample_rate as f64;
        let arrival = *clock + if index == 0 { late_by } else { 0.0 };
        buffer.add_samples(DeviceType::Microphone, vec![1.0; block], arrival);
        while let Some((mic, _)) = buffer.extract_window() {
            received.extend_from_slice(&mic);
        }
    }
    received
}

/// A stall in the middle of a recording is no longer patched, and this is
/// the test that says so on purpose rather than by omission.
///
/// It used to be: a sustained shortfall was filled with silence so the two
/// channels stayed lined up. Three measured recordings showed the shortfall
/// was almost never real — every one of them delivered more audio than it
/// had running time — so the patch fired on delivery jitter and the owner
/// heard each patch as a click. Jitter of that depth cannot be told from a
/// real stall, so the choice is which mistake to make, and silence spliced
/// into speech is the one that is audible.
#[test]
fn a_stall_in_the_middle_is_left_alone() {
    let sample_rate = 48_000u32;
    let mut buffer = AudioMixerRingBuffer::new(sample_rate, true, false);
    let mut clock = 0.0;

    // The device stalls for 400 ms, and keeps delivering afterwards.
    let mut received = run_blocks(&mut buffer, sample_rate, &mut clock, 1, 0.0);
    clock += 0.4;
    received.extend(run_blocks(&mut buffer, sample_rate, &mut clock, 300, 0.0));

    let silence = received.iter().filter(|value| **value == 0.0).count();
    assert_eq!(
        silence, 0,
        "{} samples of silence spliced in for a stall that is no longer patched",
        silence
    );
}

/// The break that is still acted on: large enough that the stream has to be
/// picked up again rather than patched. Both timelines reset, and the audio
/// after it is kept.
#[test]
fn a_break_of_seconds_resets_the_timelines() {
    let sample_rate = 48_000u32;
    let mut buffer = AudioMixerRingBuffer::new(sample_rate, true, false);
    let mut clock = 0.0;

    let mut received = run_blocks(&mut buffer, sample_rate, &mut clock, 200, 0.0);
    clock += 6.0;
    received.extend(run_blocks(&mut buffer, sample_rate, &mut clock, 300, 0.0));

    assert_eq!(buffer.timeline_resets, 1, "the break should have reset once");
    let silence = received.iter().filter(|value| **value == 0.0).count();
    assert_eq!(silence, 0, "a reset picks the stream up, it does not pad it");
    assert!(
        received.len() >= sample_rate as usize * 4,
        "the audio on both sides of the break should survive, got {} samples",
        received.len()
    );
}

/// The clock is read inside the capture handler, so work anywhere in that
/// path makes a block *look* late while the samples themselves stayed
/// continuous. Under load that happened dozens of times a minute, and each
/// correction spliced silence into the middle of speech.
#[test]
fn a_late_arrival_is_not_mistaken_for_a_gap() {
    let sample_rate = 48_000u32;
    let mut buffer = AudioMixerRingBuffer::new(sample_rate, true, false);
    let mut clock = 0.0;

    // Well into the stream one block reaches the buffer 250 ms late,
    // then delivery goes back to normal. Nothing was actually missed.
    let mut received = run_blocks(&mut buffer, sample_rate, &mut clock, 150, 0.0);
    received.extend(run_blocks(&mut buffer, sample_rate, &mut clock, 1, 0.25));
    received.extend(run_blocks(&mut buffer, sample_rate, &mut clock, 300, 0.0));

    let silence = received.iter().filter(|value| **value == 0.0).count();
    assert_eq!(silence, 0, "{} samples of silence spliced into the stream", silence);
}

/// The failure the owner actually heard: not one late block, but a handler
/// held up again and again for longer than the observation window, so every
/// reading in it agreed on a shortfall that was never in the audio.
///
/// Modelled on the measured recording — blocks arriving 120ms late for two
/// and a half seconds, twice, with the stream itself continuous throughout.
/// Under the old 100ms threshold this spliced silence into the middle of
/// speech; the recording it came from delivered 101.4% of its own length
/// with nothing dropped, so there was nothing to repair.
#[test]
fn a_handler_held_up_for_seconds_is_not_mistaken_for_a_gap() {
    let sample_rate = 48_000u32;
    let mut buffer = AudioMixerRingBuffer::new(sample_rate, true, false);
    let mut clock = 0.0;

    let mut received = run_blocks(&mut buffer, sample_rate, &mut clock, 200, 0.0);
    for _ in 0..2 {
        // 250 blocks of 10ms, each reading pushed 120ms late by the work
        // ahead of it. No audio is missing: every block is delivered whole.
        for _ in 0..250 {
            received.extend(run_blocks(&mut buffer, sample_rate, &mut clock, 1, 0.12));
        }
        received.extend(run_blocks(&mut buffer, sample_rate, &mut clock, 200, 0.0));
    }

    let silence = received.iter().filter(|value| **value == 0.0).count();
    assert_eq!(
        silence, 0,
        "{} samples of silence spliced into a stream that never broke",
        silence
    );
}

#[test]
fn aligns_late_source_to_recording_clock() {
    let mut ring = AudioMixerRingBuffer::with_window_ms(10, true, true, 600.0);
    ring.add_samples(DeviceType::Microphone, vec![1.0; 6], 0.6);
    ring.add_samples(DeviceType::System, vec![2.0; 6], 1.0);

    let (mic, system) = ring.extract_window().unwrap();
    assert_eq!(mic, vec![1.0; 6]);
    assert_eq!(system, vec![0.0, 0.0, 0.0, 0.0, 2.0, 2.0]);
}

#[test]
fn falls_back_when_requested_source_did_not_start() {
    let mut ring = AudioMixerRingBuffer::with_window_ms(10, true, true, 600.0);
    ring.add_samples(DeviceType::Microphone, vec![1.0; 6], 0.6);
    assert!(!ring.can_mix());

    ring.set_enabled(true, false);
    let (mic, system) = ring.extract_window().unwrap();
    assert_eq!(mic, vec![1.0; 6]);
    assert_eq!(system, vec![0.0; 6]);
}

#[test]
fn tail_tracks_keep_equal_lengths() {
    let mut ring = AudioMixerRingBuffer::with_window_ms(10, true, true, 600.0);
    ring.add_samples(DeviceType::Microphone, vec![1.0; 4], 0.4);
    ring.add_samples(DeviceType::System, vec![2.0; 2], 0.2);

    let (mic, system) = ring.extract_remaining().unwrap();
    assert_eq!(mic.len(), system.len());
    assert_eq!(mic.len(), 4);
}

#[test]
fn long_clock_gap_resets_without_allocating_silence() {
    let mut ring = AudioMixerRingBuffer::with_window_ms(10, true, true, 600.0);
    ring.add_samples(DeviceType::Microphone, vec![1.0; 2], 0.2);
    ring.add_samples(DeviceType::System, vec![2.0; 2], 0.2);

    // A jump this large is still only acted on once it has held for the
    // observation window; until then it could be this thread, not the clock.
    let mut clock = 60.2;
    for _ in 0..12 {
        ring.add_samples(DeviceType::Microphone, vec![3.0; 2], clock);
        clock += 0.2;
    }

    assert_eq!(ring.timeline_resets, 1);
    assert!(ring.system_buffer.is_empty());
    assert!(
        ring.mic_inserted_samples < 10,
        "a reset must not also fill the gap ({} samples inserted)",
        ring.mic_inserted_samples
    );
}

#[test]
fn production_window_is_fifty_milliseconds() {
    let ring = AudioMixerRingBuffer::new(48_000, true, true);

    assert_eq!(ring.window_size_samples, 2_400);
    // A second, and deliberately more than the twelve windows `can_mix`
    // waits before mixing a source's partner without it: a cap below that
    // would discard the samples being waited for.
    assert_eq!(ring.max_buffer_size, 48_000);
}

#[test]
fn production_window_aligns_split_callbacks_without_padding() {
    let mut ring = AudioMixerRingBuffer::new(48_000, true, true);
    ring.add_samples(DeviceType::Microphone, vec![1.0; 2_400], 0.05);
    ring.add_samples(DeviceType::System, vec![2.0; 1_200], 0.025);
    assert!(!ring.can_mix());

    ring.add_samples(DeviceType::System, vec![2.0; 1_200], 0.05);
    let (mic, system) = ring.extract_window().unwrap();

    assert_eq!(mic, vec![1.0; 2_400]);
    assert_eq!(system, vec![2.0; 2_400]);
}

#[test]
fn completed_vad_segments_are_queued() {
    let (sender, mut receiver) = mpsc::unbounded_channel();
    let mut chunk_id = 0;
    let segment = SpeechSegment {
        samples: vec![0.25; 1_600],
        start_timestamp_ms: 1_000.0,
        end_timestamp_ms: 1_100.0,
        confidence: 0.8,
    };

    AudioPipeline::enqueue_source_speech(
        vec![segment],
        DeviceType::Microphone,
        &sender,
        &mut chunk_id,
    );

    let segment = receiver
        .try_recv()
        .expect("completed speech should be queued");
    assert!(!segment.data.is_empty());
    assert_eq!(chunk_id, 1);
}

#[test]
fn muted_microphone_capture_sends_aligned_silence() {
    let state = RecordingState::new();
    state.start_recording().unwrap();
    state.set_microphone_muted(true);
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let device = Arc::new(AudioDevice::new(
        "Test microphone".to_string(),
        AudioDeviceType::Input,
    ));
    let capture = AudioCapture::new(device, state, 48_000, 1, DeviceType::Microphone, None);

    capture.process_audio_data(&vec![0.5; 1_024]);

    let chunk = receiver
        .try_recv()
        .expect("muted mic chunk should be retained");
    assert_eq!(chunk.data.len(), 1_024);
    assert!(chunk.data.iter().all(|sample| *sample == 0.0));
    assert_eq!(chunk.device_type, DeviceType::Microphone);
}

/// Pause has to stop the recording, not just the clock: the streams stay
/// open, so anything still reaching the pipeline is audio the owner
/// believes is not being kept.
#[test]
fn paused_capture_records_nothing() {
    let state = RecordingState::new();
    state.start_recording().unwrap();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let device = Arc::new(AudioDevice::new(
        "Test microphone".to_string(),
        AudioDeviceType::Input,
    ));
    let capture = AudioCapture::new(
        device,
        Arc::clone(&state),
        48_000,
        1,
        DeviceType::Microphone,
        None,
    );

    state.pause_recording().unwrap();
    capture.process_audio_data(&vec![0.5; 1_024]);
    assert!(receiver.try_recv().is_err(), "a paused recording kept audio");

    state.resume_recording().unwrap();
    capture.process_audio_data(&vec![0.5; 1_024]);
    let chunk = receiver.try_recv().expect("resumed capture should be kept");
    assert_eq!(chunk.data.len(), 1_024);
}

/// What a real-time callback decides with atomics alone, before the block
/// goes to the processing thread: the mute seen at capture travels with
/// it, so a mute that lands while the block is queued cannot unmute it.
#[test]
fn admission_is_decided_at_capture_and_the_mute_travels_with_the_block() {
    let state = RecordingState::new();
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let device = Arc::new(AudioDevice::new(
        "Test microphone".to_string(),
        AudioDeviceType::Input,
    ));
    let capture = AudioCapture::new(
        device,
        Arc::clone(&state),
        48_000,
        1,
        DeviceType::Microphone,
        None,
    );

    assert_eq!(capture.admit(), None, "nothing is admitted before recording");
    state.start_recording().unwrap();
    assert_eq!(capture.admit(), Some(false));
    state.set_microphone_muted(true);
    let muted = capture.admit();
    assert_eq!(muted, Some(true));
    state.pause_recording().unwrap();
    assert_eq!(capture.admit(), None, "nothing is admitted while paused");
    state.resume_recording().unwrap();

    // Unmuted by the time the queued block is processed: still silence.
    state.set_microphone_muted(false);
    capture.process_block(&vec![0.5; 1_024], muted.unwrap());
    let chunk = receiver.try_recv().expect("the block should reach the pipeline");
    assert!(chunk.data.iter().all(|sample| *sample == 0.0));
}

#[test]
fn muted_system_capture_sends_aligned_silence() {
    let state = RecordingState::new();
    state.start_recording().unwrap();
    state.set_system_audio_muted(true);
    let (sender, mut receiver) = mpsc::unbounded_channel();
    state.set_audio_sender(sender);
    let device = Arc::new(AudioDevice::new(
        "Test system output".to_string(),
        AudioDeviceType::Output,
    ));
    let capture = AudioCapture::new(device, state, 48_000, 1, DeviceType::System, None);

    capture.process_audio_data(&vec![0.5; 1_024]);

    let chunk = receiver
        .try_recv()
        .expect("muted system chunk should be retained");
    assert_eq!(chunk.data.len(), 1_024);
    assert!(chunk.data.iter().all(|sample| *sample == 0.0));
    assert_eq!(chunk.device_type, DeviceType::System);
}

#[test]
fn system_gain_is_applied_once_without_changing_alignment() {
    let mut samples = vec![0.0, 0.25, -0.25, 0.4];
    let original_len = samples.len();

    let limiter_hit = apply_system_gain(&mut samples, 2.0);

    assert!(!limiter_hit);
    assert_eq!(samples.len(), original_len);
    assert_eq!(samples, vec![0.0, 0.5, -0.5, 0.8]);
}

#[test]
fn system_gain_limits_positive_and_negative_clipping() {
    const PEAK_LIMIT: f32 = 0.891_250_9;
    let mut samples = vec![0.6, -0.3, 0.0];

    let limiter_hit = apply_system_gain(&mut samples, 2.0);

    assert!(limiter_hit);
    assert!((samples[0] - PEAK_LIMIT).abs() < f32::EPSILON);
    assert!((samples[1] + PEAK_LIMIT / 2.0).abs() < f32::EPSILON);
    assert_eq!(samples[2], 0.0);
}
