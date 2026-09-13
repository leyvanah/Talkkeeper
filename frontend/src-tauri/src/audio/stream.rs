use std::sync::Arc;
use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{Device, Stream, SupportedStreamConfig};
use log::{error, info, warn};
use tokio::sync::mpsc;

use super::devices::{AudioDevice, get_device_and_config};
use super::pipeline::AudioCapture;
use super::recording_state::{RecordingState, DeviceType};
use super::capture::{AudioCaptureBackend, get_current_backend};

#[cfg(target_os = "macos")]
use super::capture::CoreAudioCapture;
#[cfg(target_os = "macos")]
use super::recording_state::AudioError;

/// Stream backend implementation
pub enum StreamBackend {
    /// CPAL-based stream (ScreenCaptureKit or default)
    Cpal(Stream),
    /// Core Audio direct implementation (macOS only)
    #[cfg(target_os = "macos")]
    CoreAudio {
        task: Option<tokio::task::JoinHandle<()>>,
    },
    /// The microphone as a voice call opens it, with Windows cancelling the
    /// speakers' echo before we see the samples (Windows only).
    #[cfg(target_os = "windows")]
    WasapiComms {
        capture: Option<super::capture::CommsCapture>,
        task: Option<tokio::task::JoinHandle<()>>,
    },
}

// SAFETY: While Stream doesn't implement Send, we ensure it's only accessed
// from the same thread context by using spawn_blocking for operations that cross thread boundaries
unsafe impl Send for StreamBackend {}

/// Simplified audio stream wrapper with multi-backend support
pub struct AudioStream {
    device: Arc<AudioDevice>,
    backend: StreamBackend,
    /// A second, unrecorded microphone stream that only says when the owner is
    /// speaking. Held here so it lives exactly as long as the capture it
    /// belongs to. See `audio::own_speech`.
    #[cfg(target_os = "windows")]
    own_speech_detector: Option<super::capture::CommsCapture>,
}

// SAFETY: AudioStream contains StreamBackend which we've marked as Send
unsafe impl Send for AudioStream {}

impl AudioStream {
    /// Create a new audio stream for the given device
    pub async fn create(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<super::recording_state::AudioChunk>>,
    ) -> Result<Self> {
        // Get current backend from global config
        let backend_type = get_current_backend();
        Self::create_with_backend(device, state, device_type, recording_sender, backend_type).await
    }

    /// Create a new audio stream with explicit backend selection
    pub async fn create_with_backend(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<super::recording_state::AudioChunk>>,
        backend_type: AudioCaptureBackend,
    ) -> Result<Self> {
        info!("🎵 Stream: Creating audio stream for device: {} with backend: {:?}, device_type: {:?}",
              device.name, backend_type, device_type);

        // For system audio devices, use the selected backend
        // For microphone devices, always use CPAL
        #[cfg(target_os = "macos")]
        let use_core_audio = device_type == DeviceType::System
            && backend_type == AudioCaptureBackend::CoreAudio;

        #[cfg(not(target_os = "macos"))]
        let use_core_audio = false;

        #[cfg(target_os = "macos")]
        info!("🎵 Stream: use_core_audio = {}, device_type == System: {}, backend == CoreAudio: {}",
              use_core_audio,
              device_type == DeviceType::System,
              backend_type == AudioCaptureBackend::CoreAudio);

        #[cfg(not(target_os = "macos"))]
        info!("🎵 Stream: use_core_audio = {}, device_type == System: {}",
              use_core_audio,
              device_type == DeviceType::System);

        #[cfg(target_os = "macos")]
        if use_core_audio {
            info!("🎵 Stream: Using Core Audio backend (cidre) for system audio");
            return Self::create_core_audio_stream(device, state, device_type, recording_sender).await;
        }

        // Default path: use CPAL
        #[cfg(target_os = "macos")]
        let backend_name = if backend_type == AudioCaptureBackend::ScreenCaptureKit {
            "ScreenCaptureKit"
        } else {
            "CPAL (default)"
        };

        #[cfg(not(target_os = "macos"))]
        let backend_name = "CPAL";

        // The microphone, and only the microphone, can come from Windows
        // already cleaned of echo. The system channel must stay raw: it is
        // the other person's voice, and cancelling it would be cancelling
        // the recording.
        //
        // The own-speech detector below wins when both are asked for: it needs
        // the ordinary microphone to record, and this branch would hand back
        // the cleaned one instead.
        #[cfg(target_os = "windows")]
        if device_type == DeviceType::Microphone
            && super::recording_preferences::system_echo_cancellation()
            && !super::recording_preferences::own_speech_detector()
        {
            match Self::create_comms_stream(
                device.clone(),
                state.clone(),
                device_type.clone(),
                recording_sender.clone(),
            )
            .await
            {
                Ok(stream) => return Ok(stream),
                // Falling back rather than failing: an older Windows, a
                // driver without the mode, or a device in use elsewhere
                // should cost echo cancellation, not the recording.
                Err(error) => warn!(
                    "Communications capture unavailable, falling back to ordinary capture: {}",
                    error
                ),
            }
        }

        info!("🎵 Stream: Using CPAL backend ({}) for device: {}", backend_name, device.name);

        // The hybrid: record the ordinary microphone, where the owner's voice
        // is never suppressed, and open a second one in the communications
        // category to be read only for when he is the one speaking. Letting
        // Windows clean the microphone itself costs exactly this — his voice
        // goes down along with the echo whenever he talks over the other
        // person — and this is the way around it.
        #[cfg(target_os = "windows")]
        {
            let wants_detector = device_type == DeviceType::Microphone
                && super::recording_preferences::own_speech_detector();
            let mut stream =
                Self::create_cpal_stream(device, state, device_type, recording_sender).await?;
            if wants_detector {
                stream.own_speech_detector = Self::start_own_speech_detector();
            }
            return Ok(stream);
        }

        #[cfg(not(target_os = "windows"))]
        Self::create_cpal_stream(device, state, device_type, recording_sender).await
    }

    /// Open the microphone the way a voice call does (Windows only).
    ///
    /// Samples cross a channel on their way to the shared capture
    /// processor, for a plain reason: the rate Windows picks for this mode
    /// is only known once the stream is open, and the processor has to be
    /// built with it. One copy per 10ms packet is not worth designing
    /// around.
    #[cfg(target_os = "windows")]
    async fn create_comms_stream(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<super::recording_state::AudioChunk>>,
    ) -> Result<Self> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<f32>>();
        let comms = super::capture::CommsCapture::start(move |samples| {
            let _ = tx.send(samples.to_vec());
        })?;

        let sample_rate = comms.sample_rate();
        // Windows hands this mode over already mixed to one channel.
        let capture = AudioCapture::new(
            device.clone(),
            state.clone(),
            sample_rate,
            1,
            device_type,
            recording_sender,
        );

        let task = tokio::spawn(async move {
            while let Some(chunk) = rx.recv().await {
                capture.process_audio_data(&chunk);
            }
        });

        info!(
            "✅ Stream: microphone opened in communications mode at {} Hz for {}",
            sample_rate, device.name
        );

        Ok(Self {
            device,
            backend: StreamBackend::WasapiComms {
                capture: Some(comms),
                task: Some(task),
            },
            own_speech_detector: None,
        })
    }

    /// Open the second microphone stream that only says who is speaking.
    ///
    /// Nothing it delivers is recorded, transcribed or written down: it feeds
    /// the detector and nothing else. Failing to open costs the detector, not
    /// the recording — the microphone is already captured by then — and with
    /// the detector closed the pipeline keeps every microphone segment, which
    /// is the behaviour from before this existed.
    #[cfg(target_os = "windows")]
    fn start_own_speech_detector() -> Option<super::capture::CommsCapture> {
        let gate = super::own_speech::OwnSpeechGate::shared();
        let feed = gate.clone();

        match super::capture::CommsCapture::start_as(
            super::capture::CommsRole::Detector,
            move |samples| feed.push(samples),
        ) {
            Ok(capture) => {
                gate.opened(capture.sample_rate());
                info!(
                    "🎙️ Own-speech detector listening at {} Hz — the recorded microphone stays as the room sounds",
                    capture.sample_rate()
                );
                Some(capture)
            }
            Err(error) => {
                gate.closed();
                warn!(
                    "Own-speech detector unavailable, keeping every microphone segment: {}",
                    error
                );
                None
            }
        }
    }

    /// Create a CPAL-based stream (ScreenCaptureKit on macOS)
    async fn create_cpal_stream(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<super::recording_state::AudioChunk>>,
    ) -> Result<Self> {
        info!("Creating CPAL stream for device: {}", device.name);

        // Get the underlying cpal device and config
        let (cpal_device, config) = get_device_and_config(&device).await?;

        info!("Audio config - Sample rate: {}, Channels: {}, Format: {:?}",
              config.sample_rate().0, config.channels(), config.sample_format());

        // Create audio capture processor
        let capture = AudioCapture::new(
            device.clone(),
            state.clone(),
            config.sample_rate().0,
            config.channels(),
            device_type,
            recording_sender,
        );

        // Build the appropriate stream based on sample format
        let stream = Self::build_stream(&cpal_device, &config, capture.clone())?;

        // Start the stream
        stream.play()?;
        info!("CPAL stream started for device: {}", device.name);

        Ok(Self {
            device,
            backend: StreamBackend::Cpal(stream),
            #[cfg(target_os = "windows")]
            own_speech_detector: None,
        })
    }

    /// Create a Core Audio stream (macOS only)
    #[cfg(target_os = "macos")]
    async fn create_core_audio_stream(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<super::recording_state::AudioChunk>>,
    ) -> Result<Self> {
        info!("🔊 Stream: Creating Core Audio stream for device: {}", device.name);

        // Create Core Audio capture
        info!("🔊 Stream: Calling CoreAudioCapture::new()...");
        let capture_impl = CoreAudioCapture::new()
            .map_err(|e| {
                error!("❌ Stream: CoreAudioCapture::new() failed: {}", e);
                anyhow::anyhow!("Failed to create Core Audio capture: {}", e)
            })?;

        info!("✅ Stream: CoreAudioCapture created, calling stream()...");
        let core_stream = capture_impl.stream()
            .map_err(|e| {
                error!("❌ Stream: capture_impl.stream() failed: {}", e);
                anyhow::anyhow!("Failed to create Core Audio stream: {}", e)
            })?;

        let sample_rate = core_stream.sample_rate();
        info!("✅ Stream: Core Audio stream created with sample rate: {} Hz", sample_rate);

        // Create audio capture processor for pipeline integration
        // CRITICAL: Core Audio tap is MONO (with_mono_global_tap_excluding_processes)
        let state_for_stream = state.clone();
        let capture = AudioCapture::new(
            device.clone(),
            state.clone(),
            sample_rate,
            1, // Core Audio tap is MONO (not stereo!)
            device_type,
            recording_sender,
        );

        // Spawn task to process Core Audio stream samples
        // The stream needs to be polled continuously to produce samples
        let device_name = device.name.clone();
        info!("🔊 Stream: Spawning tokio task to poll Core Audio stream...");
        let task = tokio::spawn({
            let capture = capture.clone();
            let mut stream = core_stream;

            async move {
                use futures_util::StreamExt;

                let mut buffer = Vec::new();
                let mut frame_count = 0;
                let frames_per_chunk = 1024; // Process in chunks of 1024 samples
                let mut terminal_error_reported = false;

                info!("✅ Stream: Core Audio processing task started for {}", device_name);

                let mut _sample_count = 0u64;
                loop {
                    let sample = match tokio::time::timeout(
                        tokio::time::Duration::from_secs(5),
                        stream.next(),
                    )
                    .await
                    {
                        Ok(Some(sample)) => sample,
                        Ok(None) => break,
                        Err(_) => {
                            error!(
                                "Core Audio stopped delivering callbacks for {}",
                                device_name
                            );
                            state_for_stream.report_error(AudioError::ChannelClosed);
                            terminal_error_reported = true;
                            break;
                        }
                    };
                    let current_sample_rate = stream.sample_rate();
                    if current_sample_rate != sample_rate {
                        error!(
                            "Core Audio sample rate changed during recording: {} -> {} Hz",
                            sample_rate, current_sample_rate
                        );
                        state_for_stream.report_error(AudioError::SampleRateUnsupported);
                        terminal_error_reported = true;
                        break;
                    }
                    _sample_count += 1;
                    // if _sample_count % 48000 == 0 {
                    //     info!("📊 Stream: Received {} samples from Core Audio stream", _sample_count);
                    // }

                    buffer.push(sample);
                    frame_count += 1;

                    // Process when we have enough samples
                    if frame_count >= frames_per_chunk {
                        capture.process_audio_data(&buffer);
                        buffer.clear();
                        frame_count = 0;
                    }
                }

                // Process any remaining samples
                if !buffer.is_empty() {
                    capture.process_audio_data(&buffer);
                }

                if !terminal_error_reported && state_for_stream.is_recording() {
                    error!("Core Audio stream ended unexpectedly for {}", device_name);
                    state_for_stream.report_error(AudioError::ChannelClosed);
                }

                info!("⚠️ Stream: Core Audio processing task ended for {}", device_name);
            }
        });

        info!("✅ Stream: Core Audio stream fully initialized for device: {}", device.name);

        Ok(Self {
            device: device.clone(),
            backend: StreamBackend::CoreAudio {
                task: Some(task),
            },
        })
    }

    /// Build stream based on sample format
    fn build_stream(
        device: &Device,
        config: &SupportedStreamConfig,
        capture: AudioCapture,
    ) -> Result<Stream> {
        let config_copy = config.clone();

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => {
                let capture_clone = capture.clone();
                device.build_input_stream(
                    &config_copy.into(),
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        capture.process_audio_data(data);
                    },
                    move |err| {
                        capture_clone.handle_stream_error(err);
                    },
                    None,
                )?
            }
            cpal::SampleFormat::I16 => {
                let capture_clone = capture.clone();
                device.build_input_stream(
                    &config_copy.into(),
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter()
                            .map(|&sample| sample as f32 / i16::MAX as f32)
                            .collect();
                        capture.process_audio_data(&f32_data);
                    },
                    move |err| {
                        capture_clone.handle_stream_error(err);
                    },
                    None,
                )?
            }
            cpal::SampleFormat::I32 => {
                let capture_clone = capture.clone();
                device.build_input_stream(
                    &config_copy.into(),
                    move |data: &[i32], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter()
                            .map(|&sample| sample as f32 / i32::MAX as f32)
                            .collect();
                        capture.process_audio_data(&f32_data);
                    },
                    move |err| {
                        capture_clone.handle_stream_error(err);
                    },
                    None,
                )?
            }
            cpal::SampleFormat::I8 => {
                let capture_clone = capture.clone();
                device.build_input_stream(
                    &config_copy.into(),
                    move |data: &[i8], _: &cpal::InputCallbackInfo| {
                        let f32_data: Vec<f32> = data.iter()
                            .map(|&sample| sample as f32 / i8::MAX as f32)
                            .collect();
                        capture.process_audio_data(&f32_data);
                    },
                    move |err| {
                        capture_clone.handle_stream_error(err);
                    },
                    None,
                )?
            }
            _ => {
                return Err(anyhow::anyhow!("Unsupported sample format: {:?}", config.sample_format()));
            }
        };

        Ok(stream)
    }

    /// Get device info
    pub fn device(&self) -> &AudioDevice {
        &self.device
    }

    /// Stop the stream
    pub fn stop(self) -> Result<()> {
        info!("Stopping audio stream for device: {}", self.device.name);

        // The detector first, and always: the pipeline asks it whether the
        // owner was speaking, and an open detector with nothing arriving would
        // be answering about a recording that has ended.
        #[cfg(target_os = "windows")]
        if let Some(detector) = self.own_speech_detector {
            detector.stop();
            super::own_speech::OwnSpeechGate::shared().closed();
            info!("Own-speech detector stopped");
        }

        match self.backend {
            StreamBackend::Cpal(stream) => {
                // CRITICAL: Pause the stream first to stop callbacks immediately
                // This ensures closures stop executing before we drop the stream,
                // allowing Arc references captured in callbacks to be released
                if let Err(e) = stream.pause() {
                    warn!("Failed to pause stream before drop: {}", e);
                }
                info!("Stream paused, now dropping to release callbacks");
                drop(stream);
            }
            #[cfg(target_os = "windows")]
            StreamBackend::WasapiComms { capture, task } => {
                // The capture thread first: it owns the channel's sender,
                // so dropping it ends the forwarding task on its own rather
                // than by abort, and no packet is lost on the way out.
                if let Some(capture) = capture {
                    capture.stop();
                }
                if let Some(task) = task {
                    task.abort();
                }
                info!("Communications capture stopped");
            }
            #[cfg(target_os = "macos")]
            StreamBackend::CoreAudio { task } => {
                // Abort the processing task and wait briefly for cleanup
                if let Some(task_handle) = task {
                    info!("Aborting Core Audio task...");
                    task_handle.abort();
                    // Give the runtime a moment to clean up the aborted task
                    // This helps ensure Arc references in the closure are dropped
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    info!("Core Audio task aborted");
                }
            }
        }

        // Explicitly drop self.device Arc reference
        drop(self.device);
        info!("Audio stream stopped and device reference dropped");
        Ok(())
    }
}

/// Audio stream manager for handling multiple streams
pub struct AudioStreamManager {
    microphone_stream: Option<AudioStream>,
    system_stream: Option<AudioStream>,
    state: Arc<RecordingState>,
}

// SAFETY: AudioStreamManager contains AudioStream which we've marked as Send
unsafe impl Send for AudioStreamManager {}

impl AudioStreamManager {
    pub fn new(state: Arc<RecordingState>) -> Self {
        Self {
            microphone_stream: None,
            system_stream: None,
            state,
        }
    }

    /// Start audio streams for the given devices
    pub async fn start_streams(
        &mut self,
        microphone_device: Option<Arc<AudioDevice>>,
        system_device: Option<Arc<AudioDevice>>,
        recording_sender: Option<mpsc::UnboundedSender<super::recording_state::AudioChunk>>,
    ) -> Result<()> {
        use super::capture::get_current_backend;
        let backend = get_current_backend();

        // Whatever the last recording left behind: a detector that is not
        // being fed must not still read as open, or the pipeline would weigh
        // microphone segments against a timeline nobody is writing.
        super::own_speech::OwnSpeechGate::shared().closed();

        info!("🎙️ Starting audio streams with backend: {:?}", backend);

        // Start microphone stream
        if let Some(mic_device) = microphone_device {
            info!("🎤 Creating microphone stream: {} (always uses CPAL)", mic_device.name);
            match AudioStream::create(mic_device.clone(), self.state.clone(), DeviceType::Microphone, recording_sender.clone()).await {
                Ok(stream) => {
                    self.state.set_microphone_device(mic_device);
                    self.state.set_capture_active(DeviceType::Microphone, true);
                    self.microphone_stream = Some(stream);
                    info!("✅ Microphone stream created successfully");
                }
                Err(e) => {
                    error!("❌ Failed to create microphone stream: {}", e);
                    self.state.finish_capture_setup();
                    return Err(e);
                }
            }
        } else {
            info!("ℹ️ No microphone device specified, skipping microphone stream");
        }

        // Start system audio stream
        if let Some(sys_device) = system_device {
            info!("🔊 Creating system audio stream: {} (backend: {:?})", sys_device.name, backend);
            match AudioStream::create(sys_device.clone(), self.state.clone(), DeviceType::System, recording_sender.clone()).await {
                Ok(stream) => {
                    self.state.set_system_device(sys_device);
                    self.state.set_capture_active(DeviceType::System, true);
                    self.system_stream = Some(stream);
                    info!("✅ System audio stream created with {:?} backend", backend);
                }
                Err(e) => {
                    warn!("⚠️ Failed to create system audio stream: {}", e);
                    // Don't fail if only system audio fails
                }
            }
        } else {
            info!("ℹ️ No system device specified, skipping system audio stream");
        }

        // Ensure at least one stream was created
        if self.microphone_stream.is_none() && self.system_stream.is_none() {
            self.state.finish_capture_setup();
            return Err(anyhow::anyhow!("No audio streams could be created"));
        }

        self.state.finish_capture_setup();
        Ok(())
    }

    /// Stop all audio streams
    pub fn stop_streams(&mut self) -> Result<()> {
        info!("Stopping all audio streams");

        let mut errors = Vec::new();

        // Stop microphone stream
        if let Some(mic_stream) = self.microphone_stream.take() {
            if let Err(e) = mic_stream.stop() {
                error!("Failed to stop microphone stream: {}", e);
                errors.push(e);
            }
        }

        // Stop system stream
        if let Some(sys_stream) = self.system_stream.take() {
            if let Err(e) = sys_stream.stop() {
                error!("Failed to stop system stream: {}", e);
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            Err(anyhow::anyhow!("Failed to stop some streams: {:?}", errors))
        } else {
            info!("All audio streams stopped successfully");
            Ok(())
        }
    }

    /// Get stream count
    pub fn active_stream_count(&self) -> usize {
        let mut count = 0;
        if self.microphone_stream.is_some() {
            count += 1;
        }
        if self.system_stream.is_some() {
            count += 1;
        }
        count
    }

    /// Check if any streams are active
    pub fn has_active_streams(&self) -> bool {
        self.microphone_stream.is_some() || self.system_stream.is_some()
    }
}

impl Drop for AudioStreamManager {
    fn drop(&mut self) {
        if let Err(e) = self.stop_streams() {
            error!("Error stopping streams during drop: {}", e);
        }
    }
}
