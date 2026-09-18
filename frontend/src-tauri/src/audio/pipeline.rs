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

/// How long a shortfall must persist before it is read as a break in the
/// stream rather than a late thread. Long enough that no burst of work inside
/// the app can hold every reading in the window high, short enough that a real
/// break is repaired while it is still the current one.
const TIMELINE_OBSERVATION_SECONDS: f64 = 2.0;

/// The smallest sustained shortfall worth reporting at all.
///
/// No longer a repair threshold — nothing between this and
/// [`TIMELINE_RESET_SECONDS`] is filled any more (see the fill decision in
/// `add_samples`). It is what a shortfall has to reach before it counts as a
/// reading rather than noise, which is how a real break grows to the point of
/// resetting the timelines.
///
/// Raised from 100ms because the clock is read at the end of the capture
/// handler: anything that holds the handler up makes a block look late, and at
/// 100ms the delivery jitter of an ordinary recording cleared that line
/// constantly. Measured false readings ran to 142ms, so the line sits at twice
/// that.
const TIMELINE_GAP_SECONDS: f64 = 0.3;

/// A sustained shortfall this large is no longer a gap to fill but a stream
/// that has to be picked up again from where it now is.
const TIMELINE_RESET_SECONDS: f64 = 5.0;

/// The block the pipeline works in: mic and system are aligned, echo-cancelled,
/// mixed and converted to the working rate one window at a time.
const MIXING_WINDOW_MS: f32 = 50.0;

/// That window in capture samples.
fn mixing_window_samples(sample_rate: u32) -> usize {
    ((sample_rate as f32 * MIXING_WINDOW_MS / 1000.0) as usize).max(1)
}

/// One capture source's place on the shared timeline.
///
/// A sound card hands over a continuous, rate-accurate stream. The timestamp
/// on a block is not the device's: it is the recording clock read at the end of
/// the capture handler, after that block has been filtered, resampled and
/// normalized. Anything that holds the handler up pushes the reading later,
/// and nothing can push it earlier. A single late reading therefore says
/// nothing about the audio — under load it said a lot, and every correction it
/// triggered was a step in the waveform. Only a shortfall that *every* reading
/// over a window agrees on is the device having stopped producing sound.
#[derive(Default)]
struct SourceTimeline {
    /// True once this source has placed a block, so its start has been lined
    /// up against the other source.
    started: bool,
    /// Recent (arrival clock, shortfall in seconds), oldest first.
    lag_readings: VecDeque<(f64, f64)>,
}

impl SourceTimeline {
    fn reset(&mut self) {
        self.started = false;
        self.lag_readings.clear();
    }

    /// The part of the current shortfall that has lasted long enough to be the
    /// stream rather than the scheduler. Zero while the readings disagree.
    fn sustained_gap(&mut self, clock: f64, lag_seconds: f64) -> f64 {
        self.lag_readings.push_back((clock, lag_seconds));
        // Keep one reading from beyond the window's far edge, so the span of
        // what is kept covers the whole window rather than stopping just short.
        while self.lag_readings.len() >= 2
            && clock - self.lag_readings[1].0 >= TIMELINE_OBSERVATION_SECONDS
        {
            self.lag_readings.pop_front();
        }

        // Not enough history yet to tell a stalled device from a late thread.
        let Some(&(oldest_at, _)) = self.lag_readings.front() else {
            return 0.0;
        };
        if clock - oldest_at < TIMELINE_OBSERVATION_SECONDS {
            return 0.0;
        }

        let sustained = self
            .lag_readings
            .iter()
            .map(|&(_, lag)| lag)
            .fold(f64::INFINITY, f64::min);
        if sustained >= TIMELINE_GAP_SECONDS {
            // Repaired: the readings still to come are measured against the
            // timeline as it will be after the fill.
            self.lag_readings.clear();
            sustained
        } else {
            0.0
        }
    }
}

/// Ring buffer for synchronized audio mixing
/// Accumulates samples from mic and system streams until we have aligned windows
struct AudioMixerRingBuffer {
    mic_buffer: VecDeque<f32>,
    system_buffer: VecDeque<f32>,
    window_size_samples: usize,  // Fixed mixing window (e.g., 50ms)
    max_buffer_size: usize,  // Safety limit (e.g., 100ms)
    mic_enabled: bool,
    system_enabled: bool,
    sample_rate: f64,
    timeline_origin: Option<f64>,
    output_samples: usize,
    mic_timeline: SourceTimeline,
    system_timeline: SourceTimeline,
    /// Diagnostics: how often the assembled audio was not simply the stream as
    /// captured. Any of these being non-zero means the recording has seams.
    padded_mic_windows: u64,
    padded_system_windows: u64,
    dropped_samples: u64,
    /// Samples each device actually handed over. Short of the recording's own
    /// length means the audio was lost before this buffer ever saw it - in the
    /// capture callback or below it - and no amount of care here recovers it.
    mic_received_samples: u64,
    system_received_samples: u64,
    mic_inserted_samples: u64,
    system_inserted_samples: u64,
    /// How many separate repairs those inserted samples were spread across.
    ///
    /// The total alone cannot be acted on: a second of silence appended once at
    /// the end is inaudible, while the same second split into ten fills lands in
    /// the middle of speech ten times. Counting the events is what tells those
    /// apart, and it is what the owner hears.
    mic_fill_events: u64,
    system_fill_events: u64,
    timeline_resets: u64,
}

impl AudioMixerRingBuffer {
    fn new(sample_rate: u32, mic_enabled: bool, system_enabled: bool) -> Self {
        Self::with_window_ms(sample_rate, mic_enabled, system_enabled, MIXING_WINDOW_MS)
    }

    fn with_window_ms(
        sample_rate: u32,
        mic_enabled: bool,
        system_enabled: bool,
        window_ms: f32,
    ) -> Self {
        let window_size_samples = ((sample_rate as f32 * window_ms / 1000.0) as usize).max(1);

        // Holds a second, and it has to outlast the patience in `can_mix`.
        //
        // A source is waited on for twelve windows before its partner is mixed
        // without it; a cap below that would throw away the very samples being
        // waited for, at the overflow check, and count them as dropped. The cap
        // is what a source may run ahead by while the other catches up, so it
        // is set well clear of the 600ms of waiting: jitter this deep is
        // ordinary here, and a recording is not a live monitor — latency costs
        // nothing, discarded audio cannot be recovered.
        let max_buffer_size = window_size_samples * 20;  // 1s (was 400ms)

        info!(
            "🔊 Ring buffer initialized: window={}ms ({} samples), max={}ms ({} samples)",
            window_ms,
            window_size_samples,
            window_ms * 8.0,
            max_buffer_size
        );

        Self {
            mic_buffer: VecDeque::with_capacity(max_buffer_size),
            system_buffer: VecDeque::with_capacity(max_buffer_size),
            window_size_samples,
            max_buffer_size,
            mic_enabled,
            system_enabled,
            sample_rate: sample_rate as f64,
            timeline_origin: None,
            output_samples: 0,
            mic_timeline: SourceTimeline::default(),
            system_timeline: SourceTimeline::default(),
            padded_mic_windows: 0,
            padded_system_windows: 0,
            dropped_samples: 0,
            mic_received_samples: 0,
            system_received_samples: 0,
            mic_inserted_samples: 0,
            system_inserted_samples: 0,
            mic_fill_events: 0,
            system_fill_events: 0,
            timeline_resets: 0,
        }
    }

    fn set_enabled(&mut self, mic_enabled: bool, system_enabled: bool) {
        self.mic_enabled = mic_enabled;
        self.system_enabled = system_enabled;
    }

    fn add_samples(
        &mut self,
        device_type: DeviceType,
        samples: Vec<f32>,
        timestamp: f64,
    ) -> Option<f64> {
        // Log buffer health periodically for diagnostics
        static mut SAMPLE_COUNTER: u64 = 0;
        unsafe {
            SAMPLE_COUNTER += 1;
            if SAMPLE_COUNTER % 200 == 0 {
                debug!(
                    "📊 Ring buffer status: mic={} samples, sys={} samples (max={})",
                    self.mic_buffer.len(),
                    self.system_buffer.len(),
                    self.max_buffer_size
                );
            }
        }

        if matches!(device_type, DeviceType::Mixed) {
            return None;
        }

        let duration = samples.len() as f64 / self.sample_rate;
        let start = (timestamp - duration).max(0.0);
        let origin = *self.timeline_origin.get_or_insert(start);
        let chunk_start = (start - origin).max(0.0) * self.sample_rate;
        let buffered_end = self.output_samples
            + match device_type {
                DeviceType::Microphone => self.mic_buffer.len(),
                _ => self.system_buffer.len(),
            };

        // How far behind where the clock puts it this source's timeline sits.
        // Positive means the clock ran ahead: either the device stopped
        // producing, or the block simply took longer to reach here.
        let lag_seconds = chunk_start / self.sample_rate - buffered_end as f64 / self.sample_rate;
        let sample_rate = self.sample_rate;
        let timeline = match device_type {
            DeviceType::Microphone => &mut self.mic_timeline,
            _ => &mut self.system_timeline,
        };
        let already_running = timeline.started;
        let gap_seconds = if already_running {
            timeline.sustained_gap(timestamp, lag_seconds)
        } else {
            // The very first block of a source is where the two streams are
            // lined up against each other; there is no history to weigh it
            // against and nothing yet to disturb.
            timeline.started = true;
            lag_seconds.max(0.0)
        };

        let mut discontinuity_start = None;
        if gap_seconds >= TIMELINE_RESET_SECONDS {
            warn!(
                "Audio timeline discontinuity of {:.1}s detected; resetting source alignment",
                gap_seconds
            );
            self.timeline_resets += 1;
            self.mic_buffer.clear();
            self.system_buffer.clear();
            self.mic_timeline.reset();
            self.system_timeline.reset();
            self.timeline_origin = Some(start);
            self.output_samples = 0;
            discontinuity_start = Some(start);
        }

        // Silence is written into a source's timeline for exactly one reason:
        // to line its first block up against the other source, which starts at
        // its own moment. Nothing after that is repaired.
        //
        // What used to be repaired here was a shortfall measured against the
        // clock, and three recordings showed there is no shortfall to repair.
        // Each delivered *more* audio than it had running time — 100.1%, 101.4%
        // and 104% — with no samples dropped and not one window short on the way
        // out. Blocks simply arrive unevenly: a third of a second late, then in
        // a burst that more than catches up. A jitter that deep is not
        // distinguishable from a device that stopped, so repairing it meant
        // splicing silence into speech every time the delivery bunched up, and
        // the owner heard every splice as a click in his own voice. Raising the
        // threshold only made the splices bigger: the shortfall accumulates
        // until whatever the threshold is cuts it off.
        //
        // The two things the repair existed for are still handled. A source
        // starting late is lined up on its first block, above. A break large
        // enough to matter — five seconds — resets both timelines instead,
        // which is the honest response to a stream that has to be picked up
        // again rather than patched.
        //
        // What is given up: a genuine dropout between 0.3s and 5s leaves the
        // channels that far apart until the next reset. Across three recordings
        // that never once happened, while the jitter happened in all of them.
        let fill = if discontinuity_start.is_some() || already_running {
            0
        } else {
            (gap_seconds.max(0.0) * sample_rate).round() as usize
        };
        let is_mic = matches!(device_type, DeviceType::Microphone);
        if is_mic {
            self.mic_inserted_samples += fill as u64;
            self.mic_received_samples += samples.len() as u64;
            if fill > 0 {
                self.mic_fill_events += 1;
            }
        } else {
            self.system_inserted_samples += fill as u64;
            self.system_received_samples += samples.len() as u64;
            if fill > 0 {
                self.system_fill_events += 1;
            }
        }

        // One line per repair, at info, because the owner hears each one of
        // these as a break in their own voice and debug is not written to the
        // log file. Eleven lines for a minute of recording is a diagnosis; it
        // says whether the shortfall grows steadily (a rate mismatch) or comes
        // in bursts (a handler that was held up), which have different cures.
        if fill > 0 {
            info!(
                "🔇 Filled a {:.0}ms gap in the {} timeline at {:.1}s of the recording \
                 (shortfall {:.0}ms, raw lag {:.0}ms, {} samples buffered)",
                fill as f64 / sample_rate * 1000.0,
                if is_mic { "microphone" } else { "system" },
                timestamp,
                gap_seconds * 1000.0,
                lag_seconds * 1000.0,
                buffered_end,
            );
        }

        let buffer = match device_type {
            DeviceType::Microphone => &mut self.mic_buffer,
            _ => &mut self.system_buffer,
        };
        buffer.extend(std::iter::repeat(0.0).take(fill));
        // Captured samples are never trimmed to meet the clock. The clock is
        // the less trustworthy of the two, and a trim deletes speech.
        buffer.extend(samples);

        // CRITICAL FIX: Add warnings before dropping samples
        // This helps diagnose timing issues in production
        if self.mic_buffer.len() > self.max_buffer_size {
            warn!(
                "⚠️ Microphone buffer overflow: {} > {} samples, dropping oldest {} samples",
                self.mic_buffer.len(),
                self.max_buffer_size,
                self.mic_buffer.len() - self.max_buffer_size
            );
        }
        if self.system_buffer.len() > self.max_buffer_size {
            error!("🔴 SYSTEM AUDIO BUFFER OVERFLOW: {} > {} samples, dropping {} samples - THIS CAUSES DISTORTION!",
                  self.system_buffer.len(), self.max_buffer_size,
                  self.system_buffer.len() - self.max_buffer_size);
        }

        // Safety: prevent buffer overflow (keep only last 200ms)
        while self.mic_buffer.len() > self.max_buffer_size {
            self.mic_buffer.pop_front();
            self.dropped_samples += 1;
        }
        while self.system_buffer.len() > self.max_buffer_size {
            self.system_buffer.pop_front();
            self.dropped_samples += 1;
        }
        discontinuity_start
    }

    fn can_mix(&self) -> bool {
        let all_ready = (!self.mic_enabled || self.mic_buffer.len() >= self.window_size_samples)
            && (!self.system_enabled || self.system_buffer.len() >= self.window_size_samples)
            && (self.mic_enabled || self.system_enabled);
        // Mixing without one of the sources fills its window with silence, so
        // the lead has to be long enough that only a source which has really
        // stopped can reach it - not one whose blocks are momentarily late.
        //
        // Six windows was 300ms, and the measured delivery jitter reaches
        // 346ms. While the timeline still padded a late source, its buffer was
        // topped up with silence and this line was never reached; with that
        // padding gone the jitter walked straight past it — 57 windows in one
        // recording came out half empty, against none before. A half-empty
        // microphone window paired with a full system one is worse than a late
        // one: the echo canceller subtracts the far end from the wrong place,
        // and the other speaker's voice stays in the owner's channel.
        //
        // Twelve windows is 600ms: past the jitter with room to spare, and
        // still far short of the five seconds that count as a real break.
        let surviving_source_ahead =
            self.mic_buffer.len().max(self.system_buffer.len()) >= self.window_size_samples * 12;
        all_ready || surviving_source_ahead
    }

    fn extract_window(&mut self) -> Option<(Vec<f32>, Vec<f32>)> {
        if !self.can_mix() {
            return None;
        }

        // Extract mic window with zero-padding for incomplete buffers
        // Zero-padding (silence) is preferred over last-sample-hold to prevent artifacts

        // Extract mic window (or pad with zeros if insufficient data)
        let mic_window = if self.mic_buffer.len() >= self.window_size_samples {
            // Enough mic data - drain window
            self.mic_buffer.drain(0..self.window_size_samples).collect()
        } else if !self.mic_buffer.is_empty() {
            self.padded_mic_windows += 1;
            // Some mic data but not enough - consume all + pad with zeros
            let available: Vec<f32> = self.mic_buffer.drain(..).collect();
            let mut padded = Vec::with_capacity(self.window_size_samples);
            padded.extend_from_slice(&available);

            // Use zero-padding (silence) to prevent repetition artifacts
            // Zero-padding is inaudible at 48kHz sample rate
            padded.resize(self.window_size_samples, 0.0);

            padded
        } else {
            // A window with no microphone data at all is the same silence as
            // a half-filled one, only more of it, so it counts the same way.
            // It did not use to be counted, which made `windows padded: 0`
            // read as "nothing was substituted" when whole windows were.
            self.padded_mic_windows += 1;
            vec![0.0; self.window_size_samples]
        };

        // Extract system window (or pad with zeros if insufficient data)
        let sys_window = if self.system_buffer.len() >= self.window_size_samples {
            // Enough system data - drain window
            self.system_buffer
                .drain(0..self.window_size_samples)
                .collect()
        } else if !self.system_buffer.is_empty() {
            self.padded_system_windows += 1;
            // Some system data but not enough - consume all + pad with zeros
            let available: Vec<f32> = self.system_buffer.drain(..).collect();
            let mut padded = Vec::with_capacity(self.window_size_samples);
            padded.extend_from_slice(&available);

            // Use zero-padding (silence) to prevent repetition artifacts
            // Zero-padding is inaudible at 48kHz sample rate
            padded.resize(self.window_size_samples, 0.0);

            padded
        } else {
            // Counted for the same reason as the microphone side above.
            self.padded_system_windows += 1;
            vec![0.0; self.window_size_samples]
        };

        self.output_samples += self.window_size_samples;
        Some((mic_window, sys_window))
    }

    /// Everything that made the saved audio differ from the captured stream.
    fn seam_report(&self) -> String {
        let seconds = |samples: u64| samples as f64 / self.sample_rate;
        format!(
            "mic delivered {:.3}s (+{:.3}s silence inserted over {} fills, {} windows padded), \
             system delivered {:.3}s (+{:.3}s silence inserted over {} fills, {} windows padded), \
             samples dropped: {}, timeline resets: {}",
            seconds(self.mic_received_samples),
            seconds(self.mic_inserted_samples),
            self.mic_fill_events,
            self.padded_mic_windows,
            seconds(self.system_received_samples),
            seconds(self.system_inserted_samples),
            self.system_fill_events,
            self.padded_system_windows,
            self.dropped_samples,
            self.timeline_resets
        )
    }

    fn extract_remaining(&mut self) -> Option<(Vec<f32>, Vec<f32>)> {
        let len = self
            .mic_buffer
            .len()
            .max(self.system_buffer.len())
            .min(self.window_size_samples);
        if len == 0 {
            return None;
        }

        let mut mic: Vec<f32> = self
            .mic_buffer
            .drain(..self.mic_buffer.len().min(len))
            .collect();
        let mut system: Vec<f32> = self
            .system_buffer
            .drain(..self.system_buffer.len().min(len))
            .collect();
        mic.resize(len, 0.0);
        system.resize(len, 0.0);
        self.output_samples += len;
        Some((mic, system))
    }
}

/// Mixes mic + system for the *recording file only*.
///
/// Transcription no longer hears this mix — each source is VAD'd and sent to
/// Whisper on its own path (see the dual-VAD loop below). That way simultaneous
/// talk doesn't make the louder side crush the quieter one in STT.
///
/// For the recording we still want a single stereo-ish mono file, so:
/// 1. Always give system some headroom (loopback is near full-scale; mic is quieter dialog).
/// 2. Duck system further while the local mic is active.
/// 3. If the sum would clip, shrink **system first** so the mic keeps its level.
struct ProfessionalAudioMixer;

/// Apply user-selected system gain without changing sample count or source timing.
/// If the gained chunk exceeds -1 dBFS, attenuate the whole chunk so its waveform
/// shape is preserved instead of hard-clipping individual samples.
fn apply_system_gain(samples: &mut [f32], gain: f32) -> bool {
    const PEAK_LIMIT: f32 = 0.891_250_9; // -1 dBFS

    let gain = gain.clamp(0.5, 3.0);
    let gained_peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs() * gain));
    let limiter_hit = gained_peak > PEAK_LIMIT;
    let effective_gain = if limiter_hit {
        gain * PEAK_LIMIT / gained_peak
    } else {
        gain
    };

    for sample in samples {
        *sample *= effective_gain;
    }
    limiter_hit
}

impl ProfessionalAudioMixer {
    fn new(_sample_rate: u32) -> Self {
        Self
    }

    fn mix_window(&mut self, mic_window: &[f32], sys_window: &[f32]) -> Vec<f32> {
        let max_len = mic_window.len().max(sys_window.len());
        let mut mixed = Vec::with_capacity(max_len);

        let mic_rms = if mic_window.is_empty() {
            0.0
        } else {
            (mic_window.iter().map(|&x| x * x).sum::<f32>() / mic_window.len() as f32).sqrt()
        };
        // Headroom always; extra duck while the user is speaking so remote audio
        // doesn't bury them in the saved recording.
        let sys_gain = if mic_rms > 0.012 { 0.50 } else { 0.65 };

        for i in 0..max_len {
            let m = mic_window.get(i).copied().unwrap_or(0.0);
            let mut s = sys_window.get(i).copied().unwrap_or(0.0) * sys_gain;

            let sum = m + s;
            let mixed_sample = if sum.abs() <= 1.0 {
                sum
            } else {
                // Mic-priority limiting: carve system down to leave room for mic.
                let room = (1.0 - m.abs()).max(0.0);
                if s.abs() > room {
                    s = s.signum() * room;
                }
                let sum2 = m + s;
                if sum2.abs() > 1.0 {
                    // Last resort (extreme mic peaks): soft-scale the residual.
                    sum2 / sum2.abs()
                } else {
                    sum2
                }
            };

            mixed.push(mixed_sample);
        }

        mixed
    }
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
