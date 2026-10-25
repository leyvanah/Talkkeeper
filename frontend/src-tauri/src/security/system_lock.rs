//! Locking the archive when Windows locks the session or goes to sleep.
//!
//! The owner's decision (tasks#116): Win+L, switching to another user and the
//! computer going to sleep all lock the archive, always - no setting. A
//! recording in progress is not interrupted: the lock is owed and paid as
//! soon as the recording, or a job holding the key, ends (see
//! `commands::pay_due_lock`).
//!
//! Windows tells a window about both: `WM_POWERBROADCAST` with
//! `PBT_APMSUSPEND` goes to every top-level window, and `WM_WTSSESSION_CHANGE`
//! to a window that registered for it. So a hidden window of our own, on a
//! thread of its own, listens for them; the app's real windows are Tauri's and
//! stay untouched.
//!
//! What this does not cover, and the idle lock does: a laptop on Modern
//! Standby may not send `PBT_APMSUSPEND` at all when its lid closes. Windows
//! then normally locks the session (a lock event here), and the idle clock
//! counts the time asleep by the wall clock as well (`session.rs`).

use tauri::{AppHandle, Manager, Runtime};

use super::commands::{pay_due_lock, SecurityState};
use super::session::LockReason;

/// Starts listening. Does nothing outside Windows.
pub fn watch<R: Runtime>(app: AppHandle<R>) {
    #[cfg(windows)]
    windows_watch::start(app);
    #[cfg(not(windows))]
    let _ = app;
}

/// Asks for the lock and pays it at once when nothing forbids it.
fn on_event<R: Runtime>(app: &AppHandle<R>, reason: LockReason) {
    let Some(state) = app.try_state::<SecurityState>() else {
        return;
    };
    if !state.session.is_unlocked() {
        return;
    }
    state.session.request_lock(reason);
    let recording = crate::audio::recording_commands::is_recording_active();
    if !pay_due_lock(app, recording) {
        log::info!(
            "Archive lock owed ({reason:?}): a recording or a job is running; it locks when they end"
        );
    }
}

#[cfg(windows)]
mod windows_watch {
    use std::sync::OnceLock;

    use tauri::{AppHandle, Runtime};
    use windows::core::w;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::RemoteDesktop::{
        WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        TranslateMessage, MSG, PBT_APMSUSPEND, WINDOW_EX_STYLE, WM_POWERBROADCAST,
        WM_WTSSESSION_CHANGE, WNDCLASSW, WS_OVERLAPPED, WTS_CONSOLE_DISCONNECT,
        WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK,
    };

    use super::LockReason;

    /// The window procedure has no way to carry the app handle, so the
    /// reaction is kept here, set once.
    static REACT: OnceLock<Box<dyn Fn(LockReason) + Send + Sync>> = OnceLock::new();

    pub fn start<R: Runtime>(app: AppHandle<R>) {
        if REACT.set(Box::new(move |reason| super::on_event(&app, reason))).is_err() {
            return;
        }
        let started = std::thread::Builder::new()
            .name("system-lock-watch".into())
            .spawn(|| {
                if let Err(error) = unsafe { listen() } {
                    log::warn!("Not watching for sleep and session lock: {error}");
                }
            });
        if let Err(error) = started {
            log::warn!("Could not start watching for sleep and session lock: {error}");
        }
    }

    unsafe fn listen() -> windows::core::Result<()> {
        let instance = GetModuleHandleW(None)?;
        let class = w!("TalkkeeperSystemLockWatch");
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(on_message),
            hInstance: instance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        // A top-level window that is never shown: top-level, because the
        // power broadcast is not sent to message-only windows.
        let window = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!(""),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        // Without session notifications the window still hears the power
        // broadcast, so sleep goes on locking: worth keeping.
        match WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION) {
            Ok(()) => log::info!("Watching for sleep and Windows session lock"),
            Err(error) => log::warn!(
                "Watching for sleep only; Windows session lock is not reported: {error}"
            ),
        }

        let mut message = MSG::default();
        // 0 is WM_QUIT and -1 an error; either ends the loop.
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Ok(())
    }

    unsafe extern "system" fn on_message(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        let reason = match message {
            WM_POWERBROADCAST if wparam.0 as u32 == PBT_APMSUSPEND => Some(LockReason::Sleep),
            WM_WTSSESSION_CHANGE
                if matches!(
                    wparam.0 as u32,
                    WTS_SESSION_LOCK | WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT
                ) =>
            {
                Some(LockReason::SessionLocked)
            }
            _ => None,
        };
        if let (Some(reason), Some(react)) = (reason, REACT.get()) {
            react(reason);
        }
        if message == WM_POWERBROADCAST {
            // TRUE: the suspend may go ahead.
            return LRESULT(1);
        }
        DefWindowProcW(window, message, wparam, lparam)
    }
}
