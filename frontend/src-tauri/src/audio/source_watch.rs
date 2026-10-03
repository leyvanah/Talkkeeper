//! Keeps both capture sources alive for as long as the recording runs.
//!
//! A capture stream ends for good when its device goes away: headphones
//! unplugged, a Bluetooth headset out of range, Windows invalidating the
//! endpoint. That used to stop the whole recording. Now the stream reports the
//! source lost (`RecordingState::report_source_lost`), the other source goes
//! on recording, the mixer holds the missing one's place with silence, and
//! this watcher opens a stream on what Windows offers instead — as a rule the
//! new default device. Once it delivers, the mixer lines it up again.
//!
//! A source that was the Windows default when the recording started also
//! follows the default when it moves without anything failing: plugging
//! headphones in sends the other side's voice there, and the loopback of the
//! speakers would go on recording silence.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use log::{info, warn};
use tauri::{AppHandle, Emitter, Runtime};

use super::devices::{default_input_device, default_output_device, AudioDevice};
use super::recording_commands::{recording_lifecycle, with_recording_manager};
use super::recording_state::DeviceType;
use super::stream::AudioStream;

/// How often the sources are looked at.
const TICK: Duration = Duration::from_secs(1);
/// Longest wait between attempts to open a replacement that keeps failing.
const MAX_RETRY: Duration = Duration::from_secs(10);

/// Bumped per recording, so a watcher left from the last one stands down.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Start watching the recording that has just started.
pub fn start<R: Runtime>(app: AppHandle<R>) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    tauri::async_runtime::spawn(async move {
        let mut sources = [
            Source::new(DeviceType::Microphone),
            Source::new(DeviceType::System),
        ];
        loop {
            tokio::time::sleep(TICK).await;
            if GENERATION.load(Ordering::SeqCst) != generation {
                return;
            }
            for source in &mut sources {
                if !source.check(&app).await {
                    return;
                }
            }
        }
    });
}

struct Source {
    device_type: DeviceType,
    /// Lost, and not yet replaced.
    pending: bool,
    failures: u32,
    retry_at: Option<Instant>,
    /// A new default seen on the last look; followed once it has held.
    new_default: Option<String>,
}

impl Source {
    fn new(device_type: DeviceType) -> Self {
        Self {
            device_type,
            pending: false,
            failures: 0,
            retry_at: None,
            new_default: None,
        }
    }

    fn label(&self) -> &'static str {
        match self.device_type {
            DeviceType::Microphone => "microphone",
            _ => "system",
        }
    }

    fn default_device(&self) -> Option<AudioDevice> {
        match self.device_type {
            DeviceType::Microphone => default_input_device().ok(),
            _ => default_output_device().ok(),
        }
    }

    /// One look at the source. False once the recording is over.
    async fn check<R: Runtime>(&mut self, app: &AppHandle<R>) -> bool {
        let device_type = self.device_type.clone();
        let Some((lost, current, follows)) = with_recording_manager(|manager| {
            (
                manager.get_state().take_source_lost(&device_type),
                manager.source_device(&device_type),
                manager.follows_default(&device_type),
            )
        }) else {
            return false;
        };
        let Some(current) = current else {
            // Recorded without this source from the start; nothing to keep up.
            return true;
        };

        if lost {
            self.pending = true;
            self.failures = 0;
            self.retry_at = None;
            let _ = app.emit("recording-source-lost", self.label());
        }

        let default = self.default_device();
        let candidates: Vec<AudioDevice> = if self.pending {
            if self.retry_at.is_some_and(|at| Instant::now() < at) {
                return true;
            }
            // A device picked on purpose is tried again first, in case it is
            // already back; then whatever Windows now calls the default.
            let mut candidates = Vec::new();
            if !follows {
                candidates.push((*current).clone());
            }
            candidates.extend(default);
            candidates
        } else if follows {
            match default {
                Some(default) if default.name != current.name => {
                    // Windows can pass through a device on its way to the
                    // next; move only once the new default has held a tick.
                    if self.new_default.as_deref() != Some(default.name.as_str()) {
                        self.new_default = Some(default.name.clone());
                        return true;
                    }
                    vec![default, (*current).clone()]
                }
                _ => {
                    self.new_default = None;
                    return true;
                }
            }
        } else {
            return true;
        };

        if candidates.is_empty() {
            self.retry_later();
            return true;
        }

        match self.replace(candidates).await {
            Ok(Some(device)) => {
                info!(
                    "🔁 {} capture now from '{}'; the recording went on",
                    self.label(),
                    device
                );
                self.pending = false;
                self.failures = 0;
                self.retry_at = None;
                self.new_default = None;
                let _ = app.emit(
                    "recording-source-switched",
                    serde_json::json!({ "source": self.label(), "device": device }),
                );
            }
            Ok(None) => return false,
            Err(error) => {
                warn!(
                    "Could not open a replacement {} capture yet: {}",
                    self.label(),
                    error
                );
                // The old stream is gone by now, if it was not already.
                self.pending = true;
                self.retry_later();
            }
        }
        true
    }

    fn retry_later(&mut self) {
        self.failures += 1;
        let wait = TICK
            .saturating_mul(1 << self.failures.min(4))
            .min(MAX_RETRY);
        self.retry_at = Some(Instant::now() + wait);
    }

    /// Swap the source's stream for one on the first candidate that opens.
    /// `Ok(None)` when the recording ended meanwhile.
    async fn replace(&self, candidates: Vec<AudioDevice>) -> Result<Option<String>> {
        let _lifecycle = recording_lifecycle().await;
        let device_type = self.device_type.clone();
        let Some((old, state)) = with_recording_manager(|manager| {
            (
                manager.take_stream(&device_type),
                manager.get_state().clone(),
            )
        }) else {
            return Ok(None);
        };

        // The old stream goes first: a microphone stream carries the
        // own-speech detector, and closing an old one after the new one had
        // opened would switch the new one's detector off.
        if let Some(old) = old {
            let _ = tokio::task::spawn_blocking(move || old.stop()).await;
        }
        // Whatever the new stream delivers is later than this; blocks of the
        // old one still on their way are earlier.
        let restart_at = state.get_active_recording_duration().unwrap_or(0.0);
        state.mark_source_restarted(&device_type, restart_at);

        let mut last_error = anyhow::anyhow!("no device to open");
        for candidate in candidates {
            let device = Arc::new(candidate);
            match AudioStream::create(device.clone(), state.clone(), device_type.clone(), None)
                .await
            {
                Ok(stream) => {
                    let name = device.name.clone();
                    let placed = with_recording_manager(|manager| {
                        manager.put_stream(device_type.clone(), device, stream);
                    });
                    return Ok(placed.map(|_| name));
                }
                Err(error) => last_error = error.context(format!("'{}'", device.name)),
            }
        }
        Err(last_error)
    }
}
