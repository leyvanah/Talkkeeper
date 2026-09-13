//! Capturing the microphone the way a voice call does, so Windows removes the
//! echo before the samples reach us.
//!
//! ## Why this exists
//!
//! Without headphones the other person's voice comes out of the speakers and
//! back into the microphone. The application then hears it, transcribes it, and
//! attributes it to the owner. We spent a long time trying to subtract that
//! echo ourselves and it never worked well, for a reason that is structural
//! rather than a bug: to cancel an echo you need the signal that was played and
//! the exact delay before it came back. An application has neither. It can
//! capture what the system is playing, but that copy arrives *before* it has
//! been through the speakers, the amplifier and the room, and how much before
//! is a moving target.
//!
//! Windows has both, and hands the result to any application that asks in the
//! right way. A browser asks — that is what `echoCancellation: true` does — and
//! that is why nobody hears themselves echoed back on a video call, headphones
//! or not. The ask is a stream opened in the communications category: Windows
//! then runs the driver's own canceller, which sits close enough to the
//! hardware to know the delay exactly.
//!
//! ## What was measured before writing this
//!
//! On the owner's machine, speakers playing speech while he talked over them:
//!
//! | | ordinary capture | communications capture |
//! |---|---|---|
//! | correlation with what was playing | 0.094 | **0.000** |
//! | his own voice (energy) | 23.8 | 1684.3 |
//!
//! The far end is gone, and his voice is louder rather than quieter — the same
//! mode turns on automatic gain. Confirmed by ear on the recordings as well.
//!
//! ## What it costs
//!
//! Windows also applies noise suppression and automatic gain in this mode. The
//! recording is cleaner and more even, but less faithful to the room: a quiet
//! moment is pushed to digital silence rather than kept as quiet. For speech
//! that is a gain; for anything where the original dynamics matter it is not,
//! so this is a setting rather than a decision made for the owner.
//!
//! Windows only. Everywhere else the existing capture path is unchanged.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use anyhow::{anyhow, Result};
use tracing::{info, warn};
use windows::core::Interface;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, AudioCategory_Communications, AudioClientProperties,
    IAudioCaptureClient, IAudioClient, IAudioClient2, IMMDeviceEnumerator, MMDeviceEnumerator,
    AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_SHAREMODE_SHARED, WAVEFORMATEXTENSIBLE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED,
};

/// `WAVE_FORMAT_IEEE_FLOAT`, as it appears in a format tag or a subformat GUID.
const FORMAT_FLOAT: u16 = 0x0003;
/// `WAVE_FORMAT_EXTENSIBLE`: the real format is in the subformat GUID.
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// How long to sleep between polls.
///
/// Event-driven capture is the other option and it is a trap here: in shared
/// mode it wants the buffer duration left at zero, and asking for both a
/// duration *and* events is accepted by Windows and then delivers empty
/// packets — which looks exactly like a dead microphone. Polling every 10ms
/// costs nothing next to a recording pipeline and cannot fail that way.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// A microphone stream Windows has already cleaned.
pub struct CommsCapture {
    running: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    /// The rate Windows chose. The caller has to know: it is not guaranteed to
    /// be the rate the ordinary path would have used.
    sample_rate: u32,
}

impl CommsCapture {
    /// Starts capture, calling `on_samples` with mono f32 as it arrives.
    ///
    /// Returns an error rather than degrading if the mode is unavailable: the
    /// caller falls back to the ordinary capture path, and a silent downgrade
    /// would leave the owner with echo and no idea why.
    pub fn start<F>(on_samples: F) -> Result<Self>
    where
        F: FnMut(&[f32]) + Send + 'static,
    {
        // The rate is needed before the worker starts, so the device is opened
        // here and the handle handed over.
        let (tx, rx) = std::sync::mpsc::channel();
        let running = Arc::new(AtomicBool::new(true));
        let running_for_worker = running.clone();

        let worker = std::thread::Builder::new()
            .name("wasapi-comms-capture".into())
            .spawn(move || {
                // COM has to be initialised on the thread that uses it, which
                // is why the whole session lives in this worker rather than
                // being opened outside and moved in.
                let outcome = unsafe { run(running_for_worker, tx, on_samples) };
                if let Err(error) = outcome {
                    warn!("Communications capture stopped: {error}");
                }
            })?;

        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(sample_rate)) => {
                info!("🎙️ Microphone opened in communications mode at {sample_rate} Hz — Windows is cancelling the echo");
                Ok(Self {
                    running,
                    worker: Some(worker),
                    sample_rate,
                })
            }
            Ok(Err(error)) => {
                running.store(false, Ordering::Relaxed);
                let _ = worker.join();
                Err(error)
            }
            Err(_) => {
                running.store(false, Ordering::Relaxed);
                Err(anyhow!("communications capture did not start in time"))
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for CommsCapture {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The capture session. Runs until `running` goes false.
unsafe fn run<F>(
    running: Arc<AtomicBool>,
    started: std::sync::mpsc::Sender<Result<u32>>,
    mut on_samples: F,
) -> Result<()>
where
    F: FnMut(&[f32]) + Send + 'static,
{
    // Already-initialised is not an error: something else in the process may
    // have got here first, and this thread is still free to use COM.
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    let open = || -> Result<(IAudioClient, IAudioCaptureClient, Format)> {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        // The communications *role* matters as much as the category: Windows
        // lets the owner pick a different device for calls, and that is the one
        // a call would use.
        let device = enumerator.GetDefaultAudioEndpoint(eCapture, eCommunications)?;
        let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;

        // This is the whole point of the module.
        let client2: IAudioClient2 = client.cast()?;
        let properties = AudioClientProperties {
            cbSize: std::mem::size_of::<AudioClientProperties>() as u32,
            bIsOffload: false.into(),
            eCategory: AudioCategory_Communications,
            Options: Default::default(),
        };
        client2.SetClientProperties(&properties)?;

        let format_ptr = client.GetMixFormat()?;
        let format = Format::read(format_ptr);
        client.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            0,
            10_000_000, // a second, in 100ns units
            0,
            format_ptr,
            None,
        )?;
        CoTaskMemFree(Some(format_ptr as *const _));

        let capture: IAudioCaptureClient = client.GetService()?;
        client.Start()?;
        Ok((client, capture, format))
    };

    let (client, capture, format) = match open() {
        Ok(opened) => opened,
        Err(error) => {
            let _ = started.send(Err(error));
            return Ok(());
        }
    };
    let _ = started.send(Ok(format.sample_rate));

    let mut mono: Vec<f32> = Vec::with_capacity(format.sample_rate as usize / 10);
    while running.load(Ordering::Relaxed) {
        std::thread::sleep(POLL_INTERVAL);

        while capture.GetNextPacketSize().unwrap_or(0) > 0 {
            let mut frames = 0u32;
            let mut data = std::ptr::null_mut();
            let mut flags = 0u32;
            if capture
                .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                .is_err()
                || frames == 0
            {
                break;
            }

            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
            mono.clear();
            mono.reserve(frames as usize);
            for frame in 0..frames as usize {
                mono.push(if silent {
                    0.0
                } else {
                    format.frame_to_mono(data, frame)
                });
            }
            let _ = capture.ReleaseBuffer(frames);

            if !mono.is_empty() {
                on_samples(&mono);
            }
        }
    }

    client.Stop().ok();
    info!("🎙️ Communications capture stopped");
    Ok(())
}

/// What Windows chose to hand us, so the samples can be read back.
#[derive(Clone, Copy)]
struct Format {
    sample_rate: u32,
    channels: usize,
    bits: u16,
    float: bool,
}

impl Format {
    unsafe fn read(ptr: *const windows::Win32::Media::Audio::WAVEFORMATEX) -> Self {
        let format = &*ptr;
        let tag = if format.wFormatTag == FORMAT_EXTENSIBLE {
            let extended = &*(ptr as *const WAVEFORMATEXTENSIBLE);
            extended.SubFormat.data1 as u16
        } else {
            format.wFormatTag
        };
        Self {
            sample_rate: format.nSamplesPerSec,
            channels: format.nChannels.max(1) as usize,
            bits: format.wBitsPerSample,
            float: tag == FORMAT_FLOAT,
        }
    }

    /// One frame, mixed down to mono.
    ///
    /// Averaged rather than taking channel zero: a microphone array can leave
    /// one side near-empty, and reading only the first channel then looks like
    /// a dead device — which cost an afternoon of debugging to find out.
    unsafe fn frame_to_mono(&self, data: *const u8, frame: usize) -> f32 {
        let mut sum = 0.0f32;
        for channel in 0..self.channels {
            let at = frame * self.channels + channel;
            sum += if self.float && self.bits == 32 {
                *(data as *const f32).add(at)
            } else if self.bits == 16 {
                *(data as *const i16).add(at) as f32 / 32768.0
            } else {
                0.0
            };
        }
        sum / self.channels as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_averaged_across_channels() {
        // Guards the mixdown that a first-channel-only read got wrong.
        let format = Format {
            sample_rate: 48_000,
            channels: 2,
            bits: 32,
            float: true,
        };
        let samples: [f32; 4] = [0.0, 1.0, 0.5, 0.5];
        let mixed = unsafe { format.frame_to_mono(samples.as_ptr() as *const u8, 0) };
        assert!((mixed - 0.5).abs() < 1e-6, "got {mixed}");
        let second = unsafe { format.frame_to_mono(samples.as_ptr() as *const u8, 1) };
        assert!((second - 0.5).abs() < 1e-6, "got {second}");
    }

    #[test]
    fn sixteen_bit_frames_are_scaled_to_unit_range() {
        let format = Format {
            sample_rate: 48_000,
            channels: 1,
            bits: 16,
            float: false,
        };
        let samples: [i16; 2] = [16384, -32768];
        let first = unsafe { format.frame_to_mono(samples.as_ptr() as *const u8, 0) };
        let second = unsafe { format.frame_to_mono(samples.as_ptr() as *const u8, 1) };
        assert!((first - 0.5).abs() < 1e-4, "got {first}");
        assert!((second + 1.0).abs() < 1e-4, "got {second}");
    }
}
