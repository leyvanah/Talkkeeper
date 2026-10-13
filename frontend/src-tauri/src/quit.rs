//! Quit from the tray, without cutting a recording off.
//!
//! Quit used to be a bare `app.exit(0)`. Pressed during a recording it ended
//! the process mid-capture: the recording never became a meeting and its tail
//! could be lost. Stopping in the backend alone is not enough either - the
//! meeting is written by the main window, after Stop, from the transcript it
//! has been showing - so the process has to outlive both the stop and that
//! save.
//!
//! With nothing recorded, Quit still exits at once. With a recording on, or
//! one on pause, the window asks first: "Stop and quit" is the ordinary Stop,
//! then a wait for the window to report the meeting saved, then the exit;
//! "Cancel" leaves the recording running. Quit pressed while a stop or its
//! save is already under way waits for them and then exits.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Runtime};

use crate::audio::recording_commands;

/// Sent to the main window to ask whether to stop the recording and quit.
const ASK_EVENT: &str = "quit-requested-during-recording";

/// The longest the window's save after Stop is waited for. Its own steps are
/// bounded by about a minute and a quarter (waiting out recognition, late
/// lines, the write); past this the window is taken not to be answering, and
/// whatever it did not save is left to the recovery dialog.
const SAVE_WAIT: Duration = Duration::from_secs(180);

const POLL: Duration = Duration::from_millis(200);

/// When the main window was last handed a stopped recording to save, until it
/// reports the save settled. `None` when nothing is waiting to be saved.
static SAVE_HANDED_AT: Mutex<Option<Instant>> = Mutex::new(None);

fn handed_at() -> std::sync::MutexGuard<'static, Option<Instant>> {
    SAVE_HANDED_AT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// The main window is about to be given a stopped recording to save.
pub fn save_handed_to_window() {
    *handed_at() = Some(Instant::now());
}

/// The window has finished with the recording it was handed: saved it,
/// discarded it as too short, or failed. Also called when it could not be
/// handed over at all.
#[tauri::command]
pub fn recording_save_settled() {
    *handed_at() = None;
}

fn save_still_pending(handed: Option<Instant>, now: Instant) -> bool {
    handed.is_some_and(|at| now.saturating_duration_since(at) < SAVE_WAIT)
}

fn save_pending() -> bool {
    save_still_pending(*handed_at(), Instant::now())
}

#[derive(Debug, PartialEq, Eq)]
enum OnQuit {
    /// Nothing is being recorded or saved.
    ExitNow,
    /// A recording is on (or paused): the owner decides.
    Ask,
    /// A stop or its save is under way: let it finish, then exit.
    WaitThenExit,
}

fn on_quit(recording: bool, stopping: bool, save_pending: bool) -> OnQuit {
    if stopping {
        OnQuit::WaitThenExit
    } else if recording {
        OnQuit::Ask
    } else if save_pending {
        OnQuit::WaitThenExit
    } else {
        OnQuit::ExitNow
    }
}

/// Until the stop has ended and the window has saved what it stopped.
async fn wait_for_save() {
    let started = Instant::now();
    while recording_commands::is_stopping() || save_pending() {
        tokio::time::sleep(POLL).await;
    }
    log::info!("Quit: waited {:?} for the recording to be saved", started.elapsed());
}

/// The tray's Quit.
pub fn request_quit<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let action = on_quit(
            recording_commands::is_recording().await,
            recording_commands::is_stopping(),
            save_pending(),
        );
        log::info!("Quit requested from the tray: {:?}", action);
        match action {
            OnQuit::ExitNow => app.exit(0),
            OnQuit::Ask => {
                crate::tray::focus_main_window(&app);
                if let Err(error) = app.emit_to("main", ASK_EVENT, ()) {
                    // No window to ask in. The recording keeps going rather
                    // than being cut off; Stop in the tray still works.
                    log::error!("Quit: could not ask the main window: {}", error);
                }
            }
            OnQuit::WaitThenExit => {
                // So the owner sees the save the exit is waiting for.
                crate::tray::focus_main_window(&app);
                wait_for_save().await;
                app.exit(0);
            }
        }
    });
}

/// "Stop and quit" in the window's question: the tray's Stop, then the exit
/// once the window has saved the meeting.
#[tauri::command]
pub async fn stop_recording_and_quit<R: Runtime>(app: AppHandle<R>) -> Result<(), String> {
    log::info!("Quit: stopping the recording before exit");
    crate::tray::set_tray_state(&app, crate::tray::RecordingState::Stopping);

    // Same path as the tray's Stop; the backend does not use it any more.
    let timestamp = chrono::Local::now().format("%Y-%m-%dT%H-%M-%S").to_string();
    let save_path = crate::paths::install_data_root().join(format!("recording-{}.wav", timestamp));
    let outcome = recording_commands::stop_recording(
        app.clone(),
        recording_commands::RecordingArgs {
            save_path: save_path.to_string_lossy().to_string(),
        },
    )
    .await;

    if let Err(error) = outcome {
        // The recording may still be on: stay open rather than lose it.
        log::error!("Quit: the recording did not stop, not exiting: {}", error);
        crate::tray::update_tray_menu_async(&app).await;
        return Err(error);
    }

    wait_for_save().await;
    app.exit(0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quits_at_once_when_nothing_is_recorded_or_saved() {
        assert_eq!(on_quit(false, false, false), OnQuit::ExitNow);
    }

    #[test]
    fn asks_while_recording() {
        // Pause keeps the recording on: it is asked about the same way.
        assert_eq!(on_quit(true, false, false), OnQuit::Ask);
        assert_eq!(on_quit(true, false, true), OnQuit::Ask);
    }

    #[test]
    fn waits_for_a_stop_or_save_under_way() {
        // A recording being stopped still counts as recording until the end.
        assert_eq!(on_quit(true, true, false), OnQuit::WaitThenExit);
        assert_eq!(on_quit(false, true, false), OnQuit::WaitThenExit);
        assert_eq!(on_quit(false, false, true), OnQuit::WaitThenExit);
    }

    #[test]
    fn a_save_is_pending_until_settled_or_too_long_unanswered() {
        let now = Instant::now();
        assert!(!save_still_pending(None, now));
        assert!(save_still_pending(Some(now), now));
        assert!(save_still_pending(Some(now), now + SAVE_WAIT - Duration::from_secs(1)));
        assert!(!save_still_pending(Some(now), now + SAVE_WAIT));
    }

    #[test]
    fn settling_clears_the_pending_save() {
        save_handed_to_window();
        assert!(save_pending());
        recording_save_settled();
        assert!(!save_pending());
    }
}
