use anyhow::Result;
use chrono::Utc;
use log::{debug, info, warn};
use realfft::num_complex::{Complex32, ComplexFloat};
use realfft::RealFftPlanner;
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use std::collections::VecDeque;
use std::path::PathBuf;
use nnnoiseless::DenoiseState;

use super::encode::encode_single_audio; // Correct path to encode module

/// Sanitize a filename to be safe for filesystem use
pub fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '\'' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Create a meeting folder with timestamp and return the path
/// Creates structure: base_path/MeetingName_YYYY-MM-DD_HH-MM-SS-mmm/
///
/// # Arguments
/// * `base_path` - Base directory for meetings
/// * `meeting_name` - Name of the meeting
pub fn create_meeting_folder(base_path: &PathBuf, meeting_name: &str) -> Result<PathBuf> {
    std::fs::create_dir_all(base_path)?;
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S-%3f").to_string();
    let sanitized_name = sanitize_filename(meeting_name);
    let folder_stem = format!("{}_{}", sanitized_name, timestamp);
    let meeting_folder = (0..1000)
        .find_map(|suffix| {
            let folder_name = if suffix == 0 {
                folder_stem.clone()
            } else {
                format!("{}_{}", folder_stem, suffix)
            };
            let candidate = base_path.join(folder_name);
            match std::fs::create_dir(&candidate) {
                Ok(()) => Some(Ok(candidate)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .transpose()?
        .ok_or_else(|| anyhow::anyhow!("Could not create a unique meeting folder"))?;

    log::info!("Created meeting folder: {}", meeting_folder.display());
    Ok(meeting_folder)
}

pub fn normalize_v2(audio: &[f32]) -> Vec<f32> {
    let rms = (audio.iter().map(|&x| x * x).sum::<f32>() / audio.len() as f32).sqrt();
    let peak = audio
        .iter()
        .fold(0.0f32, |max, &sample| max.max(sample.abs()));

    // Return the original audio if it's completely silent
    if rms == 0.0 || peak == 0.0 {
        return audio.to_vec();
    }

    // Increase target RMS for better voice volume while keeping peak in check
    let target_rms = 0.9;  // Increased from 0.6
    let target_peak = 0.95; // Slightly reduced to prevent clipping

    let rms_scaling = target_rms / rms;
    let peak_scaling = target_peak / peak;

    // Apply a minimum scaling factor to boost very quiet audio
    let min_scaling = 1.5; // Minimum boost for quiet audio
    let scaling_factor = (rms_scaling.min(peak_scaling)).max(min_scaling);

    // Apply scaling with soft clipping to prevent harsh distortion
    audio
        .iter()
        .map(|&sample| {
            let scaled = sample * scaling_factor;
            // Soft clip at ±0.95 to prevent harsh distortion
            if scaled > 0.95 {
                0.95 + (scaled - 0.95) * 0.05
            } else if scaled < -0.95 {
                -0.95 + (scaled + 0.95) * 0.05
            } else {
                scaled
            }
        })
        .collect()
}

/// True peak limiter: holds the signal back by a lookahead window and rides one
/// smooth gain over it, so a loud transient is ducked as a whole.
///
/// The previous version scaled single samples that crossed the limit and left
/// their neighbours untouched, which is clipping by another name - it flattened
/// the tips of plosives and consonants and sprayed high-frequency hash across
/// the recording. That was audible as crackle and cost the transcription words.
struct TruePeakLimiter {
    /// Delay line holding the samples not yet released.
    buffer: Vec<f32>,
    write_position: usize,
    /// Loudest magnitude still inside the lookahead window, kept as a
    /// decreasing run of (sample index, magnitude) so the front is always the
    /// answer. Reading it off a plain scan of the delay line instead cost 144
    /// comparisons for every one of 48000 samples a second - inside the
    /// capture callback, where whatever does not finish in time is audio the
    /// device drops.
    peaks: VecDeque<(u64, f32)>,
    samples_seen: u64,
    /// Gain currently applied, moving towards `target_gain`.
    gain: f32,
    /// How fast the gain may fall (per sample) when a peak arrives.
    attack_coefficient: f32,
    /// How fast it returns to unity once the peak has passed.
    release_coefficient: f32,
}

impl TruePeakLimiter {
    fn new(sample_rate: u32) -> Self {
        // Long enough to see a transient coming, short enough to stay live
        const LOOKAHEAD_MS: f32 = 3.0;
        // Reaching the needed reduction within the lookahead avoids overshoot
        const ATTACK_MS: f32 = 1.0;
        // Slow enough that speech does not pump between syllables
        const RELEASE_MS: f32 = 80.0;

        let lookahead_samples = ((sample_rate as f32 * LOOKAHEAD_MS / 1000.0) as usize).max(1);
        let per_sample = |ms: f32| 1.0 - (-1.0 / (sample_rate as f32 * ms / 1000.0)).exp();

        Self {
            buffer: vec![0.0; lookahead_samples],
            write_position: 0,
            peaks: VecDeque::with_capacity(lookahead_samples),
            samples_seen: 0,
            gain: 1.0,
            attack_coefficient: per_sample(ATTACK_MS),
            release_coefficient: per_sample(RELEASE_MS),
        }
    }

    fn process(&mut self, sample: f32, true_peak_limit: f32) -> f32 {
        // The incoming sample enters the delay line; the one leaving it is what
        // we emit, so the gain has already reacted to what is coming.
        let delayed = self.buffer[self.write_position];
        self.buffer[self.write_position] = sample;
        self.write_position = (self.write_position + 1) % self.buffer.len();

        // Loudest sample still inside the lookahead window decides the target.
        // Anything smaller than the arriving sample can never be the loudest
        // again, so it is dropped; what remains decreases from the front, and
        // the front leaves once it has aged out of the window.
        let magnitude = sample.abs();
        while self
            .peaks
            .back()
            .is_some_and(|&(_, loudest)| loudest <= magnitude)
        {
            self.peaks.pop_back();
        }
        self.peaks.push_back((self.samples_seen, magnitude));
        let window = self.buffer.len() as u64;
        while self
            .peaks
            .front()
            .is_some_and(|&(index, _)| index + window <= self.samples_seen)
        {
            self.peaks.pop_front();
        }
        self.samples_seen += 1;
        let peak = self.peaks.front().map_or(0.0, |&(_, loudest)| loudest);

        let target_gain = if peak > true_peak_limit {
            true_peak_limit / peak
        } else {
            1.0
        };

        let coefficient = if target_gain < self.gain {
            self.attack_coefficient
        } else {
            self.release_coefficient
        };
        self.gain += (target_gain - self.gain) * coefficient;

        // The gain envelope is smooth, but never let a sample through the ceiling
        (delayed * self.gain).clamp(-true_peak_limit, true_peak_limit)
    }
}

/// Professional loudness normalizer using EBU R128 standard
/// This is a STATEFUL normalizer that tracks cumulative loudness over time
///
/// EBU R128 is the broadcast industry standard for loudness normalization:
/// - Target: -20 LUFS for meeting speech
/// - Used by: Netflix, YouTube, Spotify, all professional broadcast
/// - Perceptually accurate (not just simple RMS)
///
pub struct LoudnessNormalizer {
    ebur128: ebur128::EbuR128,
    limiter: TruePeakLimiter,
    gain_linear: f32,
    loudness_buffer: Vec<f32>,
    true_peak_limit: f32,
}

impl LoudnessNormalizer {
    /// Create a new EBU R128 loudness normalizer
    ///
    /// # Arguments
    /// * `channels` - Number of audio channels (1 for mono, 2 for stereo)
    /// * `sample_rate` - Sample rate in Hz (e.g., 48000)
    pub fn new(channels: u32, sample_rate: u32) -> Result<Self> {
        const TRUE_PEAK_LIMIT: f64 = -1.0;
        const ANALYZE_CHUNK_SIZE: usize = 512;

        // Integrated loudness only. Asking for TRUE_PEAK as well ran a
        // four-times oversampling interpolator over every sample to find
        // inter-sample peaks - inside the capture callback, for a number
        // nothing ever read: the ceiling is held by `TruePeakLimiter` below.
        let ebur128 = ebur128::EbuR128::new(channels, sample_rate, ebur128::Mode::I)
            .map_err(|e| anyhow::anyhow!("Failed to create EBU R128 normalizer: {}", e))?;

        let true_peak_limit = 10_f32.powf(TRUE_PEAK_LIMIT as f32 / 20.0);

        Ok(Self {
            ebur128,
            limiter: TruePeakLimiter::new(sample_rate),
            gain_linear: 1.0,
            loudness_buffer: Vec::with_capacity(ANALYZE_CHUNK_SIZE),
            true_peak_limit,
        })
    }

    /// Normalize loudness using EBU R128 standard with true peak limiting
    ///
    /// This maintains cumulative loudness measurements across all processed audio,
    /// resulting in consistent normalization that sounds natural.
    ///
    /// Target: -20 LUFS with bounded, smoothed gain
    /// Applies sample-by-sample with 10ms lookahead limiter to prevent clipping
    pub fn normalize_loudness(&mut self, samples: &[f32], additional_gain: f32) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        // Keep speech clear without chasing quiet sections into clipping. The
        // old unbounded, instantaneous gain produced thousands of 0 dBFS mic
        // samples and damaged consonants before VAD/transcription.
        const TARGET_LUFS: f64 = -20.0;
        const MIN_GAIN: f32 = 0.5;
        const MAX_GAIN: f32 = 2.0;
        const ANALYZE_CHUNK_SIZE: usize = 512;
        let additional_gain = additional_gain.clamp(0.5, 3.0);

        let mut normalized_samples = Vec::with_capacity(samples.len());

        for &sample in samples {
            // Accumulate samples for loudness analysis
            self.loudness_buffer.push(sample);

            // Analyze loudness every 512 samples
            if self.loudness_buffer.len() >= ANALYZE_CHUNK_SIZE {
                if let Err(e) = self.ebur128.add_frames_f32(&self.loudness_buffer) {
                    warn!("Failed to add frames to EBU R128: {}", e);
                } else {
                    // Update gain based on cumulative loudness
                    if let Ok(current_lufs) = self.ebur128.loudness_global() {
                        if current_lufs.is_finite() && current_lufs < 0.0 {
                            let gain_db = TARGET_LUFS - current_lufs;
                            let target_gain = 10_f32.powf(gain_db as f32 / 20.0).clamp(MIN_GAIN, MAX_GAIN);
                            let blend = if target_gain < self.gain_linear { 0.2 } else { 0.05 };
                            self.gain_linear += (target_gain - self.gain_linear) * blend;
                        }
                    }
                }
                self.loudness_buffer.clear();
            }

            // Apply gain and true peak limiting
            let amplified = sample * self.gain_linear * additional_gain;
            let limited = self.limiter.process(amplified, self.true_peak_limit);

            normalized_samples.push(limited);
        }

        normalized_samples
    }
}

#[cfg(test)]
mod loudness_normalizer_tests {
    use super::*;

    /// This chain runs inside the capture callback. Whatever it cannot finish
    /// before the device hands over the next block is audio the device drops -
    /// silently, upstream of every counter the app keeps. So it has to stay
    /// far ahead of real time, and in the unoptimized build too, because that
    /// is the build the app is developed and tested against.
    #[test]
    fn the_microphone_chain_keeps_far_ahead_of_the_device() {
        let sample_rate = 48_000u32;
        let seconds = 30;
        let samples: Vec<f32> = (0..sample_rate as usize * seconds)
            .map(|n| {
                let t = n as f32 / sample_rate as f32;
                // Speech-like: a moving formant with syllable-rate amplitude
                (t * 220.0 * std::f32::consts::TAU).sin()
                    * (0.3 + 0.3 * (t * 4.0 * std::f32::consts::TAU).sin())
            })
            .collect();

        let mut filter = HighPassFilter::new(sample_rate, 80.0);
        let mut normalizer = LoudnessNormalizer::new(1, sample_rate).expect("normalizer");
        let block = sample_rate as usize / 100; // 10 ms, as the device delivers

        // Timed a second at a time, because the cost must not grow with the
        // length of the recording either: a chain that keeps up for the first
        // minute and falls behind by the thirtieth loses the end of a session.
        let mut processed = 0usize;
        let mut per_second = Vec::with_capacity(seconds);
        for second in samples.chunks(sample_rate as usize) {
            let started = std::time::Instant::now();
            for chunk in second.chunks(block) {
                let filtered = filter.process(chunk);
                processed += normalizer.normalize_loudness(&filtered, 1.0).len();
            }
            per_second.push(started.elapsed().as_secs_f64());
        }

        let elapsed: f64 = per_second.iter().sum();
        let realtime_factor = seconds as f64 / elapsed;
        let first = per_second[0];
        let last = per_second[per_second.len() - 1];

        assert_eq!(processed, samples.len(), "the chain changed the sample count");
        println!(
            "microphone chain: {}s of audio in {:.3}s = {:.1}x real time \
             (first second {:.1} ms, last second {:.1} ms)",
            seconds,
            elapsed,
            realtime_factor,
            first * 1000.0,
            last * 1000.0
        );
        assert!(
            realtime_factor > 20.0,
            "the microphone chain runs at only {:.1}x real time; the capture \
             callback will fall behind the device and lose audio",
            realtime_factor
        );
        assert!(
            last < first * 2.0,
            "the microphone chain slows down as the recording goes on: \
             {:.1} ms for the first second, {:.1} ms for the last",
            first * 1000.0,
            last * 1000.0
        );
    }

    /// Runs a real recording through the microphone chain (high-pass filter and
    /// loudness normaliser) so its effect can be measured on its own:
    ///   CHAIN_IN=<in.wav> CHAIN_OUT=<out.wav> cargo test --lib audio_processing -- --ignored --nocapture
    #[test]
    #[ignore = "needs CHAIN_IN and CHAIN_OUT"]
    fn passes_a_recording_through_the_microphone_chain() {
        let input = std::env::var("CHAIN_IN").expect("CHAIN_IN");
        let output = std::env::var("CHAIN_OUT").expect("CHAIN_OUT");
        let (samples, sample_rate) =
            crate::diarization::dsp::read_wav(std::path::Path::new(&input)).expect("read wav");

        let mut filter = HighPassFilter::new(sample_rate, 80.0);
        let mut normalizer = LoudnessNormalizer::new(1, sample_rate).expect("normalizer");

        // The capture callback hands over small blocks, so feed it the same way
        let block = sample_rate as usize / 100; // 10 ms
        let mut processed = Vec::with_capacity(samples.len());
        for chunk in samples.chunks(block) {
            let filtered = filter.process(chunk);
            processed.extend(normalizer.normalize_loudness(&filtered, 1.0));
        }

        let mut bytes = Vec::with_capacity(44 + processed.len() * 2);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&((36 + processed.len() * 2) as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&((processed.len() * 2) as u32).to_le_bytes());
        for sample in &processed {
            bytes.extend_from_slice(&((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes());
        }
        std::fs::write(&output, bytes).expect("write wav");
        println!("wrote {} samples to {}", processed.len(), output);
    }

    /// Energy above 4 kHz, the band where clipping hash shows up.
    fn high_frequency_share(samples: &[f32]) -> f32 {
        let mut previous = 0.0f32;
        let mut difference_energy = 0.0f32;
        let mut total_energy = 0.0f32;
        for &sample in samples {
            difference_energy += (sample - previous) * (sample - previous);
            total_energy += sample * sample;
            previous = sample;
        }
        if total_energy <= f32::EPSILON {
            0.0
        } else {
            difference_energy / total_energy
        }
    }

    /// Speech with transients that overshoot the ceiling, like a plosive.
    fn signal_with_peaks(sample_rate: u32, limit: f32) -> Vec<f32> {
        (0..sample_rate as usize)
            .map(|i| {
                let t = i as f32 / sample_rate as f32;
                let tone = (2.0 * std::f32::consts::PI * 220.0 * t).sin() * limit * 0.8;
                // Four short bursts well above the ceiling
                let burst = if (i / (sample_rate as usize / 8)) % 2 == 0 { 2.2 } else { 1.0 };
                tone * burst
            })
            .collect()
    }

    #[test]
    fn limiter_holds_the_ceiling_without_clipping_the_waveform() {
        let sample_rate = 48_000;
        let limit = 10_f32.powf(-1.0 / 20.0);
        let input = signal_with_peaks(sample_rate, limit);

        let mut limiter = TruePeakLimiter::new(sample_rate);
        let output: Vec<f32> = input
            .iter()
            .map(|&sample| limiter.process(sample, limit))
            .collect();

        // Nothing may exceed the ceiling
        let peak = output.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= limit + 1e-6, "limiter let {} through a ceiling of {}", peak, limit);

        // ...and the result must stay smooth. Per-sample clipping roughly doubled
        // the high-frequency share; riding a gain envelope leaves it alone.
        let before = high_frequency_share(&input);
        let after = high_frequency_share(&output);
        assert!(
            after < before * 1.3,
            "limiting added high-frequency hash: {:.5} -> {:.5}",
            before,
            after
        );
    }

    #[test]
    fn quiet_audio_passes_through_untouched() {
        let sample_rate = 48_000;
        let limit = 10_f32.powf(-1.0 / 20.0);
        let mut limiter = TruePeakLimiter::new(sample_rate);

        let input: Vec<f32> = (0..sample_rate as usize)
            .map(|i| 0.1 * (2.0 * std::f32::consts::PI * 300.0 * i as f32 / sample_rate as f32).sin())
            .collect();
        let output: Vec<f32> = input.iter().map(|&s| limiter.process(s, limit)).collect();

        // Same waveform, only delayed by the lookahead
        let delay = (sample_rate as f32 * 3.0 / 1000.0) as usize;
        let error = input[..input.len() - delay]
            .iter()
            .zip(&output[delay..])
            .fold(0.0f32, |worst, (a, b)| worst.max((a - b).abs()));
        assert!(error < 1e-3, "quiet audio was altered by {}", error);
    }

    #[test]
    fn meeting_names_remove_concat_sensitive_apostrophes() {
        assert_eq!(sanitize_filename("O'Brien Review"), "O_Brien Review");
    }

    #[test]
    fn meeting_folders_never_reuse_an_existing_directory() {
        let root = tempfile::tempdir().unwrap();
        let first = create_meeting_folder(&root.path().to_path_buf(), "Standup").unwrap();
        let second = create_meeting_folder(&root.path().to_path_buf(), "Standup").unwrap();

        assert_ne!(first, second);
        assert!(first.is_dir());
        assert!(second.is_dir());
    }

    #[test]
    fn normalization_and_user_gain_never_exceed_true_peak_limit() {
        let mut normalizer = LoudnessNormalizer::new(1, 48_000).unwrap();
        let hot_signal = vec![0.9; 48_000];
        let normalized = normalizer.normalize_loudness(&hot_signal, 3.0);
        let peak = normalized.iter().map(|sample| sample.abs()).fold(0.0, f32::max);

        assert!(peak <= normalizer.true_peak_limit + 1e-6);
    }

    #[test]
    fn normalization_gain_stays_inside_safe_bounds() {
        let mut normalizer = LoudnessNormalizer::new(1, 48_000).unwrap();
        let quiet_signal: Vec<f32> = (0..96_000)
            .map(|sample| ((sample as f32 * 440.0 * std::f32::consts::TAU) / 48_000.0).sin() * 0.001)
            .collect();
        normalizer.normalize_loudness(&quiet_signal, 1.0);

        assert!((0.5..=2.0).contains(&normalizer.gain_linear));
    }
}

/// RNNoise-based noise suppression processor
///
/// Uses a recurrent neural network to suppress background noise while preserving speech.
/// Processes audio at 48kHz in 10ms frames (480 samples per frame).
///
/// Benefits:
/// - 10-15 dB noise reduction in typical office/home environments
/// - Preserves speech quality and intelligibility
/// - Low latency (~10ms per frame)
/// - Cross-platform (works on macOS, Windows, Linux)
pub struct NoiseSuppressionProcessor {
    denoiser: DenoiseState<'static>,
    frame_buffer: Vec<f32>,
    frame_size: usize,  // 480 samples at 48kHz = 10ms
}

impl NoiseSuppressionProcessor {
    /// Create a new noise suppression processor
    ///
    /// # Arguments
    /// * `sample_rate` - Must be 48000 Hz (RNNoise requirement)
    pub fn new(sample_rate: u32) -> Result<Self> {
        if sample_rate != 48000 {
            return Err(anyhow::anyhow!(
                "Noise suppression requires 48kHz sample rate, got {}Hz",
                sample_rate
            ));
        }

        const FRAME_SIZE: usize = DenoiseState::FRAME_SIZE;

        info!("Initializing RNNoise noise suppression (frame size: {} samples, 10ms @ 48kHz)", FRAME_SIZE);

        Ok(Self {
            denoiser: *DenoiseState::new(),
            frame_buffer: Vec::with_capacity(FRAME_SIZE * 2),
            frame_size: FRAME_SIZE,
        })
    }

    /// Apply noise suppression to audio samples
    ///
    /// Processes audio in 480-sample frames (10ms at 48kHz).
    /// Buffers partial frames for next call.
    ///
    /// CRITICAL FIX: Always returns same length as input to prevent latency accumulation
    ///
    /// # Arguments
    /// * `samples` - Input audio samples at 48kHz
    ///
    /// # Returns
    /// Noise-suppressed audio samples (SAME LENGTH as input)
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }

        // CRITICAL: Remember original input length
        let input_len = samples.len();

        // Add new samples to buffer
        self.frame_buffer.extend_from_slice(samples);

        let mut output = Vec::with_capacity(input_len);

        // Process complete frames
        while self.frame_buffer.len() >= self.frame_size {
            // Extract one frame
            let frame: Vec<f32> = self.frame_buffer.drain(0..self.frame_size).collect();

            // RNNoise processes audio: separate input and output buffers
            let mut denoised_frame = vec![0.0f32; self.frame_size];

            // Apply noise suppression
            // process_frame(output: &mut [f32], input: &[f32]) -> f32
            // Returns VAD probability (0.0-1.0), higher means more likely to be speech
            let _vad_prob = self.denoiser.process_frame(&mut denoised_frame, &frame);

            output.extend_from_slice(&denoised_frame);
        }

        // Return processed output without forcing length matching
        // Frame-based processing naturally creates variable-length output
        // Downstream pipeline handles this correctly via ring buffer
        output
    }

    /// Get the number of buffered samples waiting for processing
    pub fn buffered_samples(&self) -> usize {
        self.frame_buffer.len()
    }

    /// Flush any remaining buffered samples
    /// Call this at the end of recording to process partial frames
    pub fn flush(&mut self) -> Vec<f32> {
        if self.frame_buffer.is_empty() {
            return Vec::new();
        }

        // Pad the remaining samples to a full frame with zeros
        let remaining = self.frame_buffer.len();
        let mut input_frame = self.frame_buffer.clone();
        if input_frame.len() < self.frame_size {
            input_frame.resize(self.frame_size, 0.0);
        }

        let mut output = vec![0.0f32; self.frame_size];
        self.denoiser.process_frame(&mut output, &input_frame);
        self.frame_buffer.clear();

        // Return only the original samples (without padding)
        output.truncate(remaining);
        output
    }
}

/// High-pass filter to remove low-frequency rumble and noise
/// Removes frequencies below cutoff_hz (typically 80-100 Hz for speech)
pub struct HighPassFilter {
    #[allow(dead_code)]
    sample_rate: f32,
    #[allow(dead_code)]
    cutoff_hz: f32,
    // First-order IIR filter coefficients
    alpha: f32,
    prev_input: f32,
    prev_output: f32,
}

impl HighPassFilter {
    /// Create a new high-pass filter
    ///
    /// # Arguments
    /// * `sample_rate` - Audio sample rate in Hz
    /// * `cutoff_hz` - Cutoff frequency in Hz (typical: 80-100 Hz for speech)
    pub fn new(sample_rate: u32, cutoff_hz: f32) -> Self {
        let sample_rate_f = sample_rate as f32;
        let rc = 1.0 / (2.0 * std::f32::consts::PI * cutoff_hz);
        let dt = 1.0 / sample_rate_f;
        let alpha = rc / (rc + dt);

        info!("Initializing high-pass filter: cutoff={}Hz @ {}Hz", cutoff_hz, sample_rate);

        Self {
            sample_rate: sample_rate_f,
            cutoff_hz,
            alpha,
            prev_input: 0.0,
            prev_output: 0.0,
        }
    }

    /// Apply high-pass filter to audio samples
    /// Uses first-order IIR (Infinite Impulse Response) filter
    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let mut output = Vec::with_capacity(samples.len());

        for &sample in samples {
            // First-order high-pass IIR filter formula:
            // y[n] = alpha * (y[n-1] + x[n] - x[n-1])
            let filtered = self.alpha * (self.prev_output + sample - self.prev_input);

            self.prev_input = sample;
            self.prev_output = filtered;

            output.push(filtered);
        }

        output
    }

    /// Reset filter state (call when starting new recording)
    pub fn reset(&mut self) {
        self.prev_input = 0.0;
        self.prev_output = 0.0;
    }
}

pub fn spectral_subtraction(audio: &[f32], d: f32) -> Result<Vec<f32>> {
    let mut real_planner = RealFftPlanner::<f32>::new();
    let window_size = 1600; // 16k sample rate - 100ms

    // CRITICAL FIX: Handle cases where audio is longer than window size
    if audio.is_empty() {
        return Ok(Vec::new());
    }

    // If audio is longer than window size, truncate to prevent overflow
    let processed_audio = if audio.len() > window_size {
        warn!("Audio length {} exceeds window size {}, truncating", audio.len(), window_size);
        &audio[..window_size]
    } else {
        audio
    };

    let r2c = real_planner.plan_fft_forward(window_size);
    let mut y = r2c.make_output_vec();

    // Safe padding: only pad if audio is shorter than window size
    let mut padded_audio = processed_audio.to_vec();
    if processed_audio.len() < window_size {
        let padding_needed = window_size - processed_audio.len();
        padded_audio.extend(vec![0.0f32; padding_needed]);
    }

    let mut indata = padded_audio;
    r2c.process(&mut indata, &mut y)?;

    let mut processed_audio = y
        .iter()
        .map(|&x| {
            let magnitude_y = x.abs().powf(2.0);

            let div = 1.0 - (d / magnitude_y);

            let gain = {
                if div > 0.0 {
                    f32::sqrt(div)
                } else {
                    0.0f32
                }
            };

            x * gain
        })
        .collect::<Vec<Complex32>>();

    let c2r = real_planner.plan_fft_inverse(window_size);

    let mut outdata = c2r.make_output_vec();

    c2r.process(&mut processed_audio, &mut outdata)?;

    Ok(outdata)
}

// not an average of non-speech segments, but I don't know how much pause time we
// get. for now, we will just assume the noise is constant (kinda defeats the purpose)
// but oh well
pub fn average_noise_spectrum(audio: &[f32]) -> f32 {
    let mut total_sum = 0.0f32;

    for sample in audio {
        let magnitude = sample.abs();

        total_sum += magnitude.powf(2.0);
    }

    total_sum / audio.len() as f32
}

pub fn audio_to_mono(audio: &[f32], channels: u16) -> Vec<f32> {
    let mut mono_samples = Vec::with_capacity(audio.len() / channels as usize);

    // For microphone arrays (> 2 channels), only use first 2 channels
    // Many microphone arrays have auxiliary channels for beam-forming/noise cancellation
    // that can contain anti-phase signals. Averaging all channels can cause destructive
    // interference resulting in near-zero output.
    let effective_channels = if channels > 2 { 2 } else { channels };

    // Iterate over the audio slice in chunks, each containing `channels` samples
    for chunk in audio.chunks(channels as usize) {
        // Sum only the first effective_channels (typically 1-2 for mic arrays)
        let sum: f32 = chunk.iter().take(effective_channels as usize).sum();

        // Calculate the average mono sample using effective channel count
        let mono_sample = sum / effective_channels as f32;

        // Store the computed mono sample
        mono_samples.push(mono_sample);
    }

    mono_samples
}

/// High-quality audio resampling with adaptive parameters based on sample rate ratio
///
/// This function automatically selects the best resampling parameters based on:
/// - Sample rate ratio (upsampling vs downsampling)
/// - Quality requirements (integer ratios get optimized paths)
/// - Anti-aliasing needs
///
/// Supports all common sample rates: 8kHz, 16kHz, 24kHz, 44.1kHz, 48kHz, etc.
pub fn resample(input: &[f32], from_sample_rate: u32, to_sample_rate: u32) -> Result<Vec<f32>> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    // Fast path: No resampling needed
    if from_sample_rate == to_sample_rate {
        return Ok(input.to_vec());
    }

    let ratio = to_sample_rate as f64 / from_sample_rate as f64;

    // Adaptive parameters based on sample rate ratio
    let (sinc_len, interpolation_type, oversampling) = if ratio >= 2.0 {
        // Large upsampling (e.g., 8kHz → 16kHz, 16kHz → 48kHz, 24kHz → 48kHz)
        // Needs high quality to avoid artifacts
        debug!("High-quality upsampling: {}Hz → {}Hz (ratio: {:.2}x)",
               from_sample_rate, to_sample_rate, ratio);
        (
            512,                              // Longer sinc for smoother interpolation
            SincInterpolationType::Cubic,     // Cubic for best quality
            512,                              // Higher oversampling
        )
    } else if ratio >= 1.5 {
        // Moderate upsampling (e.g., 32kHz → 48kHz)
        debug!("Moderate upsampling: {}Hz → {}Hz (ratio: {:.2}x)",
               from_sample_rate, to_sample_rate, ratio);
        (
            384,
            SincInterpolationType::Cubic,
            384,
        )
    } else if ratio > 1.0 {
        // Small upsampling (e.g., 44.1kHz → 48kHz)
        debug!("Small upsampling: {}Hz → {}Hz (ratio: {:.2}x)",
               from_sample_rate, to_sample_rate, ratio);
        (
            256,
            SincInterpolationType::Linear,
            256,
        )
    } else if ratio <= 0.5 {
        // Large downsampling (e.g., 48kHz → 16kHz, 48kHz → 8kHz)
        // Needs strong anti-aliasing
        debug!("Anti-aliased downsampling: {}Hz → {}Hz (ratio: {:.2}x)",
               from_sample_rate, to_sample_rate, ratio);
        (
            512,                              // Longer sinc for anti-aliasing
            SincInterpolationType::Cubic,     // Cubic for quality
            512,
        )
    } else {
        // Moderate downsampling (e.g., 48kHz → 24kHz, 48kHz → 32kHz)
        debug!("Moderate downsampling: {}Hz → {}Hz (ratio: {:.2}x)",
               from_sample_rate, to_sample_rate, ratio);
        (
            384,
            SincInterpolationType::Linear,
            384,
        )
    };

    let params = SincInterpolationParameters {
        sinc_len,
        f_cutoff: 0.95,                      // Preserve most of the frequency content
        interpolation: interpolation_type,
        oversampling_factor: oversampling,
        window: WindowFunction::BlackmanHarris2,  // Best window for audio
    };

    let mut resampler = SincFixedIn::<f32>::new(
        ratio,
        2.0,  // Maximum relative deviation
        params,
        input.len(),
        1,    // Mono
    )?;

    let waves_in = vec![input.to_vec()];
    let waves_out = resampler.process(&waves_in, None)?;

    debug!("Resampling complete: {} samples → {} samples",
           input.len(), waves_out[0].len());

    Ok(waves_out.into_iter().next().unwrap())
}

// Alias for compatibility with existing code
pub fn resample_audio(input: &[f32], from_sample_rate: u32, to_sample_rate: u32) -> Vec<f32> {
    match resample(input, from_sample_rate, to_sample_rate) {
        Ok(result) => result,
        Err(e) => {
            debug!("Resampling failed: {}, returning original audio", e);
            input.to_vec()
        }
    }
}

/// Fast resampling optimized for transcription preprocessing
///
pub fn write_audio_to_file(
    audio: &[f32],
    sample_rate: u32,
    output_path: &PathBuf,
    device: &str,
    skip_encoding: bool,
) -> Result<String> {
    write_audio_to_file_with_meeting_name(audio, sample_rate, output_path, device, skip_encoding, None)
}

pub fn write_audio_to_file_with_meeting_name(
    audio: &[f32],
    sample_rate: u32,
    output_path: &PathBuf,
    device: &str,
    skip_encoding: bool,
    meeting_name: Option<&str>,
) -> Result<String> {
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
    let sanitized_device_name = device.replace(['/', '\\'], "_");

    // Create meeting folder if meeting name is provided
    let final_output_path = if let Some(name) = meeting_name {
        let sanitized_meeting_name = sanitize_filename(name);
        let meeting_folder = output_path.join(&sanitized_meeting_name);

        // Create the meeting folder if it doesn't exist
        if !meeting_folder.exists() {
            std::fs::create_dir_all(&meeting_folder)?;
        }

        meeting_folder
    } else {
        output_path.clone()
    };

    let file_path = final_output_path
        .join(format!("{}_{}.mp4", sanitized_device_name, timestamp))
        .to_str()
        .expect("Failed to create valid path")
        .to_string();
    let file_path_clone = file_path.clone();
    // Run FFmpeg in a separate task
    if !skip_encoding {
        encode_single_audio(
            bytemuck::cast_slice(audio),
            sample_rate,
            1,
            &file_path.into(),
        )?;
    }
    Ok(file_path_clone)
}

/// Write transcript text to a file alongside the recording (legacy plain text format)
pub fn write_transcript_to_file(
    transcript_text: &str,
    output_path: &PathBuf,
    meeting_name: Option<&str>,
) -> Result<String> {
    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();

    // Create meeting folder if meeting name is provided (same logic as audio)
    let final_output_path = if let Some(name) = meeting_name {
        let sanitized_meeting_name = sanitize_filename(name);
        let meeting_folder = output_path.join(&sanitized_meeting_name);

        // Create the meeting folder if it doesn't exist
        if !meeting_folder.exists() {
            std::fs::create_dir_all(&meeting_folder)?;
        }

        meeting_folder
    } else {
        output_path.clone()
    };

    let file_path = final_output_path.join(format!("transcript_{}.txt", timestamp));

    // Write transcript to file
    std::fs::write(&file_path, transcript_text)?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Write structured transcript with timestamps to JSON file
pub fn write_transcript_json_to_file(
    segments: &[super::recording_saver::TranscriptSegment],
    output_path: &PathBuf,
    meeting_name: Option<&str>,
    audio_filename: &str,
    recording_duration: f64,
) -> Result<String> {
    use serde_json::json;

    let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();

    // Create meeting folder if meeting name is provided
    let final_output_path = if let Some(name) = meeting_name {
        let sanitized_meeting_name = sanitize_filename(name);
        let meeting_folder = output_path.join(&sanitized_meeting_name);

        if !meeting_folder.exists() {
            std::fs::create_dir_all(&meeting_folder)?;
        }

        meeting_folder
    } else {
        output_path.clone()
    };

    let file_path = final_output_path.join(format!("transcript_{}.json", timestamp));

    // Create structured JSON transcript
    let transcript_json = json!({
        "version": "1.0",
        "recording_duration": recording_duration,
        "audio_file": audio_filename,
        "sample_rate": 48000,
        "created_at": Utc::now().to_rfc3339(),
        "meeting_name": meeting_name,
        "segments": segments,
    });

    // Write JSON to file with pretty formatting
    let json_string = serde_json::to_string_pretty(&transcript_json)?;
    std::fs::write(&file_path, json_string)?;

    Ok(file_path.to_string_lossy().to_string())
}
