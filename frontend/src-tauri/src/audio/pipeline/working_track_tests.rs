use super::*;
use crate::audio::decoder::decode_audio_file;
use crate::audio::device_detection::InputDeviceKind;
use crate::audio::working_track::find_working_track;

/// The whole recording chain, without a sound card: blocks in as a device
/// delivers them, and a finished working track for each source out.
#[tokio::test]
async fn a_recording_leaves_a_working_track_for_each_source() {
    let folder = tempfile::tempdir().expect("temp folder");
    let state = RecordingState::new();
    state.start_recording().expect("recording state");

    let (audio_sender, audio_receiver) = mpsc::unbounded_channel::<AudioChunk>();
    let (transcription_sender, _transcription_receiver) = mpsc::unbounded_channel();
    let sample_rate = 48_000u32;
    let mut pipeline = AudioPipeline::new(
        audio_receiver,
        transcription_sender,
        state.clone(),
        0,
        sample_rate,
        "Test microphone".to_string(),
        InputDeviceKind::Unknown,
        "Test system audio".to_string(),
        InputDeviceKind::Unknown,
    );
    pipeline.open_working_tracks(folder.path());
    let running = tokio::spawn(async move { pipeline.run().await });

    // Two seconds of speech-shaped tone from both sources, in the 10 ms
    // blocks a capture callback hands over.
    let block = sample_rate as usize / 100;
    let blocks = 200;
    let mut clock = 0.0f64;
    for index in 0..blocks {
        let samples: Vec<f32> = (0..block)
            .map(|i| {
                let t = (index * block + i) as f32 / sample_rate as f32;
                (t * 220.0 * std::f32::consts::TAU).sin() * 0.3
            })
            .collect();
        clock += block as f64 / sample_rate as f64;
        for device_type in [DeviceType::Microphone, DeviceType::System] {
            let _ = audio_sender.send(AudioChunk {
                data: samples.clone(),
                sample_rate,
                timestamp: clock,
                chunk_id: index as u64,
                device_type,
            });
        }
    }
    drop(audio_sender);
    running.await.expect("pipeline task").expect("pipeline run");

    for track in ["mic", "system"] {
        let path = working_track_path(folder.path(), track);
        let decoded = decode_audio_file(&path)
            .unwrap_or_else(|e| panic!("{track} working track unreadable: {e}"));
        assert_eq!(decoded.sample_rate, WORKING_SAMPLE_RATE);
        assert_eq!(decoded.channels, 1);
        assert!(
            (decoded.duration_seconds - 2.0).abs() < 0.05,
            "{track} working track is {:.3}s, not the two seconds recorded",
            decoded.duration_seconds
        );
    }
}

/// Without a meeting folder there is nothing to keep, and the recording
/// must not start writing derived audio somewhere of its own choosing.
#[tokio::test]
async fn a_recording_that_is_not_saved_writes_nothing() {
    let folder = tempfile::tempdir().expect("temp folder");
    let state = RecordingState::new();
    state.start_recording().expect("recording state");

    let (audio_sender, audio_receiver) = mpsc::unbounded_channel::<AudioChunk>();
    let (transcription_sender, _transcription_receiver) = mpsc::unbounded_channel();
    let sample_rate = 48_000u32;
    let mut pipeline = AudioPipeline::new(
        audio_receiver,
        transcription_sender,
        state.clone(),
        0,
        sample_rate,
        "Test microphone".to_string(),
        InputDeviceKind::Unknown,
        "Test system audio".to_string(),
        InputDeviceKind::Unknown,
    );
    let running = tokio::spawn(async move { pipeline.run().await });

    let block = sample_rate as usize / 100;
    for index in 0..50 {
        for device_type in [DeviceType::Microphone, DeviceType::System] {
            let _ = audio_sender.send(AudioChunk {
                data: vec![0.1; block],
                sample_rate,
                timestamp: (index + 1) as f64 * 0.01,
                chunk_id: index as u64,
                device_type,
            });
        }
    }
    drop(audio_sender);
    running.await.expect("pipeline task").expect("pipeline run");

    assert!(find_working_track(folder.path(), "mic").is_none());
    assert!(!folder.path().join(".work").exists());
}
