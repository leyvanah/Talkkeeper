//! Mixing the microphone and system streams into the playback track.
//!
//! The ring buffer lines the two sources up in time (each arrives on its own
//! clock and in its own bursts) and hands out equal windows of both; the
//! mixer sums a window with the system gain applied and a limiter on top.
//! Nothing here decides what is transcribed: that runs on the separate,
//! unmixed streams in the parent module.

use super::*;

/// How long a shortfall must persist before it is read as a break in the
/// stream rather than a late thread. Long enough that no burst of work inside
/// the app can hold every reading in the window high, short enough that a real
/// break is repaired while it is still the current one.
pub(super) const TIMELINE_OBSERVATION_SECONDS: f64 = 2.0;

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
pub(super) const TIMELINE_GAP_SECONDS: f64 = 0.3;

/// A sustained shortfall this large is no longer a gap to fill but a stream
/// that has to be picked up again from where it now is.
pub(super) const TIMELINE_RESET_SECONDS: f64 = 5.0;

/// The block the pipeline works in: mic and system are aligned, echo-cancelled,
/// mixed and converted to the working rate one window at a time.
pub(super) const MIXING_WINDOW_MS: f32 = 50.0;

/// That window in capture samples.
pub(super) fn mixing_window_samples(sample_rate: u32) -> usize {
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
pub(super) struct SourceTimeline {
    /// True once this source has placed a block, so its start has been lined
    /// up against the other source.
    pub(super) started: bool,
    /// Recent (arrival clock, shortfall in seconds), oldest first.
    pub(super) lag_readings: VecDeque<(f64, f64)>,
}

impl SourceTimeline {
    pub(super) fn reset(&mut self) {
        self.started = false;
        self.lag_readings.clear();
    }

    /// The part of the current shortfall that has lasted long enough to be the
    /// stream rather than the scheduler. Zero while the readings disagree.
    pub(super) fn sustained_gap(&mut self, clock: f64, lag_seconds: f64) -> f64 {
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
pub(super) struct AudioMixerRingBuffer {
    pub(super) mic_buffer: VecDeque<f32>,
    pub(super) system_buffer: VecDeque<f32>,
    pub(super) window_size_samples: usize,  // Fixed mixing window (e.g., 50ms)
    pub(super) max_buffer_size: usize,  // Safety limit (e.g., 100ms)
    pub(super) mic_enabled: bool,
    pub(super) system_enabled: bool,
    pub(super) sample_rate: f64,
    pub(super) timeline_origin: Option<f64>,
    pub(super) output_samples: usize,
    pub(super) mic_timeline: SourceTimeline,
    pub(super) system_timeline: SourceTimeline,
    /// Diagnostics: how often the assembled audio was not simply the stream as
    /// captured. Any of these being non-zero means the recording has seams.
    pub(super) padded_mic_windows: u64,
    pub(super) padded_system_windows: u64,
    pub(super) dropped_samples: u64,
    /// Samples each device actually handed over. Short of the recording's own
    /// length means the audio was lost before this buffer ever saw it - in the
    /// capture callback or below it - and no amount of care here recovers it.
    pub(super) mic_received_samples: u64,
    pub(super) system_received_samples: u64,
    pub(super) mic_inserted_samples: u64,
    pub(super) system_inserted_samples: u64,
    /// How many separate repairs those inserted samples were spread across.
    ///
    /// The total alone cannot be acted on: a second of silence appended once at
    /// the end is inaudible, while the same second split into ten fills lands in
    /// the middle of speech ten times. Counting the events is what tells those
    /// apart, and it is what the owner hears.
    pub(super) mic_fill_events: u64,
    pub(super) system_fill_events: u64,
    pub(super) timeline_resets: u64,
}

impl AudioMixerRingBuffer {
    pub(super) fn new(sample_rate: u32, mic_enabled: bool, system_enabled: bool) -> Self {
        Self::with_window_ms(sample_rate, mic_enabled, system_enabled, MIXING_WINDOW_MS)
    }

    pub(super) fn with_window_ms(
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

    pub(super) fn set_enabled(&mut self, mic_enabled: bool, system_enabled: bool) {
        self.mic_enabled = mic_enabled;
        self.system_enabled = system_enabled;
    }

    pub(super) fn add_samples(
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

    pub(super) fn can_mix(&self) -> bool {
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

    pub(super) fn extract_window(&mut self) -> Option<(Vec<f32>, Vec<f32>)> {
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
    pub(super) fn seam_report(&self) -> String {
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

    pub(super) fn extract_remaining(&mut self) -> Option<(Vec<f32>, Vec<f32>)> {
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
pub(super) struct ProfessionalAudioMixer;

/// Apply user-selected system gain without changing sample count or source timing.
/// If the gained chunk exceeds -1 dBFS, attenuate the whole chunk so its waveform
/// shape is preserved instead of hard-clipping individual samples.
pub(super) fn apply_system_gain(samples: &mut [f32], gain: f32) -> bool {
    pub(super) const PEAK_LIMIT: f32 = 0.891_250_9; // -1 dBFS

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
    pub(super) fn new(_sample_rate: u32) -> Self {
        Self
    }

    pub(super) fn mix_window(&mut self, mic_window: &[f32], sys_window: &[f32]) -> Vec<f32> {
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
