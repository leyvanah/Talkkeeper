//! The audio pipeline: turns two raw capture streams into (a) a recording and
//! (b) speech segments for transcription.
//!
//! ```text
//!   mic ─▶ 16 kHz ─▶ mic VAD ───▶ transcription_sender
//!      │        └───────────────▶ .work/mic.flac
//!      ├▶ mic.mp4
//!      └─┐
//!        ├▶ mixer ──────────────▶ audio.mp4
//!   sys ─┬┘
//!      ├▶ 16 kHz ─▶ system VAD ▶ transcription_sender
//!      │        └───────────────▶ .work/system.flac
//!      └▶ system.mp4
//! ```
//!
//! Mic and system transcription remain separate so source identity survives.
//! Mixing is only for user-facing playback. Muting either source replaces its
//! samples with silence before this pipeline; do not drop its chunks, because
//! retained mic/system tracks must stay aligned.
//!
//! ## Live level meters
//! Because mixing erases the distinction, per-source RMS/peak levels are
//! computed **before** mixing and emitted as `recording-audio-levels`
//! (throttled to ~25/sec per source). This is the only surviving pre-mix
//! signal, and it drives the mic/system meters in the recording UI. The webview
//! cannot capture system audio itself, so these meters must be Rust-driven.
//!
//! ## VAD
//! Only speech reaches the transcriber, which removes most of the silence a
//! meeting contains and correspondingly reduces transcription cost.
//!
//! ## The working track
//! VAD, recognition and diarization all run at 16 kHz, so each source is
//! converted once here and everything downstream is handed that stream. It is
//! also written to `.work/` (see [`super::working_track`]), which is what lets
//! a later pass over this recording start at the first word instead of
//! decoding and resampling the delivery tracks again.

use super::batch_processor::AudioMetricsBatcher;
use crate::batch_audio_metric;
use anyhow::Result;
use log::{debug, error, info, warn};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::audio_processing::{
    audio_to_mono, HighPassFilter, LoudnessNormalizer, NoiseSuppressionProcessor,
};
use super::devices::AudioDevice;
use super::recording_preferences;
use super::recording_state::{AudioChunk, AudioError, DeviceType, RecordingState};
use super::own_speech::measure_segment;
use super::vad::{ContinuousVadProcessor, SpeechSegment};
use super::working_track::{working_track_path, WorkingTrack, WORKING_SAMPLE_RATE};

mod mixer;
use mixer::*;

/// Per-source live audio level sample emitted to the frontend visualizer.
/// One of these is sent per incoming (single-source) audio chunk, throttled
/// to ~25 updates/sec per source so the meter animates smoothly without spam.
#[derive(Clone, serde::Serialize)]
pub struct AudioLevels {
    /// "mic" (your microphone) or "system" (other participants / computer audio)
    pub source: String,
    /// RMS energy of the chunk (0.0 – ~1.0)
    pub rms: f32,
    /// Peak absolute sample of the chunk (0.0 – ~1.0)
    pub peak: f32,
    /// True when amplified system audio exceeded sample range since the last event.
    pub limiter_hit: bool,
}

/// Simplified audio capture without broadcast channels
#[derive(Clone)]
pub struct AudioCapture {
    device: Arc<AudioDevice>,
    state: Arc<RecordingState>,
    sample_rate: u32,        // Original device sample rate
    channels: u16,
    chunk_counter: Arc<std::sync::atomic::AtomicU64>,
    device_type: DeviceType,
    recording_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
    needs_resampling: bool,  // Flag if resampling is required
    // CRITICAL FIX: Persistent resampler to preserve energy across chunks
    resampler: Arc<std::sync::Mutex<Option<SincFixedIn<f32>>>>,
    // Buffering for variable-size chunks → fixed-size resampler input
    resampler_input_buffer: Arc<std::sync::Mutex<Vec<f32>>>,
    resampler_chunk_size: usize,  // Fixed chunk size for resampler (512 samples)
    // Audio enhancement processors (microphone only)
    noise_suppressor: Arc<std::sync::Mutex<Option<NoiseSuppressionProcessor>>>,
    high_pass_filter: Arc<std::sync::Mutex<Option<HighPassFilter>>>,
    // EBU R128 normalizer for microphone audio (per-device, stateful)
    normalizer: Arc<std::sync::Mutex<Option<LoudnessNormalizer>>>,
    // Note: Using global recording timestamp for synchronization
}

impl AudioCapture {
    pub fn new(
        device: Arc<AudioDevice>,
        state: Arc<RecordingState>,
        sample_rate: u32,
        channels: u16,
        device_type: DeviceType,
        recording_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
    ) -> Self {
        // CRITICAL FIX: Detect if resampling is needed
        // Pipeline expects 48kHz, but Bluetooth devices often report 8kHz, 16kHz, or 44.1kHz
        const TARGET_SAMPLE_RATE: u32 = 48000;
        let needs_resampling = sample_rate != TARGET_SAMPLE_RATE;

        // Detect device kind (Bluetooth vs Wired) for adaptive processing
        // Use reasonable defaults for buffer size (512 samples is typical)
        let device_kind =
            super::device_detection::InputDeviceKind::detect(&device.name, 512, sample_rate);

        if needs_resampling {
            warn!("⚠️ SAMPLE RATE MISMATCH DETECTED ⚠️");
            warn!(
                "🔄 [{:?}] Audio device '{}' ({:?}) reports {} Hz (pipeline expects {} Hz)",
                device_type, device.name, device_kind, sample_rate, TARGET_SAMPLE_RATE
            );
            warn!(
                "🔄 Automatic resampling will be applied: {} Hz → {} Hz",
                sample_rate, TARGET_SAMPLE_RATE
            );

            // Log which resampling strategy will be used
            let ratio = TARGET_SAMPLE_RATE as f64 / sample_rate as f64;
            let strategy = if ratio >= 2.0 {
                "High-quality upsampling (sinc_len=512, Cubic interpolation)"
            } else if ratio >= 1.5 {
                "Moderate upsampling (sinc_len=384, Cubic)"
            } else if ratio > 1.0 {
                "Small upsampling (sinc_len=256, Linear)"
            } else if ratio <= 0.5 {
                "Anti-aliased downsampling (sinc_len=512, Cubic)"
            } else {
                "Moderate downsampling (sinc_len=384, Linear)"
            };
            info!("   Resampling strategy: {}", strategy);
        } else {
            info!(
                "✅ [{:?}] Audio device '{}' ({:?}) uses {} Hz (matches pipeline)",
                device_type, device.name, device_kind, sample_rate
            );
        }

        // Initialize audio enhancement processors for MICROPHONE ONLY
        // System audio doesn't need enhancement (already clean)
        let (noise_suppressor, high_pass_filter, normalizer) = if matches!(
            device_type,
            DeviceType::Microphone
        ) {
            // Initialize noise suppression (RNNoise) at 48kHz - CONDITIONAL based on flag
            let ns = if super::ffmpeg_mixer::RNNOISE_APPLY_ENABLED {
                match NoiseSuppressionProcessor::new(TARGET_SAMPLE_RATE) {
                    Ok(processor) => {
                        info!("✅ RNNoise noise suppression ENABLED for microphone '{}' (10-15 dB reduction)", device.name);
                        Some(processor)
                    }
                    Err(e) => {
                        warn!("⚠️ Failed to create noise suppressor: {}, continuing without noise suppression", e);
                        None
                    }
                }
            } else {
                info!("ℹ️ RNNoise noise suppression DISABLED for microphone '{}' (flag: RNNOISE_APPLY_ENABLED=false)", device.name);
                info!("   Whisper handles noise well internally - RNNoise is optional");
                None
            };

            // Initialize high-pass filter (removes rumble below 80 Hz)
            let hpf = {
                let filter = HighPassFilter::new(TARGET_SAMPLE_RATE, 80.0);
                info!(
                    "✅ High-pass filter initialized for microphone '{}' (cutoff: 80 Hz)",
                    device.name
                );
                Some(filter)
            };

            // Initialize EBU R128 normalizer (professional loudness standard)
            let norm = match LoudnessNormalizer::new(1, TARGET_SAMPLE_RATE) {
                Ok(normalizer) => {
                    info!(
                        "✅ EBU R128 normalizer initialized for microphone '{}' (target: -20 LUFS)",
                        device.name
                    );
                    Some(normalizer)
                }
                Err(e) => {
                    warn!(
                        "⚠️ Failed to create normalizer for microphone: {}, normalization disabled",
                        e
                    );
                    None
                }
            };

            (ns, hpf, norm)
        } else {
            // System audio: no enhancement needed
            info!(
                "ℹ️ System audio '{}' captured raw (no enhancement)",
                device.name
            );
            (None, None, None)
        };

        // CRITICAL FIX: Initialize persistent resampler to preserve energy across chunks
        // Creating a new resampler per chunk causes energy amplification and incorrect output sizes
        // Use fixed chunk size of 512 samples with buffering for variable-size input
        const RESAMPLER_CHUNK_SIZE: usize = 512;

        let resampler = if needs_resampling {
            let ratio = TARGET_SAMPLE_RATE as f64 / sample_rate as f64;

            // Adaptive parameters based on sample rate ratio (same logic as resample_audio)
            let (sinc_len, interpolation_type, oversampling) = if ratio >= 2.0 {
                (512, SincInterpolationType::Cubic, 512)
            } else if ratio >= 1.5 {
                (384, SincInterpolationType::Cubic, 384)
            } else if ratio > 1.0 {
                (256, SincInterpolationType::Linear, 256)
            } else if ratio <= 0.5 {
                (512, SincInterpolationType::Cubic, 512)
            } else {
                (384, SincInterpolationType::Linear, 384)
            };

            let params = SincInterpolationParameters {
                sinc_len,
                f_cutoff: 0.95,
                interpolation: interpolation_type,
                oversampling_factor: oversampling,
                window: WindowFunction::BlackmanHarris2,
            };

            match SincFixedIn::<f32>::new(
                ratio,
                2.0,  // Maximum relative deviation
                params,
                RESAMPLER_CHUNK_SIZE,
                1,    // Mono
            ) {
                Ok(resampler) => {
                    info!(
                        "✅ Persistent resampler initialized for '{}' ({}Hz → {}Hz, chunk_size={})",
                        device.name, sample_rate, TARGET_SAMPLE_RATE, RESAMPLER_CHUNK_SIZE
                    );
                    info!("   Buffering enabled for variable-size chunks (e.g., 320, 512, 1024, etc.)");
                    Some(resampler)
                }
                Err(e) => {
                    warn!(
                        "⚠️ Failed to create persistent resampler: {}, will use fallback",
                        e
                    );
                    None
                }
            }
        } else {
            None
        };

        Self {
            device,
            state,
            sample_rate,
            channels,
            chunk_counter: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            device_type,
            recording_sender,
            needs_resampling,
            resampler: Arc::new(std::sync::Mutex::new(resampler)),
            resampler_input_buffer: Arc::new(std::sync::Mutex::new(Vec::with_capacity(
                RESAMPLER_CHUNK_SIZE * 2,
            ))),
            resampler_chunk_size: RESAMPLER_CHUNK_SIZE,
            noise_suppressor: Arc::new(std::sync::Mutex::new(noise_suppressor)),
            high_pass_filter: Arc::new(std::sync::Mutex::new(high_pass_filter)),
            normalizer: Arc::new(std::sync::Mutex::new(normalizer)),
            // Using global recording time for sync
        }
    }

    /// Whether a block arriving now belongs in the recording, and whether its
    /// source was muted at that moment. `None` means drop it.
    ///
    /// Only atomic reads: this is what a real-time capture callback asks
    /// before handing the block to [`Self::process_block`] on another thread.
    pub fn admit(&self) -> Option<bool> {
        // Check if still recording
        if !self.state.is_recording() {
            return None;
        }
        // Pause stops the recording clock but not the capture streams, so
        // without this the room would keep being recorded while the owner
        // believes it is not, and the paused stretch would land in the file.
        if self.state.is_paused() {
            return None;
        }
        Some(self.state.is_audio_source_muted(&self.device_type))
    }

    /// Process a block where it arrives. For sources that already deliver on
    /// a thread of their own; a real-time callback should use [`Self::admit`]
    /// and hand the block over instead.
    pub fn process_audio_data(&self, data: &[f32]) {
        if let Some(muted) = self.admit() {
            self.process_block(data, muted);
        }
    }

    /// Everything after admission: down-mix, resample, the microphone's DSP,
    /// and the hand-off to the pipeline. Takes locks and allocates, so it must
    /// not run inside a real-time audio callback.
    pub fn process_block(&self, data: &[f32], source_muted_at_capture: bool) {

        // Convert to mono if needed
        let mut mono_data = if self.channels > 1 {
            audio_to_mono(data, self.channels)
        } else {
            data.to_vec()
        };
        if source_muted_at_capture {
            mono_data.fill(0.0);
        }

        // CRITICAL FIX: Resample to 48kHz if device uses different sample rate
        // This fixes Bluetooth devices (like Sony WH-1000XM4) that report 16kHz or 44.1kHz
        // Without this, audio is sped up 3x and VAD fails
        //
        // IMPORTANT: Uses PERSISTENT resampler with BUFFERING to preserve energy across chunks
        // Creating a new resampler per chunk causes energy amplification (173.5% RMS)
        // Buffering handles variable chunk sizes (320, 512, 1024, etc.) by accumulating to fixed 512-sample chunks
        const TARGET_SAMPLE_RATE: u32 = 48000;
        if self.needs_resampling {
            let before_len = mono_data.len();
            let before_rms = if !mono_data.is_empty() {
                (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt()
            } else {
                0.0
            };

            // Use persistent resampler with buffering to handle variable chunk sizes
            let mut resampled_output = Vec::new();
            let mut used_persistent_resampler = false;

            if let Ok(mut buffer_lock) = self.resampler_input_buffer.lock() {
                // Add new samples to buffer
                buffer_lock.extend_from_slice(&mono_data);

                // Process complete chunks through the resampler
                if let Ok(mut resampler_lock) = self.resampler.lock() {
                    if let Some(ref mut resampler) = *resampler_lock {
                        used_persistent_resampler = true;

                        // Process as many complete chunks as we have
                        while buffer_lock.len() >= self.resampler_chunk_size {
                            // Extract exactly chunk_size samples
                            let chunk: Vec<f32> =
                                buffer_lock.drain(0..self.resampler_chunk_size).collect();

                            // Rubato expects input as Vec<Vec<f32>> (one Vec per channel)
                            let waves_in = vec![chunk];

                            match resampler.process(&waves_in, None) {
                                Ok(mut waves_out) => {
                                    if let Some(output) = waves_out.pop() {
                                        resampled_output.extend_from_slice(&output);
                                    }
                                }
                                Err(e) => {
                                    warn!("⚠️ Persistent resampler processing failed: {}", e);
                                    used_persistent_resampler = false;
                                    break;
                                }
                            }
                        }
                        // Remaining samples in buffer will be processed in next iteration
                    }
                }
            }

            // CRITICAL: Only update mono_data if we got output from persistent resampler
            // If buffer is accumulating (< 512 samples), skip this chunk - data is safely buffered
            // and will be processed in next iteration with proper resampling
            let has_resampled_output = !resampled_output.is_empty();

            if has_resampled_output {
                mono_data = resampled_output;
            } else if !used_persistent_resampler {
                // Only fallback if persistent resampler is not available at all
                mono_data = super::audio_processing::resample_audio(
                    &mono_data,
                    self.sample_rate,
                    TARGET_SAMPLE_RATE,
                );
            } else {
                // Buffering: samples are accumulating in buffer, waiting for 512-sample chunk
                // Don't send partial/unprocessed data - return early
                // Audio is NOT lost - it's in the buffer and will be processed next iteration
                return;
            }

            // Log resampling only occasionally to avoid spam
            let chunk_id = self.chunk_counter.load(std::sync::atomic::Ordering::SeqCst);
            if chunk_id % 100 == 0 && has_resampled_output {
                let after_len = mono_data.len();
                let after_rms = if !mono_data.is_empty() {
                    (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt()
                } else {
                    0.0
                };
                let ratio = TARGET_SAMPLE_RATE as f64 / self.sample_rate as f64;
                let rms_preservation = if before_rms > 0.0 {
                    (after_rms / before_rms) * 100.0
                } else {
                    100.0
                };

                let buffer_size = if let Ok(buf) = self.resampler_input_buffer.lock() {
                    buf.len()
                } else {
                    0
                };

                info!(
                    "🔄 [{:?}] Persistent buffered resampler: {}Hz → {}Hz (ratio: {:.2}x)",
                    self.device_type, self.sample_rate, TARGET_SAMPLE_RATE, ratio
                );
                info!(
                    "   Chunk {}: {} → {} samples, RMS preservation: {:.1}%, buffer: {}",
                    chunk_id, before_len, after_len, rms_preservation, buffer_size
                );
            }
        }

        // AUDIO ENHANCEMENT PIPELINE (Microphone Only)
        // Processing order is critical: high-pass → noise suppression → normalization
        // This ensures noise is removed before being amplified by the normalizer
        if matches!(self.device_type, DeviceType::Microphone) {
            // STEP 1: Apply high-pass filter to remove low-frequency rumble (< 80 Hz)
            if let Ok(mut hpf_lock) = self.high_pass_filter.lock() {
                if let Some(ref mut filter) = *hpf_lock {
                    mono_data = filter.process(&mono_data);
                }
            }

            // STEP 2: Apply RNNoise noise suppression (10-15 dB reduction) - CONDITIONAL
            if super::ffmpeg_mixer::RNNOISE_APPLY_ENABLED {
                if let Ok(mut ns_lock) = self.noise_suppressor.lock() {
                    if let Some(ref mut suppressor) = *ns_lock {
                        let before_len = mono_data.len();
                        mono_data = suppressor.process(&mono_data);
                        let after_len = mono_data.len();

                        // CRITICAL MONITORING: Track buffer health
                        let chunk_id = self.chunk_counter.load(std::sync::atomic::Ordering::SeqCst);
                        if chunk_id % 100 == 0 {
                            let buffered = suppressor.buffered_samples();
                            let length_delta = (before_len as i32 - after_len as i32).abs();

                            debug!("🔇 Noise suppression health: in={}, out={}, delta={}, buffered={}, RMS={:.4}",
                                   before_len, after_len, length_delta, buffered,
                                   if !mono_data.is_empty() {
                                       (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt()
                                   } else { 0.0 });

                            // WARN if accumulating samples (potential latency buildup)
                            if buffered > 1000 {
                                warn!("⚠️ RNNoise accumulating samples: {} buffered (potential latency issue!)",
                                      buffered);
                            }

                            // WARN if significant length mismatch
                            if length_delta > 50 {
                                warn!(
                                    "⚠️ RNNoise length mismatch: input={} output={} (delta={})",
                                    before_len, after_len, length_delta
                                );
                            }
                        }
                    }
                }
            }

            // STEP 3: Apply EBU R128 normalization (professional loudness standard)
            if let Ok(mut normalizer_lock) = self.normalizer.lock() {
                if let Some(ref mut normalizer) = *normalizer_lock {
                    let mic_gain = super::recording_preferences::mic_gain();
                    mono_data = normalizer.normalize_loudness(&mono_data, mic_gain);

                    // Log normalization occasionally for debugging
                    let chunk_id = self.chunk_counter.load(std::sync::atomic::Ordering::SeqCst);
                    if chunk_id % 200 == 0 && !mono_data.is_empty() {
                        let rms = (mono_data.iter().map(|&x| x * x).sum::<f32>()
                            / mono_data.len() as f32)
                            .sqrt();
                        let peak = mono_data.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
                        debug!(
                            "🎤 After normalization chunk {}: RMS={:.4}, Peak={:.4}",
                            chunk_id, rms, peak
                        );
                    }
                }
            }

            // User gain is included before the normalizer's final limiter so it
            // cannot reintroduce hard clipping afterward.
        }

        // Check again after stateful DSP so a mute command that arrives while a
        // callback is being processed cannot leak its tail into VAD or storage.
        // Keeping the zero-filled chunk preserves mic/system track alignment.
        if source_muted_at_capture || self.state.is_audio_source_muted(&self.device_type) {
            mono_data.fill(0.0);
        }

        // Create audio chunk with stream-specific timestamp (get ID first for logging)
        let chunk_id = self
            .chunk_counter
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        // RAW AUDIO: No gain applied here - will be applied AFTER mixing
        // This prevents amplifying system audio bleed-through in the microphone

        // DIAGNOSTIC: Log audio levels for debugging (especially mic issues)
        // if chunk_id % 100 == 0 && !mono_data.is_empty() {
        //     let raw_rms = (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt();
        //     let raw_peak = mono_data.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);

        //         info!("🎙️ [{:?}] Chunk {} - Raw: RMS={:.6}, Peak={:.6}",
        //               self.device_type, chunk_id, raw_rms, raw_peak);

        //     // Warn if microphone is completely silent
        //     if matches!(self.device_type, DeviceType::Microphone) && raw_rms == 0.0 && raw_peak == 0.0 {
        //         warn!("⚠️ Microphone producing ZERO audio - check permissions or hardware!");
        //     }
        // }
        // else if chunk_id % 100 == 0 && matches!(self.device_type, DeviceType::System) {
        //     let raw_rms = (mono_data.iter().map(|&x| x * x).sum::<f32>() / mono_data.len() as f32).sqrt();
        //     let raw_peak = mono_data.iter().map(|&x| x.abs()).fold(0.0f32, f32::max);
        //     info!("🔊 [{:?}] Chunk {} - Raw: RMS={:.6}, Peak={:.6}",
        //       self.device_type, chunk_id, raw_rms, raw_peak);
            
        //     // Warn if system audio is completely silent
        //     if raw_rms == 0.0 && raw_peak == 0.0 {
        //         warn!("⚠️ System audio producing ZERO audio - check permissions or hardware!");
        //     }
        // }

        // Use global recording timestamp for proper synchronization
        let timestamp = self.state.get_active_recording_duration().unwrap_or(0.0);

        if self.state.is_audio_source_muted(&self.device_type) {
            mono_data.fill(0.0);
        }

        // RAW AUDIO CHUNK: No gain applied - will be mixed and gained downstream
        // Use 48kHz if we resampled, otherwise use original rate
        let audio_chunk = AudioChunk {
            data: mono_data,  // Raw audio (resampled if needed), no gain yet
            sample_rate: if self.needs_resampling {
                48000
            } else {
                self.sample_rate
            },
            timestamp,
            chunk_id,
            device_type: self.device_type.clone(),
        };

        // NOTE: Raw audio is NOT sent to recording saver to prevent echo
        // Only the mixed audio (from AudioPipeline) is saved to file (see pipeline.rs:726-736)
        // This ensures we only record once: mic + system properly mixed
        // Individual raw streams go only to the transcription pipeline below

        // Send to processing pipeline for transcription
        if let Err(e) = self.state.send_audio_chunk(audio_chunk) {
            // Check if this is the "pipeline not ready" error
            if e.to_string().contains("Audio pipeline not ready") {
                // This is expected during initialization, just log it as debug
                debug!("Audio pipeline not ready yet, skipping chunk {}", chunk_id);
                return;
            }

            warn!("Failed to send audio chunk: {}", e);
            // More specific error handling based on failure reason
            let error = if e.to_string().contains("channel closed") {
                AudioError::ChannelClosed
            } else if e.to_string().contains("full") {
                AudioError::BufferOverflow
            } else {
                AudioError::ProcessingFailed
            };
            self.state.report_error(error);
        } else {
            debug!("Sent audio chunk {} ({} samples)", chunk_id, data.len());
        }
    }

    /// Handle stream errors with enhanced disconnect detection
    pub fn handle_stream_error(&self, error: cpal::StreamError) {
        error!("Audio stream error for {}: {}", self.device.name, error);

        let error_str = error.to_string().to_lowercase();

        // Enhanced error detection for device disconnection
        let audio_error = if error_str.contains("device is no longer available")
            || error_str.contains("device not found")
            || error_str.contains("device disconnected")
            || error_str.contains("no such device")
            || error_str.contains("device unavailable")
            || error_str.contains("device removed")
        {
            warn!("🔌 Device disconnect detected for: {}", self.device.name);
            AudioError::DeviceDisconnected
        } else if error_str.contains("permission") || error_str.contains("access denied") {
            AudioError::PermissionDenied
        } else if error_str.contains("channel closed") {
            AudioError::ChannelClosed
        } else if error_str.contains("stream") && error_str.contains("failed") {
            AudioError::StreamFailed
        } else {
            warn!("Unknown audio error: {}", error);
            AudioError::StreamFailed
        };

        self.state.report_error(audio_error);
    }
}

/// Whether the speakers were playing during this window.
///
/// Plain loudness rather than voice detection on purpose: an echo can come
/// back from anything the machine played, and the only thing this answers is
/// whether there was anything at all for the microphone to have heard. System
/// audio arrives as digital loopback, where silence is an exact zero, so the
/// floor only has to sit above the noise a codec leaves behind.
fn far_end_is_playing(sys_window: &[f32]) -> bool {
    if sys_window.is_empty() {
        return false;
    }
    let sum_squares: f64 = sys_window.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    (sum_squares / sys_window.len() as f64).sqrt() >= 0.005
}

/// Take the speakers' echo out of one microphone window — unless Windows has
/// already done it.
///
/// Separated from the pipeline so the choice can be tested on its own: the
/// defect this guards against is not in the arithmetic but in *when* the
/// question is asked. `system_is_cancelling` is a live fact about the capture
/// stream, and the stream opens after the pipeline is built.
fn cancel_echo_window(
    canceller: Option<&mut super::echo_cancel::EchoCanceller>,
    left_to_windows: &mut Option<bool>,
    system_is_cancelling: bool,
    mic_window: Vec<f32>,
    sys_window: &[f32],
) -> Vec<f32> {
    // Said once, when the answer changes, rather than twenty times a second.
    if *left_to_windows != Some(system_is_cancelling) {
        *left_to_windows = Some(system_is_cancelling);
        if system_is_cancelling {
            info!("🔇 Echo cancellation left to Windows; the built-in canceller stays out of the way");
        } else if canceller.is_some() {
            info!("🔇 Cancelling the speakers' echo out of the microphone ourselves");
        }
    }

    match canceller {
        Some(canceller) if !system_is_cancelling => canceller.process(&mic_window, sys_window),
        _ => mic_window,
    }
}

/// VAD-driven audio processing pipeline
/// Uses Voice Activity Detection to segment speech in real-time and send only speech to Whisper
pub struct AudioPipeline {
    receiver: mpsc::UnboundedReceiver<AudioChunk>,
    transcription_sender: mpsc::UnboundedSender<AudioChunk>,
    state: Arc<RecordingState>,
    /// Separate VAD per capture source. Mic and system are transcribed on
    /// independent paths (same wall-clock windows, different sample streams)
    /// so simultaneous talk never soft-limits one source into the other for STT.
    mic_vad: ContinuousVadProcessor,
    system_vad: ContinuousVadProcessor,
    /// Each source at the rate everything downstream works in. Also written to
    /// `.work/` once the manager, which is where the meeting folder is known,
    /// has said the recording is being kept.
    mic_work: WorkingTrack,
    system_work: WorkingTrack,
    sample_rate: u32,
    chunk_id_counter: u64,
    // Performance optimization: reduce logging frequency
    last_summary_time: std::time::Instant,
    processed_chunks: u64,
    // Smart batching for audio metrics
    metrics_batcher: Option<AudioMetricsBatcher>,
    // PROFESSIONAL AUDIO MIXING: Ring buffer + RMS-based mixer
    ring_buffer: AudioMixerRingBuffer,
    mixer: ProfessionalAudioMixer,
    /// Removes the speakers' echo from the mic window before VAD, transcription
    /// and the saved tracks. None when only one source is recording, when the
    /// owner turned it off, or when the canceller could not start.
    echo_canceller: Option<super::echo_cancel::EchoCanceller>,
    /// Who cancelled the echo when the last window went through, so a change
    /// of hands is said once instead of every window. None until the first.
    echo_left_to_windows: Option<bool>,
    /// The second microphone stream, read only for whether the owner is the
    /// one speaking. Closed unless he asked for it; see `own_speech`.
    own_speech: super::own_speech::OwnSpeechGate,
    /// Both detectors, window by window, for the passes that come after this
    /// recording; see `own_speech_record`.
    own_speech_record: super::own_speech_record::GateRecorder,
    /// What that detector said about each window, along the recording's clock.
    own_speech_timeline: super::own_speech::WindowTimeline,
    /// Whether the speakers were playing in each window. Without this the
    /// detector would be free to throw away speech recorded in a silent room,
    /// where there was never an echo to mistake it for.
    far_end_timeline: super::own_speech::WindowTimeline,
    /// Microphone segments dropped as nothing but the speakers coming back.
    echo_segments_dropped: u64,
    // Recording sender for pre-mixed audio
    recording_sender_for_mixed: Option<mpsc::UnboundedSender<AudioChunk>>,
    // Live per-source level meter output (mic + system) for the frontend visualizer
    level_sender: Option<mpsc::UnboundedSender<AudioLevels>>,
    last_mic_level_emit: std::time::Instant,
    last_sys_level_emit: std::time::Instant,
    system_limiter_hit_since_emit: bool,
    last_mic_input: std::time::Instant,
    last_system_input: std::time::Instant,
}

impl AudioPipeline {
    pub fn new(
        receiver: mpsc::UnboundedReceiver<AudioChunk>,
        transcription_sender: mpsc::UnboundedSender<AudioChunk>,
        state: Arc<RecordingState>,
        target_chunk_duration_ms: u32,
        sample_rate: u32,
        mic_device_name: String,
        mic_device_kind: super::device_detection::InputDeviceKind,
        system_device_name: String,
        system_device_kind: super::device_detection::InputDeviceKind,
    ) -> Self {
        // Log device characteristics for adaptive buffering
        info!("🎛️ AudioPipeline initializing with device characteristics:");
        info!(
            "   Mic: '{}' ({:?}) - Buffer: {:?}",
            mic_device_name,
            mic_device_kind,
            mic_device_kind.buffer_timeout()
        );
        info!(
            "   System: '{}' ({:?}) - Buffer: {:?}",
            system_device_name,
            system_device_kind,
            system_device_kind.buffer_timeout()
        );

        // Device kind information can be used for adaptive buffering in the future
        // For now, we log it for monitoring and potential optimization
        let mic_enabled = mic_device_name != "No Microphone";
        let system_enabled = system_device_name != "No System Audio";
        let _ = (mic_device_kind, system_device_kind);

        // Bridge short natural pauses without adding the two-second latency used by
        // offline retranscription.
        let redemption_time = 800;

        // One VAD per capture source so simultaneous talk is segmented independently.
        // Both are fed the working track, already converted, rather than the
        // capture stream: one conversion, of known quality, shared by VAD, the
        // recognizer and every later pass over this recording.
        let make_vad = |label: &str, positive_threshold, negative_threshold| {
            match ContinuousVadProcessor::new_with_thresholds(
            WORKING_SAMPLE_RATE,
            redemption_time,
            positive_threshold,
            negative_threshold,
        ) {
            Ok(processor) => {
                info!("VAD ready for {label}: segments go straight to Whisper (no shared mix)");
                processor
            }
            Err(e) => {
                error!("Failed to create {label} VAD processor: {e}");
                panic!("VAD processor creation failed: {e}");
            }
            }
        };
        // Headset/array microphones are usually quieter than digital loopback.
        let mic_vad = make_vad("microphone", 0.20, 0.10);
        let system_vad = make_vad("system", 0.50, 0.35);

        // Conversion to the working rate, one persistent resampler per source,
        // fed exactly the windows the ring buffer hands out. Nothing is written
        // to disk until the manager supplies a meeting folder.
        let make_working_track = |label: &'static str| {
            match WorkingTrack::new(label, sample_rate, mixing_window_samples(sample_rate), None) {
                Ok(track) => track,
                Err(e) => {
                    error!("Failed to create the {label} working track: {e}");
                    panic!("Working track creation failed: {e}");
                }
            }
        };
        let mic_work = make_working_track("microphone");
        let system_work = make_working_track("system");

        // Initialize professional audio mixing components (recording file only)
        let ring_buffer = AudioMixerRingBuffer::new(sample_rate, mic_enabled, system_enabled);
        let mixer = ProfessionalAudioMixer::new(sample_rate);

        // Both detector timelines are laid out in the same windows the mixer
        // hands over, so a slot index is a timestamp.
        let window_ms = MIXING_WINDOW_MS as f64;

        // Echo can only exist when the speakers and the microphone are both
        // live. Whether *we* are the ones to cancel it is a second question,
        // and it cannot be answered here: the pipeline is built before the
        // capture streams open, so asking whether Windows is cancelling gets
        // the answer from before this recording started - which is always
        // "no". Build the canceller whenever the owner asked for one, and ask
        // the live question once per window instead. See `cancel_echo`.
        let echo_canceller = if mic_enabled
            && system_enabled
            && super::recording_preferences::echo_cancellation()
        {
            super::echo_cancel::EchoCanceller::new(sample_rate)
        } else {
            None
        };

        // Note: target_chunk_duration_ms is ignored - VAD controls segmentation now
        let _ = target_chunk_duration_ms;

        Self {
            receiver,
            transcription_sender,
            state,
            mic_vad,
            system_vad,
            mic_work,
            system_work,
            sample_rate,
            chunk_id_counter: 0,
            // Performance optimization: reduce logging frequency
            last_summary_time: std::time::Instant::now(),
            processed_chunks: 0,
            // Initialize metrics batcher for smart batching
            metrics_batcher: Some(AudioMetricsBatcher::new()),
            // Initialize professional audio mixing
            ring_buffer,
            mixer,
            echo_canceller,
            echo_left_to_windows: None,
            own_speech: super::own_speech::OwnSpeechGate::shared(),
            own_speech_record: super::own_speech_record::GateRecorder::new(MIXING_WINDOW_MS),
            own_speech_timeline: super::own_speech::WindowTimeline::new(window_ms),
            far_end_timeline: super::own_speech::WindowTimeline::new(window_ms),
            echo_segments_dropped: 0,
            recording_sender_for_mixed: None,  // Will be set by manager
            // Live level meter (set by manager); default to no output
            level_sender: None,
            last_mic_level_emit: std::time::Instant::now(),
            last_sys_level_emit: std::time::Instant::now(),
            system_limiter_hit_since_emit: false,
            last_mic_input: std::time::Instant::now(),
            last_system_input: std::time::Instant::now(),
        }
    }

    /// Point the working tracks at a meeting folder, which is only known once
    /// the recording is being saved. Without this the conversion still runs —
    /// VAD needs it — but nothing is written down.
    fn open_working_tracks(&mut self, meeting_folder: &std::path::Path) {
        // Same folder, same moment, same publish-by-rename: what the detector
        // answered is only useful next to the track it answered about.
        self.own_speech_record.open_in(meeting_folder);
        let chunk = mixing_window_samples(self.sample_rate);
        for (label, name) in [("microphone", "mic"), ("system", "system")] {
            let path = working_track_path(meeting_folder, name);
            match WorkingTrack::new(label, self.sample_rate, chunk, Some(path)) {
                Ok(track) => {
                    if name == "mic" {
                        self.mic_work = track;
                    } else {
                        self.system_work = track;
                    }
                }
                // The recording itself is unaffected; later passes fall back
                // to decoding the delivery track.
                Err(e) => warn!("Could not open the {label} working track: {e}"),
            }
        }
    }

    fn finalize_inactive_speech(&mut self, now: std::time::Instant) {
        if self.state.is_paused() {
            return;
        }
        let redemption = std::time::Duration::from_millis(800);
        let mut completed = Vec::new();
        if now.duration_since(self.last_mic_input) >= redemption {
            if let Some(segment) = self.mic_vad.finalize_active_speech() {
                completed.push((DeviceType::Microphone, segment));
            }
        }
        if now.duration_since(self.last_system_input) >= redemption {
            if let Some(segment) = self.system_vad.finalize_active_speech() {
                completed.push((DeviceType::System, segment));
            }
        }
        for (device_type, segment) in completed {
            Self::enqueue_source_speech(
                vec![segment],
                device_type,
                &self.transcription_sender,
                &mut self.chunk_id_counter,
            );
        }
    }

    /// Run VAD on one source and enqueue any finished speech segments for Whisper.
    /// `device_type` is the true origin (mic vs system) — not a post-mix guess.
    fn emit_source_speech(
        vad: &mut ContinuousVadProcessor,
        samples: &[f32],
        device_type: DeviceType,
        transcription_sender: &mpsc::UnboundedSender<AudioChunk>,
        chunk_id_counter: &mut u64,
    ) {
        match vad.process_audio(samples) {
            Ok(speech_segments) => Self::enqueue_source_speech(
                speech_segments,
                device_type,
                transcription_sender,
                chunk_id_counter,
            ),
            Err(e) => warn!("⚠️ {:?} VAD error: {}", device_type, e),
        }
    }

    fn enqueue_source_speech(
        speech_segments: Vec<SpeechSegment>,
        device_type: DeviceType,
        transcription_sender: &mpsc::UnboundedSender<AudioChunk>,
        chunk_id_counter: &mut u64,
    ) {
        for segment in speech_segments {
            let duration_ms = segment.end_timestamp_ms - segment.start_timestamp_ms;
            if segment.samples.len() < 800 {
                debug!(
                    "⏭️ Dropping short {:?} VAD segment: {:.1}ms ({} samples < 800)",
                    device_type,
                    duration_ms,
                    segment.samples.len()
                );
                continue;
            }
            info!(
                "📤 Sending {:?} VAD segment: {:.1}ms, {} samples @ {:.2}s",
                device_type,
                duration_ms,
                segment.samples.len(),
                segment.start_timestamp_ms / 1000.0
            );
            let transcription_chunk = AudioChunk {
                data: segment.samples,
                sample_rate: 16000,
                timestamp: segment.start_timestamp_ms / 1000.0,
                chunk_id: *chunk_id_counter,
                device_type: device_type.clone(),
            };
            if let Err(e) = transcription_sender.send(transcription_chunk) {
                warn!("Failed to send {:?} VAD segment: {}", device_type, e);
            } else {
                *chunk_id_counter += 1;
            }
        }
    }

    /// Run the VAD-driven audio processing pipeline
    pub async fn run(mut self) -> Result<()> {
        info!("VAD-driven audio pipeline started - segments sent in real-time based on speech detection");

        // CRITICAL FIX: Continue processing until channel is closed, not based on recording state
        // This ensures ALL chunks are processed during shutdown, fixing premature meeting completion
        // Previous bug: Loop checked `while self.state.is_recording()` which caused early exit when
        // stop_recording() was called, losing flush signals and remaining chunks in the pipeline
        loop {
            // Receive audio chunks with timeout
            match tokio::time::timeout(
                std::time::Duration::from_millis(50), // Shorter timeout for responsiveness
                self.receiver.recv(),
            )
            .await
            {
                Ok(Some(mut chunk)) => {
                    let now = std::time::Instant::now();
                    self.finalize_inactive_speech(now);
                    match chunk.device_type {
                        DeviceType::Microphone => self.last_mic_input = now,
                        DeviceType::System => self.last_system_input = now,
                        DeviceType::Mixed => {}
                    }
                    // PERFORMANCE: Check for flush signal (special chunk with ID >= u64::MAX - 10)
                    // Multiple flush signals may be sent to ensure processing
                    if chunk.chunk_id >= u64::MAX - 10 {
                        info!(
                            "📥 Received FLUSH signal #{} - flushing VAD processor",
                            u64::MAX - chunk.chunk_id
                        );
                        self.flush_remaining_audio()?;
                        // Continue processing to handle any remaining chunks
                        continue;
                    }

                    // PERFORMANCE OPTIMIZATION: Eliminate per-chunk logging overhead
                    // Logging in hot paths causes severe performance degradation
                    self.processed_chunks += 1;

                    // Apply system gain once before every consumer: telemetry,
                    // VAD/transcription, retained system.mp4, and mixed audio.mp4.
                    if matches!(chunk.device_type, DeviceType::System) {
                        self.system_limiter_hit_since_emit |= apply_system_gain(
                            &mut chunk.data,
                            recording_preferences::system_gain(),
                        );
                    }

                    // Smart batching: collect metrics instead of logging every chunk
                    if let Some(ref batcher) = self.metrics_batcher {
                        let avg_level = chunk.data.iter().map(|&x| x.abs()).sum::<f32>()
                            / chunk.data.len() as f32;
                        let duration_ms =
                            chunk.data.len() as f64 / chunk.sample_rate as f64 * 1000.0;

                        batch_audio_metric!(
                            Some(batcher),
                            chunk.chunk_id,
                            chunk.data.len(),
                            duration_ms,
                            avg_level
                        );
                    }

                    // CRITICAL: Log summary only every 200 chunks OR every 60 seconds (99.5% reduction)
                    // This eliminates I/O overhead in the audio processing hot path
                    // Use performance-optimized debug macro that compiles to nothing in release builds
                    if self.processed_chunks % 200 == 0
                        || self.last_summary_time.elapsed().as_secs() >= 60
                    {
                        perf_debug!(
                            "Pipeline processed {} chunks, current chunk: {} ({} samples)",
                            self.processed_chunks,
                            chunk.chunk_id,
                            chunk.data.len()
                        );
                        self.last_summary_time = std::time::Instant::now();
                    }

                    // LIVE METER: emit per-source RMS/peak for the frontend visualizer.
                    // Each incoming chunk is single-source (mic OR system); throttle to
                    // ~25 updates/sec per source so the meter stays smooth and cheap.
                    if let Some(ref level_tx) = self.level_sender {
                        let is_mic = matches!(chunk.device_type, DeviceType::Microphone);
                        let now = std::time::Instant::now();
                        let last = if is_mic {
                            &mut self.last_mic_level_emit
                        } else {
                            &mut self.last_sys_level_emit
                        };
                        if !chunk.data.is_empty() && now.duration_since(*last).as_millis() >= 40 {
                            *last = now;
                            let n = chunk.data.len() as f32;
                            let rms = (chunk.data.iter().map(|&x| x * x).sum::<f32>() / n).sqrt();
                            let peak = chunk.data.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
                            let limiter_hit = if is_mic {
                                false
                            } else {
                                let hit = self.system_limiter_hit_since_emit;
                                self.system_limiter_hit_since_emit = false;
                                hit
                            };
                            let _ = level_tx.send(AudioLevels {
                                source: if is_mic { "mic" } else { "system" }.to_string(),
                                rms,
                                peak,
                                limiter_hit,
                            });
                        }
                    }

                    // STEP 1: Add source audio to ring buffer for mixing
                    // Microphone audio is already normalized at capture level (AudioCapture)
                    // System audio has user gain and chunk peak limiting applied above.
                    if let Some((mic_active, system_active)) = self.state.active_capture_sources() {
                        self.ring_buffer.set_enabled(mic_active, system_active);
                    }
                    let discontinuity_start = self.ring_buffer.add_samples(
                        chunk.device_type.clone(),
                        chunk.data,
                        chunk.timestamp,
                    );
                    if let Some(start_seconds) = discontinuity_start {
                        let mut completed = Vec::new();
                        if let Some(segment) = self.mic_vad.finalize_active_speech() {
                            completed.push((DeviceType::Microphone, segment));
                        }
                        if let Some(segment) = self.system_vad.finalize_active_speech() {
                            completed.push((DeviceType::System, segment));
                        }
                        for (device_type, segment) in completed {
                            let segments = if matches!(device_type, DeviceType::Microphone) {
                                self.without_the_speakers(vec![segment])
                            } else {
                                vec![segment]
                            };
                            Self::enqueue_source_speech(
                                segments,
                                device_type,
                                &self.transcription_sender,
                                &mut self.chunk_id_counter,
                            );
                        }
                        self.mic_vad.advance_inactive_timeline_to(start_seconds);
                        self.system_vad.advance_inactive_timeline_to(start_seconds);
                        // The recording clock jumped over a break. Leave the
                        // skipped span unanswered rather than let it read as
                        // silence from either the owner or the speakers.
                        let skipped_to_ms = start_seconds * 1000.0;
                        self.own_speech_timeline.skip_to(skipped_to_ms);
                        self.far_end_timeline.skip_to(skipped_to_ms);
                    }

                    // STEP 2: Mix audio in fixed windows when both streams have sufficient data
                    while self.ring_buffer.can_mix() {
                        if let Some((mic_window, sys_window)) = self.ring_buffer.extract_window() {
                            // Strip the speakers' echo before anything downstream
                            // sees the microphone, so neither the live transcript
                            // nor a later retranscription of mic.mp4 repeats what
                            // the remote person said.
                            self.observe_window(&sys_window);
                            let mic_window = self.cancel_echo(mic_window, &sys_window);
                            // STEP 3: Convert each source to the working rate,
                            // once, and store it. Everything that reads this
                            // recording later — VAD now, recognition and
                            // diarization afterwards — listens to this stream.
                            let mic_16k = self.mic_work.push(&mic_window);
                            let sys_16k = self.system_work.push(&sys_window);

                            // STEP 4: Transcribe each source independently.
                            // Same wall-clock windows (aligned by the ring buffer),
                            // separate sample streams + VAD state — so when both
                            // sides talk at once neither is soft-limited into the
                            // other before Whisper, and device_type is exact.
                            self.emit_microphone_speech(&mic_16k);
                            Self::emit_source_speech(
                                &mut self.system_vad,
                                &sys_16k,
                                DeviceType::System,
                                &self.transcription_sender,
                                &mut self.chunk_id_counter,
                            );

                            // STEP 5: Persist three tracks for offline diarization + playback.
                            //   mic.mp4     → local user ("You")
                            //   system.mp4  → remote / computer audio
                            //   audio.mp4   → mixed playback (ducked)
                            if let Some(ref sender) = self.recording_sender_for_mixed {
                                let ts = chunk.timestamp;
                                let sr = self.sample_rate;
                                let _ = sender.send(AudioChunk {
                                    data: mic_window.clone(),
                                    sample_rate: sr,
                                    timestamp: ts,
                                    chunk_id: self.chunk_id_counter,
                                    device_type: DeviceType::Microphone,
                                });
                                let _ = sender.send(AudioChunk {
                                    data: sys_window.clone(),
                                    sample_rate: sr,
                                    timestamp: ts,
                                    chunk_id: self.chunk_id_counter,
                                    device_type: DeviceType::System,
                                });
                                let mixed = self.mixer.mix_window(&mic_window, &sys_window);
                                let _ = sender.send(AudioChunk {
                                    data: mixed,
                                    sample_rate: sr,
                                    timestamp: ts,
                                    chunk_id: self.chunk_id_counter,
                                    device_type: DeviceType::Mixed,
                                });
                            }
                        }
                    }
                }
                Ok(None) => {
                    info!(
                        "Audio pipeline: sender closed after processing {} chunks",
                        self.processed_chunks
                    );
                    break;
                }
                Err(_) => {
                    // WASAPI and some other backends can omit exact-zero
                    // callbacks. Finalize after the calibrated live redemption
                    // period, but never during Pause and never synthesize audio:
                    // wall time decides *when* to emit, while VAD audio time
                    // remains recording-relative and pause-aware.
                    self.finalize_inactive_speech(std::time::Instant::now());
                    continue;
                }
            }
        }

        // Flush any remaining VAD segments
        self.flush_remaining_audio()?;

        // Nothing more will arrive: convert what the resamplers still hold and
        // publish the working tracks. That tail is under one window — shorter
        // than the shortest segment VAD will emit — so it is stored for later
        // passes but not put through a VAD that has already been flushed.
        self.mic_work.finish();
        self.system_work.finish();
        self.own_speech_record.finish();

        info!(
            "🧵 Capture seams for this recording - {}",
            self.ring_buffer.seam_report()
        );
        if self.own_speech.is_open() {
            info!(
                "🔇 Own-speech detector: {} microphone segments dropped as the speakers coming back, {} detector samples let go",
                self.echo_segments_dropped,
                self.own_speech.dropped_samples()
            );
        }
        info!("VAD-driven audio pipeline ended");
        Ok(())
    }

    /// Write down what this window held, for both detectors.
    ///
    /// Called once per mixing window, in order, so the two timelines and the
    /// microphone VAD share one clock without any of them having to read it.
    fn observe_window(&mut self, sys_window: &[f32]) {
        let far_end = Some(far_end_is_playing(sys_window));
        let own_speech = self.own_speech.take_window(MIXING_WINDOW_MS as f64);
        self.far_end_timeline.push(far_end);
        self.own_speech_timeline.push(own_speech);
        // The same two answers, kept for whoever reads this recording later.
        // Written here rather than from the timelines above because those are
        // laid out in recording time, which runs on across a break in capture
        // while the stored track does not.
        self.own_speech_record.observe(own_speech, far_end);
    }

    /// Microphone speech, minus whatever was only the speakers coming back.
    fn emit_microphone_speech(&mut self, samples: &[f32]) {
        let segments = match self.mic_vad.process_audio(samples) {
            Ok(segments) => segments,
            Err(e) => {
                warn!("⚠️ Microphone VAD error: {}", e);
                return;
            }
        };
        let segments = self.without_the_speakers(segments);
        Self::enqueue_source_speech(
            segments,
            DeviceType::Microphone,
            &self.transcription_sender,
            &mut self.chunk_id_counter,
        );
    }

    /// Drop the microphone segments that are nothing but the speakers.
    ///
    /// Only the detector can tell the difference, so with it closed nothing is
    /// dropped: without a second opinion, the safe answer is that everything
    /// the microphone heard is the owner.
    fn without_the_speakers(&mut self, segments: Vec<SpeechSegment>) -> Vec<SpeechSegment> {
        if !self.own_speech.is_open() {
            return segments;
        }

        let mut kept = Vec::with_capacity(segments.len());
        for segment in segments {
            let from = segment.start_timestamp_ms;
            let to = segment.end_timestamp_ms;
            let (own_speech_ms, far_end_ms) =
                measure_segment(&self.own_speech_timeline, &self.far_end_timeline, from, to);

            if super::own_speech::is_only_the_speakers(own_speech_ms, far_end_ms) {
                self.echo_segments_dropped += 1;
                info!(
                    "🔇 Dropped {:.1}s of microphone at {:.1}s: the speakers played under it for {:.0}ms and the owner was audible for {:.0}ms",
                    (to - from) / 1000.0,
                    from / 1000.0,
                    far_end_ms,
                    own_speech_ms
                );
                continue;
            }
            kept.push(segment);
        }
        kept
    }

    /// The microphone window with the speakers' echo taken out of it.
    ///
    /// Windows does this far better than we can when the microphone was opened
    /// the way a voice call opens it: the driver has the signal that was played
    /// and the exact delay it came back with, and an application has neither.
    /// So while Windows is doing the job ours stands aside — subtracting an
    /// echo that is already gone takes some of the owner's voice with it.
    ///
    /// The question is asked here, once per window, rather than when the
    /// pipeline is built: the capture stream opens afterwards, and until it
    /// has, the answer is always the stale "no" from before the recording.
    fn cancel_echo(&mut self, mic_window: Vec<f32>, sys_window: &[f32]) -> Vec<f32> {
        #[cfg(target_os = "windows")]
        let system_is_cancelling = super::capture::wasapi_comms::is_active();
        #[cfg(not(target_os = "windows"))]
        let system_is_cancelling = false;

        cancel_echo_window(
            self.echo_canceller.as_mut(),
            &mut self.echo_left_to_windows,
            system_is_cancelling,
            mic_window,
            sys_window,
        )
    }

    fn flush_remaining_audio(&mut self) -> Result<()> {
        info!(
            "Flushing remaining audio from pipeline (processed {} chunks)",
            self.processed_chunks
        );

        while let Some((mic_window, sys_window)) = self.ring_buffer.extract_remaining() {
            // Same treatment as the live path for the trailing partial window
            self.observe_window(&sys_window);
            let mic_window = self.cancel_echo(mic_window, &sys_window);
            let mic_16k = self.mic_work.push(&mic_window);
            let sys_16k = self.system_work.push(&sys_window);
            self.emit_microphone_speech(&mic_16k);
            Self::emit_source_speech(
                &mut self.system_vad,
                &sys_16k,
                DeviceType::System,
                &self.transcription_sender,
                &mut self.chunk_id_counter,
            );

            if let Some(sender) = &self.recording_sender_for_mixed {
                let chunk_id = self.chunk_id_counter;
                for (data, device_type) in [
                    (mic_window.clone(), DeviceType::Microphone),
                    (sys_window.clone(), DeviceType::System),
                    (
                        self.mixer.mix_window(&mic_window, &sys_window),
                        DeviceType::Mixed,
                    ),
                ] {
                    let _ = sender.send(AudioChunk {
                        data,
                        sample_rate: self.sample_rate,
                        timestamp: 0.0,
                        chunk_id,
                        device_type,
                    });
                }
            }
        }

        let mic_final = self.mic_vad.flush();
        let sys_final = self.system_vad.flush();

        for (device_type, result) in [
            (DeviceType::Microphone, mic_final),
            (DeviceType::System, sys_final),
        ] {
            match result {
                Ok(final_segments) => {
                    for segment in final_segments {
                        let duration_ms = segment.end_timestamp_ms - segment.start_timestamp_ms;
                        if segment.samples.len() < 800 {
                            info!(
                                "⏭️ Skipping short final {:?} segment: {:.1}ms ({} samples < 800)",
                                device_type,
                                duration_ms,
                                segment.samples.len()
                            );
                            continue;
                        }
                        info!(
                            "📤 Sending final {:?} VAD segment: {:.1}ms, {} samples",
                            device_type,
                            duration_ms,
                            segment.samples.len()
                        );
                        let transcription_chunk = AudioChunk {
                            data: segment.samples,
                            sample_rate: 16000,
                            timestamp: segment.start_timestamp_ms / 1000.0,
                            chunk_id: self.chunk_id_counter,
                            device_type: device_type.clone(),
                        };
                        if let Err(e) = self.transcription_sender.send(transcription_chunk) {
                            warn!("Failed to send final {:?} VAD segment: {}", device_type, e);
                        } else {
                            self.chunk_id_counter += 1;
                        }
                    }
                }
                Err(e) => warn!("Failed to flush {:?} VAD: {}", device_type, e),
            }
        }

        Ok(())
    }
}

/// Simple audio pipeline manager
pub struct AudioPipelineManager {
    pipeline_handle: Option<JoinHandle<Result<()>>>,
    audio_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
}

impl AudioPipelineManager {
    pub fn new() -> Self {
        Self {
            pipeline_handle: None,
            audio_sender: None,
        }
    }

    /// Start the audio pipeline with device information for adaptive buffering
    pub fn start(
        &mut self,
        state: Arc<RecordingState>,
        transcription_sender: mpsc::UnboundedSender<AudioChunk>,
        target_chunk_duration_ms: u32,
        sample_rate: u32,
        recording_sender: Option<mpsc::UnboundedSender<AudioChunk>>,
        mic_device_name: String,
        mic_device_kind: super::device_detection::InputDeviceKind,
        system_device_name: String,
        system_device_kind: super::device_detection::InputDeviceKind,
        level_sender: Option<mpsc::UnboundedSender<AudioLevels>>,
        // Meeting folder to write the working tracks into. `None` when the
        // recording is not being kept, and nothing derived from it should be.
        meeting_folder: Option<std::path::PathBuf>,
    ) -> Result<()> {
        // Log device information for adaptive buffering
        info!("🎙️ Starting pipeline with device info:");
        info!(
            "   Microphone: '{}' ({:?})",
            mic_device_name, mic_device_kind
        );
        info!(
            "   System Audio: '{}' ({:?})",
            system_device_name, system_device_kind
        );

        // Create audio processing channel
        let (audio_sender, audio_receiver) = mpsc::unbounded_channel::<AudioChunk>();

        // Set sender in state for audio captures to use
        state.set_audio_sender(audio_sender.clone());

        // Create and start pipeline with device information for adaptive mixing
        let mut pipeline = AudioPipeline::new(
            audio_receiver,
            transcription_sender,
            state.clone(),
            target_chunk_duration_ms,
            sample_rate,
            mic_device_name,
            mic_device_kind,
            system_device_name,
            system_device_kind,
        );

        // CRITICAL FIX: Connect recording sender to receive pre-mixed audio
        // This ensures both mic AND system audio are captured in recordings
        pipeline.recording_sender_for_mixed = recording_sender;

        // Connect live level meter output (mic + system) for the frontend visualizer
        pipeline.level_sender = level_sender;

        // Keep the 16 kHz stream this pipeline produces anyway, so a later pass
        // over the recording does not have to reconstruct it.
        if let Some(folder) = meeting_folder {
            pipeline.open_working_tracks(&folder);
        }

        let handle = tokio::spawn(async move { pipeline.run().await });

        self.pipeline_handle = Some(handle);
        self.audio_sender = Some(audio_sender);

        info!("Audio pipeline manager started with mixed audio recording");
        Ok(())
    }

    /// Stop the audio pipeline
    pub async fn stop(&mut self) -> Result<()> {
        // Drop the sender to close the pipeline
        self.audio_sender = None;

        // Wait for pipeline to finish
        if let Some(handle) = self.pipeline_handle.take() {
            match handle.await {
                Ok(result) => result,
                Err(e) => {
                    error!("Pipeline task failed: {}", e);
                    Ok(())
                }
            }
        } else {
            Ok(())
        }
    }

    /// Force immediate flush of accumulated audio and stop pipeline
    /// PERFORMANCE CRITICAL: Eliminates 30+ second shutdown delays
    pub async fn force_flush_and_stop(&mut self) -> Result<()> {
        info!("🚀 Force flushing pipeline - processing ALL accumulated audio immediately");

        // If we have a sender, send a special flush signal first
        if let Some(sender) = &self.audio_sender {
            // Create a special flush chunk to trigger immediate processing
            let flush_chunk = AudioChunk {
                data: vec![], // Empty data signals flush
                sample_rate: 16000,
                timestamp: 0.0,
                chunk_id: u64::MAX, // Special ID to indicate flush
                device_type: super::recording_state::DeviceType::Microphone,
            };

            if let Err(e) = sender.send(flush_chunk) {
                warn!("Failed to send flush signal: {}", e);
            } else {
                info!("📤 Sent flush signal to pipeline");

                // PERFORMANCE OPTIMIZATION: Reduced wait time from 50ms to 20ms
                // Pipeline should process flush signal very quickly
                tokio::time::sleep(tokio::time::Duration::from_millis(20)).await;

                // Send multiple flush signals to ensure the pipeline catches it
                // This aggressive approach eliminates shutdown delay issues
                for i in 0..3 {
                    let additional_flush = AudioChunk {
                        data: vec![],
                        sample_rate: 16000,
                        timestamp: 0.0,
                        chunk_id: u64::MAX - (i as u64),
                        device_type: super::recording_state::DeviceType::Microphone,
                    };
                    let _ = sender.send(additional_flush);
                }

                info!("📤 Sent additional flush signals for reliability");
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        }

        // Now stop normally
        self.stop().await
    }
}

impl Default for AudioPipelineManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod working_track_tests;

#[cfg(test)]
mod ring_buffer_tests;

/// A bench, not a test: it runs real speech through the mixing buffer under a
/// delivery profile taken from the owner's own logs, and prints what came out
/// the other side.
///
/// It exists because the alternative was the owner recording himself, playing
/// it back and counting clicks by ear after every change. His ear was right
/// every time — the counts matched the log to within one — but it is not a
/// thing to spend a person on.
///
/// Marked `#[ignore]` and run on demand:
///
/// ```text
/// frontend/src-tauri/scripts/make-bench-speech.ps1      # once
/// cargo test --lib audio_bench -- --ignored --nocapture
/// ```
///
/// What it cannot do: produce the jitter itself. That is born in the capture
/// callback on a real device. The profile here is modelled on what the logs
/// measured — blocks arriving up to ~350ms late and then in a burst that more
/// than catches up — so the bench measures how the pipeline answers that, not
/// whether the device does it.
#[cfg(test)]
mod audio_bench;

#[cfg(test)]
mod echo_ownership_tests;

#[cfg(test)]
mod own_speech_gate_tests;
