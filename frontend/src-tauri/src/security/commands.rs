//! What the window can ask about the lock.
//!
//! Errors carry a machine-readable `code` so the interface can say the right
//! thing in the owner's language; the `message` is a fallback for the log.
//! Passwords cross this boundary and nowhere else — none of these commands
//! returns one, and none of them logs one.

use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use super::keystore::{Keystore, KeystoreError, DEFAULT_AUTO_LOCK_MINUTES};
use super::recovery::RecoveryError;
use super::session::{KeySession, LockState, IDLE_TICK};

/// Event the window listens for when the key is dropped without it asking —
/// idle timeout, or a lock from the tray.
pub const LOCKED_EVENT: &str = "archive-locked";

/// Everything the lock owns at runtime.
pub struct SecurityState {
    /// The keystore as last read or written. `None` until startup reads it, and
    /// when no password has been set.
    keystore: Mutex<Option<Keystore>>,
    /// The key itself.
    pub session: KeySession,
}

impl Default for SecurityState {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityState {
    pub fn new() -> Self {
        Self {
            keystore: Mutex::new(None),
            session: KeySession::new(),
        }
    }

    /// Reads the keystore from disk and tells the session whether a password
    /// exists. Called once at startup; a damaged file is reported, not ignored.
    pub fn load(&self) -> Result<(), SecurityError> {
        let keystore = Keystore::load()?;
        self.session.set_configured(keystore.is_some());
        self.session.set_auto_lock_after(
            keystore
                .as_ref()
                .and_then(|store| store.auto_lock_minutes)
                .map(|minutes| Duration::from_secs(minutes * 60)),
        );
        *self.keystore_guard() = keystore;
        Ok(())
    }

    fn keystore_guard(&self) -> std::sync::MutexGuard<'_, Option<Keystore>> {
        self.keystore
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Runs `change` against the keystore and writes the result back.
    ///
    /// The keystore is saved even when `change` fails, because a failed unlock
    /// still moves the attempt counter, and that counter is worthless if it does
    /// not survive closing the window.
    fn with_keystore<T>(
        &self,
        change: impl FnOnce(&mut Keystore) -> Result<T, KeystoreError>,
    ) -> Result<T, SecurityError> {
        let mut guard = self.keystore_guard();
        let keystore = guard.as_mut().ok_or(SecurityError::not_configured())?;

        let outcome = change(keystore);
        if let Err(error) = keystore.save() {
            log::error!("Could not write the keystore: {error}");
        }
        outcome.map_err(SecurityError::from)
    }
}

/// What the window shows and offers.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    pub state: LockState,
    /// Whether a recovery code exists, so the lock screen knows to offer it.
    pub has_recovery: bool,
    /// Seconds the owner must wait before the next attempt is accepted.
    pub wait_seconds: i64,
    /// Idle minutes before locking; `null` means never.
    pub auto_lock_minutes: Option<u64>,
    /// Consecutive failed attempts, so the screen can warn before the wait bites.
    pub failed_attempts: u32,
}

/// A failure the window has to react to differently depending on which it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityError {
    /// Stable identifier the interface branches on.
    pub code: String,
    /// English text for logs; the window renders its own by `code`.
    pub message: String,
    /// Set only for `tooManyAttempts`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<i64>,
    /// Set only for `passwordTooShort`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<usize>,
}

impl SecurityError {
    fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            wait_seconds: None,
            minimum: None,
        }
    }

    fn not_configured() -> Self {
        Self::new("notConfigured", "the archive is not protected by a password yet")
    }
}

impl From<KeystoreError> for SecurityError {
    fn from(error: KeystoreError) -> Self {
        let message = error.to_string();
        match error {
            KeystoreError::NotConfigured => Self::not_configured(),
            KeystoreError::AlreadyConfigured => Self::new("alreadyConfigured", message),
            KeystoreError::WrongPassword => Self::new("wrongPassword", message),
            KeystoreError::NoRecoveryCode => Self::new("noRecoveryCode", message),
            KeystoreError::WrongRecoveryCode => Self::new("wrongRecoveryCode", message),
            KeystoreError::TooManyAttempts { seconds } => Self {
                wait_seconds: Some(seconds),
                ..Self::new("tooManyAttempts", message)
            },
            KeystoreError::PasswordTooShort { minimum } => Self {
                minimum: Some(minimum),
                ..Self::new("passwordTooShort", message)
            },
            KeystoreError::RecoveryFormat(inner) => match inner {
                RecoveryError::ChecksumMismatch => Self::new("recoveryTypo", message),
                RecoveryError::WrongLength { .. } => Self::new("recoveryWrongLength", message),
                RecoveryError::BadCharacter(_) => Self::new("recoveryBadCharacter", message),
            },
            KeystoreError::Corrupt(_) => Self::new("keystoreCorrupt", message),
            KeystoreError::Io(_) => Self::new("keystoreIo", message),
        }
    }
}

impl From<super::envelope::EnvelopeError> for SecurityError {
    fn from(error: super::envelope::EnvelopeError) -> Self {
        Self::new("keystoreCorrupt", error.to_string())
    }
}

/// The recovery code, returned exactly once at the moment it is created.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodeResponse {
    pub recovery_code: Option<String>,
}

/// Whether the archive is locked, and what the lock screen may offer.
#[tauri::command]
pub fn security_status(state: State<'_, SecurityState>) -> SecurityStatus {
    let guard = state.keystore_guard();
    SecurityStatus {
        state: state.session.state(),
        has_recovery: guard.as_ref().is_some_and(Keystore::has_recovery),
        wait_seconds: guard.as_ref().map_or(0, Keystore::wait_required),
        auto_lock_minutes: guard.as_ref().and_then(|store| store.auto_lock_minutes),
        failed_attempts: guard.as_ref().map_or(0, |store| store.failed_attempts),
    }
}

/// Protects the archive for the first time and leaves it unlocked.
///
/// The recovery code comes back once, here. It is never stored in the clear and
/// cannot be asked for again — only replaced.
#[tauri::command]
pub fn security_setup(
    password: String,
    with_recovery: bool,
    state: State<'_, SecurityState>,
) -> Result<RecoveryCodeResponse, SecurityError> {
    if Keystore::exists() {
        return Err(SecurityError::new(
            "alreadyConfigured",
            "the archive is already protected by a password",
        ));
    }

    let (keystore, code, dek) = Keystore::create(&password, with_recovery)?;
    keystore.save()?;

    state.session.unlock(dek);
    state.session.set_auto_lock_after(
        keystore
            .auto_lock_minutes
            .map(|minutes| Duration::from_secs(minutes * 60)),
    );
    *state.keystore_guard() = Some(keystore);

    log::info!(
        "Archive protected with a password (recovery code kept: {})",
        with_recovery
    );

    Ok(RecoveryCodeResponse {
        recovery_code: code.map(|code| code.to_string()),
    })
}

/// Opens the archive with the password.
#[tauri::command]
pub fn security_unlock(
    password: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    let dek = state.with_keystore(|keystore| keystore.unlock_with_password(&password))?;
    state.session.unlock(dek);
    log::info!("Archive unlocked");
    Ok(())
}

/// Opens the archive with the recovery code, for an owner who forgot the
/// password. They are expected to set a new one straight afterwards.
#[tauri::command]
pub fn security_unlock_with_recovery(
    code: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    let dek = state.with_keystore(|keystore| keystore.unlock_with_recovery(&code))?;
    state.session.unlock(dek);
    log::info!("Archive unlocked with the recovery code");
    Ok(())
}

/// Drops the key. Refused while recording: it would end the session.
#[tauri::command]
pub async fn security_lock<R: Runtime>(app: AppHandle<R>) -> Result<(), SecurityError> {
    if crate::audio::recording_commands::is_recording().await {
        return Err(SecurityError::new(
            "recordingInProgress",
            "the archive cannot be locked while recording",
        ));
    }

    let state = app.state::<SecurityState>();
    state.session.lock();
    log::info!("Archive locked");
    Ok(())
}

/// Tells the lock the owner is still there, so the idle clock restarts.
#[tauri::command]
pub fn security_touch(state: State<'_, SecurityState>) {
    state.session.touch();
}

/// Swaps the password. The recovery code keeps working.
#[tauri::command]
pub fn security_change_password(
    current_password: String,
    new_password: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    state.with_keystore(|keystore| keystore.change_password(&current_password, &new_password))?;
    log::info!("Archive password changed");
    Ok(())
}

/// Sets a new password from the recovery code and opens the archive with it.
#[tauri::command]
pub fn security_reset_password(
    recovery_code: String,
    new_password: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    state.with_keystore(|keystore| {
        keystore.reset_password_with_recovery(&recovery_code, &new_password)
    })?;
    let dek = state.with_keystore(|keystore| keystore.unlock_with_password(&new_password))?;
    state.session.unlock(dek);
    log::info!("Archive password reset with the recovery code");
    Ok(())
}

/// Issues a fresh recovery code and retires the previous one.
#[tauri::command]
pub fn security_regenerate_recovery(
    password: String,
    state: State<'_, SecurityState>,
) -> Result<RecoveryCodeResponse, SecurityError> {
    let code = state.with_keystore(|keystore| keystore.regenerate_recovery(&password))?;
    log::info!("New recovery code issued");
    Ok(RecoveryCodeResponse {
        recovery_code: Some(code.to_string()),
    })
}

/// Drops the recovery envelope, leaving the password as the only way in.
#[tauri::command]
pub fn security_remove_recovery(
    password: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    state.with_keystore(|keystore| keystore.remove_recovery(&password))?;
    log::info!("Recovery code removed; the password is now the only way in");
    Ok(())
}

/// Sets the idle timeout, in minutes. `None` never locks on its own.
#[tauri::command]
pub fn security_set_auto_lock(
    minutes: Option<u64>,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    state.with_keystore(|keystore| {
        keystore.auto_lock_minutes = minutes;
        Ok(())
    })?;
    state
        .session
        .set_auto_lock_after(minutes.map(|minutes| Duration::from_secs(minutes * 60)));
    Ok(())
}

/// Removes the password entirely, after proving it is known.
///
/// In B2 this only deletes keys; from B3 onward it would have to decrypt the
/// archive first, and this command will grow that step rather than being
/// allowed to strand data behind a key it just deleted.
#[tauri::command]
pub fn security_disable(
    password: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    state.with_keystore(|keystore| keystore.unlock_with_password(&password).map(|_| ()))?;

    std::fs::remove_file(Keystore::path()).map_err(KeystoreError::Io)?;
    *state.keystore_guard() = None;
    state.session.set_configured(false);
    log::info!("Password protection removed");
    Ok(())
}

/// Watches for the archive sitting open with nobody there.
///
/// Never locks during a recording — the key is what a recording writes with, and
/// dropping it mid-session would cost the session. The check simply waits; once
/// the recording stops, the next tick locks if the idle time has passed.
pub fn start_idle_locker<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(IDLE_TICK).await;

            let Some(state) = app.try_state::<SecurityState>() else {
                continue;
            };
            if !state.session.idle_expired() {
                continue;
            }
            if crate::audio::recording_commands::is_recording().await {
                // Deliberately not locking, and deliberately not resetting the
                // idle clock either: the moment recording stops, this locks.
                continue;
            }

            state.session.lock();
            log::info!("Archive locked after idle timeout");
            if let Err(error) = app.emit(LOCKED_EVENT, ()) {
                log::warn!("Could not tell the window the archive locked: {error}");
            }
        }
    });
}

/// Reads the keystore at startup and reports whether the window should open on
/// the lock screen. A damaged keystore is surfaced rather than swallowed.
pub fn initialize(app: &AppHandle<impl Runtime>) {
    let state = app.state::<SecurityState>();
    match state.load() {
        Ok(()) => log::info!(
            "Archive protection: {:?}",
            state.session.state()
        ),
        Err(error) => log::error!("Could not read the keystore: {}", error.message),
    }

    if DEFAULT_AUTO_LOCK_MINUTES.is_some() {
        start_idle_locker(app.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wrong_password_reaches_the_window_as_its_own_code() {
        let error: SecurityError = KeystoreError::WrongPassword.into();
        assert_eq!(error.code, "wrongPassword");
        assert!(error.wait_seconds.is_none());
    }

    #[test]
    fn a_throttled_attempt_carries_the_wait_the_window_has_to_show() {
        let error: SecurityError = KeystoreError::TooManyAttempts { seconds: 40 }.into();
        assert_eq!(error.code, "tooManyAttempts");
        assert_eq!(error.wait_seconds, Some(40));
    }

    #[test]
    fn a_mistyped_recovery_code_is_a_different_code_from_a_wrong_one() {
        // The window says "check what you typed" for one and "this code does not
        // open this archive" for the other; they must not collapse into each other.
        let typo: SecurityError =
            KeystoreError::RecoveryFormat(RecoveryError::ChecksumMismatch).into();
        let wrong: SecurityError = KeystoreError::WrongRecoveryCode.into();
        assert_eq!(typo.code, "recoveryTypo");
        assert_eq!(wrong.code, "wrongRecoveryCode");
    }

    #[test]
    fn a_short_password_carries_the_minimum_so_the_window_need_not_hardcode_it() {
        let error: SecurityError = KeystoreError::PasswordTooShort { minimum: 8 }.into();
        assert_eq!(error.code, "passwordTooShort");
        assert_eq!(error.minimum, Some(8));
    }

    #[test]
    fn no_password_reaches_the_window_inside_an_error() {
        // Every message here is built from the error type, never from input.
        let error: SecurityError = KeystoreError::WrongPassword.into();
        assert!(!error.message.contains("horse"));
        let serialized = serde_json::to_string(&error).unwrap();
        assert!(serialized.contains("wrongPassword"));
    }

    #[test]
    fn a_state_with_no_keystore_answers_that_nothing_is_configured() {
        let state = SecurityState::new();
        let error = state.with_keystore(|_| Ok(())).unwrap_err();
        assert_eq!(error.code, "notConfigured");
    }
}
