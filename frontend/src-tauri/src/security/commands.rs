//! What the window can ask about the lock.
//!
//! Errors carry a machine-readable `code` so the interface can say the right
//! thing in the owner's language; the `message` is a fallback for the log.
//! Passwords cross this boundary and nowhere else — none of these commands
//! returns one, and none of them logs one.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use super::keystore::{Keystore, KeystoreError, DEFAULT_AUTO_LOCK_MINUTES};
use super::recovery::RecoveryError;
use super::session::{KeySession, LockState, IDLE_TICK};
use crate::audio::archive_encryption::{ConversionReport, Direction};

/// Event the window listens for when the key is dropped without it asking —
/// idle timeout, or a lock from the tray.
pub const LOCKED_EVENT: &str = "archive-locked";

/// Progress while recordings on disk are being converted: `{done, total}`.
/// An archive of a year's sessions is gigabytes, and a window with no sign of
/// life is a window the owner force-quits halfway through.
pub const CONVERSION_EVENT: &str = "archive-conversion";

/// Everything the lock owns at runtime.
pub struct SecurityState {
    /// The keystore as last read or written. `None` until startup reads it, and
    /// when no password has been set.
    keystore: Mutex<Option<Keystore>>,
    /// Where that keystore is read from and written to. A field rather than a
    /// constant so the lifecycle can be tested against a temporary directory
    /// instead of the owner's real archive.
    path: std::path::PathBuf,
    /// The key itself.
    pub session: Arc<KeySession>,
}

impl Default for SecurityState {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityState {
    pub fn new() -> Self {
        let state = Self::at(Keystore::path());
        // Only the real state publishes itself: a test built on a temporary
        // keystore must not become the session the audio pipeline encrypts with.
        super::session::install(state.session.clone());
        state
    }

    /// A state backed by a keystore at an explicit path.
    pub fn at(path: std::path::PathBuf) -> Self {
        Self {
            keystore: Mutex::new(None),
            path,
            session: Arc::new(KeySession::new()),
        }
    }

    /// Reads the keystore from disk and tells the session whether a password
    /// exists. Called once at startup; a damaged file is reported, not ignored.
    pub fn load(&self) -> Result<(), SecurityError> {
        let keystore = Keystore::load_from(&self.path)?;
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
        if let Err(error) = keystore.save_to(&self.path) {
            log::error!("Could not write the keystore: {error}");
        }
        outcome.map_err(SecurityError::from)
    }

    /// Protects a fresh archive and leaves it open. Shared by the command and
    /// its tests so both take the same path through the keystore.
    fn create(
        &self,
        password: &str,
        with_recovery: bool,
    ) -> Result<Option<String>, SecurityError> {
        if self.path.exists() {
            return Err(SecurityError::new(
                "alreadyConfigured",
                "the archive is already protected by a password",
            ));
        }

        let (keystore, code, dek) = Keystore::create(password, with_recovery)?;
        keystore.save_to(&self.path)?;

        self.session.unlock(dek);
        self.session.set_auto_lock_after(
            keystore
                .auto_lock_minutes
                .map(|minutes| Duration::from_secs(minutes * 60)),
        );
        *self.keystore_guard() = Some(keystore);

        Ok(code.map(|code| code.to_string()))
    }

    /// Writes a copy of the keystore to `target`.
    ///
    /// What travels is the password envelope (and the recovery one, when there
    /// is one): still sealed, still argon2id-slow to guess at. The quick-unlock
    /// envelope stays behind on purpose — its secret is sealed by DPAPI to this
    /// Windows account and is dead weight anywhere else.
    fn export_key_backup(&self, target: &std::path::Path) -> Result<(), SecurityError> {
        let Some(mut keystore) = Keystore::load_from(&self.path)? else {
            return Err(SecurityError::not_configured());
        };

        #[cfg(windows)]
        {
            keystore.quick = None;
        }

        keystore.save_to(target)?;
        Ok(())
    }
}

/// Run one pass over the recordings on disk, off the UI thread.
///
/// Returns `None` when the archive is locked: converting needs the key, and
/// there is nothing to say about an archive that cannot be opened.
async fn convert_archive<R: Runtime>(
    app: &AppHandle<R>,
    direction: Direction,
) -> Option<ConversionReport> {
    let roots = crate::audio::recording_preferences::recording_roots(app).await;
    let reporter = app.clone();
    let task = tauri::async_runtime::spawn_blocking(move || {
        super::session::with_current_key(|key| {
            crate::audio::archive_encryption::convert_all(&roots, key, direction, |done, total| {
                let _ = reporter.emit(
                    CONVERSION_EVENT,
                    serde_json::json!({ "done": done, "total": total }),
                );
            })
        })
    });
    match task.await {
        Ok(report) => report,
        Err(error) => {
            log::error!("The archive conversion task failed: {error}");
            None
        }
    }
}

/// How many recordings are encrypted and how many are not.
///
/// The settings screen uses this to tell the owner the truth about an archive
/// that is only partly converted, which is the state a crash or a restored
/// backup leaves behind.
#[tauri::command]
pub async fn security_recording_encryption<R: Runtime>(
    app: AppHandle<R>,
) -> Result<RecordingEncryption, SecurityError> {
    let roots = crate::audio::recording_preferences::recording_roots(&app).await;
    let counted = tauri::async_runtime::spawn_blocking(move || {
        crate::audio::archive_encryption::count_recordings(&roots)
    })
    .await
    .map_err(|error| SecurityError::new("countFailed", error.to_string()))?;

    Ok(RecordingEncryption {
        encrypted: counted.encrypted,
        plaintext: counted.plaintext,
    })
}

/// Bring recordings that are still plaintext under the key.
///
/// Deliberately something the owner asks for rather than something that
/// happens on its own: it rewrites files that hold sessions which cannot be
/// recorded again, and the owner is the one who decides when a backup exists.
/// Each file is verified before it replaces the original — see
/// [`crate::audio::archive_encryption`].
#[tauri::command]
pub async fn security_encrypt_recordings<R: Runtime>(
    app: AppHandle<R>,
) -> Result<RecordingEncryption, SecurityError> {
    let report = convert_archive(&app, Direction::Encrypt)
        .await
        .ok_or_else(|| SecurityError::new("locked", "the archive is locked"))?;

    if !report.is_complete() {
        log::error!(
            "{} recordings could not be encrypted; they are unchanged",
            report.failed.len()
        );
    }
    security_recording_encryption(app).await
}

/// Where the copy of the database is written before B4 first converts it.
///
/// Kept next to the database rather than somewhere tidy, so an owner who
/// needs it finds it in the folder they are already looking at.
fn database_backup_path() -> std::path::PathBuf {
    crate::paths::install_data_root().join("meeting_minutes.before-encryption.sqlite")
}

/// How many database values are sealed and how many are not.
#[tauri::command]
pub async fn security_field_encryption(
    state: State<'_, crate::state::AppState>,
) -> Result<FieldEncryption, SecurityError> {
    let counts = crate::database::field_encryption::count(state.db_manager.pool())
        .await
        .map_err(|error| SecurityError::new("countFailed", error.to_string()))?;
    Ok(FieldEncryption {
        sealed: counts.sealed,
        plaintext: counts.plaintext,
    })
}

/// Bring the titles, transcripts, names and summaries already in the
/// database under the key.
///
/// A copy of the database is written first, once, and never overwritten.
/// The conversion itself is one transaction, so a failure leaves the archive
/// exactly as it was — but a copy costs a few megabytes and answers the
/// question the owner would otherwise have to trust an answer to.
#[tauri::command]
pub async fn security_encrypt_fields(
    state: State<'_, crate::state::AppState>,
) -> Result<FieldEncryption, SecurityError> {
    if !super::session::archive_is_open() {
        return Err(SecurityError::new("locked", "the archive is locked"));
    }
    convert_database(state.db_manager.pool(), DatabaseDirection::Encrypt).await?;
    security_field_encryption(state).await
}

use crate::database::field_encryption::Direction as DatabaseDirection;

/// One pass over the database columns, with the backup taken first when the
/// pass is the one that seals things.
async fn convert_database(
    pool: &sqlx::SqlitePool,
    direction: DatabaseDirection,
) -> Result<(), SecurityError> {
    if direction == DatabaseDirection::Encrypt {
        match crate::database::field_encryption::back_up(pool, &database_backup_path()).await {
            Ok(true) => log::info!("Wrote a copy of the database before encrypting it"),
            Ok(false) => log::info!("A copy of the database from before encryption already exists"),
            // Worth stopping for: the copy is the owner's way back if the
            // conversion turns out to have been a mistake.
            Err(error) => {
                log::error!("Could not copy the database: {error}");
                return Err(SecurityError::new("backupFailed", error.to_string()));
            }
        }
    }

    match crate::database::field_encryption::convert_all(pool, direction).await {
        Ok(report) => {
            log::info!("Database fields: {}", report.summary());
            Ok(())
        }
        Err(error) => {
            log::error!("The database conversion failed and was rolled back: {error}");
            Err(SecurityError::new(
                match direction {
                    DatabaseDirection::Encrypt => "fieldEncryptionFailed",
                    DatabaseDirection::Decrypt => "fieldDecryptionIncomplete",
                },
                error.to_string(),
            ))
        }
    }
}

/// The state of the text columns in the database.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldEncryption {
    pub sealed: usize,
    pub plaintext: usize,
}

/// The state of the recordings on disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingEncryption {
    pub encrypted: usize,
    pub plaintext: usize,
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
    /// Whether this machine could offer a Windows Hello prompt at all.
    pub quick_available: bool,
    /// Whether the owner turned quick unlock on. Off unless they did.
    pub quick_enabled: bool,
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
            #[cfg(windows)]
            KeystoreError::QuickKeyUnusable(_) => Self::new("quickKeySetupFailed", message),
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
        #[cfg(windows)]
        quick_available: super::hello::is_available(),
        #[cfg(not(windows))]
        quick_available: false,
        #[cfg(windows)]
        quick_enabled: guard.as_ref().is_some_and(Keystore::has_quick_unlock),
        #[cfg(not(windows))]
        quick_enabled: false,
    }
}

/// Turns quick unlock on for this archive, after the password is proven.
///
/// Opt-in and reversible: an archive is password-only until this is called, and
/// [`security_quick_disable`] puts it back that way.
#[cfg(windows)]
#[tauri::command]
pub fn security_quick_enable(
    password: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    if !super::hello::is_available() {
        return Err(SecurityError::new(
            "quickUnavailable",
            "Windows Hello is not set up on this machine",
        ));
    }

    // Logged on the way out as well as on success. Without this a refused
    // password left nothing behind at all: the only trace of six rejected
    // attempts was the counter inside the keystore, which is not where anyone
    // looks when a switch simply does not move.
    state
        .with_keystore(|keystore| keystore.enable_quick_unlock(&password))
        .inspect_err(|error| {
            log::warn!("Quick unlock was not turned on: {} ({})", error.code, error.message)
        })?;
    log::info!("Quick unlock turned on");
    Ok(())
}

/// Turns quick unlock off, leaving the password as the way in.
#[cfg(windows)]
#[tauri::command]
pub fn security_quick_disable(state: State<'_, SecurityState>) -> Result<(), SecurityError> {
    state.with_keystore(|keystore| {
        keystore.disable_quick_unlock();
        Ok(())
    })?;
    log::info!("Quick unlock turned off");
    Ok(())
}

/// Opens the archive with a Windows Hello prompt.
///
/// Runs off the async runtime: the prompt is modal and blocks until the owner
/// answers, which would otherwise stall every other task in the process.
#[cfg(windows)]
#[tauri::command]
pub async fn security_quick_unlock<R: Runtime>(
    prompt: String,
    app: AppHandle<R>,
) -> Result<(), SecurityError> {
    let window = app
        .get_webview_window("main")
        .and_then(|window| window.hwnd().ok())
        .map(|handle| handle.0 as isize)
        .ok_or_else(|| {
            SecurityError::new("quickNoWindow", "the app window could not be found")
        })?;

    let quick = {
        let state = app.state::<SecurityState>();
        let guard = state.keystore_guard();
        guard
            .as_ref()
            .ok_or_else(SecurityError::not_configured)?
            .quick
            .clone()
            .ok_or_else(|| SecurityError::new("quickNotConfigured", "quick unlock is not set up"))?
    };

    let dek = tauri::async_runtime::spawn_blocking(move || {
        super::quick::unlock(&quick, window, &prompt)
    })
    .await
    .map_err(|error| SecurityError::new("quickFailed", error.to_string()))?
    .map_err(SecurityError::from)?;

    app.state::<SecurityState>().session.unlock(dek);
    log::info!("Archive unlocked with Windows Hello");
    Ok(())
}

#[cfg(windows)]
impl From<super::quick::QuickError> for SecurityError {
    fn from(error: super::quick::QuickError) -> Self {
        let message = error.to_string();
        match error {
            super::quick::QuickError::NotConfigured => {
                Self::new("quickNotConfigured", message)
            }
            super::quick::QuickError::Hello(super::hello::HelloError::Declined) => {
                Self::new("quickDeclined", message)
            }
            super::quick::QuickError::Hello(super::hello::HelloError::Unavailable) => {
                Self::new("quickUnavailable", message)
            }
            super::quick::QuickError::Hello(_) => Self::new("quickFailed", message),
            // A DPAPI failure normally means the Windows account changed, or the
            // keystore was carried over from another machine. Either way the
            // password still works, and saying so beats a generic error.
            super::quick::QuickError::Dpapi(_) => Self::new("quickKeyUnusable", message),
            super::quick::QuickError::Corrupt(_) => Self::new("quickKeyUnusable", message),
        }
    }
}

/// Protects the archive for the first time and leaves it unlocked.
///
/// The recovery code comes back once, here. It is never stored in the clear and
/// cannot be asked for again — only replaced.
#[tauri::command]
pub async fn security_setup<R: Runtime>(
    app: AppHandle<R>,
    password: String,
    with_recovery: bool,
    state: State<'_, SecurityState>,
    archive: State<'_, crate::state::AppState>,
) -> Result<RecoveryCodeResponse, SecurityError> {
    let recovery_code = state.create(&password, with_recovery)?;

    log::info!(
        "Archive protected with a password (recovery code kept: {})",
        with_recovery
    );

    // Setting a password has to cover the sessions already recorded, or the
    // lock screen promises something that is not true. This is the one moment
    // it happens without being asked for separately: the owner just asked for
    // the archive to be protected.
    if let Some(report) = convert_archive(&app, Direction::Encrypt).await {
        if !report.is_complete() {
            log::error!(
                "{} recordings could not be encrypted and are still plaintext",
                report.failed.len()
            );
        }
    }

    // The same argument covers the database: a lock screen over readable
    // titles and transcripts would be a promise the file does not keep. A
    // failure here is logged rather than returned — the password is already
    // set and the recovery code has to reach the owner, who would otherwise
    // have a protected archive and no code for it. The settings screen shows
    // what is still in the clear and offers the pass again.
    if let Err(error) = convert_database(archive.db_manager.pool(), DatabaseDirection::Encrypt).await {
        log::error!("The database was not encrypted at setup: {}", error.message);
    }

    Ok(RecoveryCodeResponse { recovery_code })
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

/// Writes a copy of the keystore where the owner asked for it.
///
/// Because that copy is what an offline attack against the password needs, it
/// belongs somewhere other than with the recordings. The interface says so
/// where the owner chooses the file.
#[tauri::command]
pub fn security_export_key_backup(
    path: String,
    state: State<'_, SecurityState>,
) -> Result<(), SecurityError> {
    state.export_key_backup(std::path::Path::new(&path))?;
    log::info!("Key backup written");
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
    if super::session::background_work_running() {
        return Err(SecurityError::new(
            "jobInProgress",
            "the archive cannot be locked while a summary, transcription or import is running",
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
/// **The recordings are decrypted before the key is deleted, and the key is
/// not deleted unless every one of them made it.** Deleting it first would
/// leave the archive behind a key that no longer exists anywhere — the sessions
/// would still be on disk, unreadable forever. A recording that cannot be
/// converted therefore cancels the whole operation, with the password still in
/// place and nothing lost.
#[tauri::command]
pub async fn security_disable<R: Runtime>(
    app: AppHandle<R>,
    password: String,
    state: State<'_, SecurityState>,
    archive: State<'_, crate::state::AppState>,
) -> Result<(), SecurityError> {
    // Opens the archive as well as proving the password: the key is what the
    // recordings have to be decrypted with.
    let dek = state.with_keystore(|keystore| keystore.unlock_with_password(&password))?;
    state.session.unlock(dek);

    // The database goes first: it converts inside a transaction, so a
    // failure costs nothing, while a half-decrypted folder of recordings
    // would have to be converted back.
    convert_database(archive.db_manager.pool(), DatabaseDirection::Decrypt).await?;

    match convert_archive(&app, Direction::Decrypt).await {
        Some(report) if report.is_complete() => {
            log::info!("Archive decrypted: {} recordings", report.converted);
        }
        Some(report) => {
            log::error!(
                "Keeping the password: {} recordings could not be decrypted",
                report.failed.len()
            );
            return Err(SecurityError::new(
                "decryptionIncomplete",
                "some recordings could not be decrypted, so the password was kept",
            ));
        }
        None => {
            return Err(SecurityError::new(
                "locked",
                "the archive closed before the recordings could be decrypted",
            ));
        }
    }

    std::fs::remove_file(&state.path).map_err(KeystoreError::Io)?;
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
            if crate::audio::recording_commands::is_recording().await
                || super::session::background_work_running()
            {
                // Deliberately not locking, and deliberately not resetting the
                // idle clock either: the moment the recording or the job
                // stops, this locks.
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

    // Clean up after a conversion that was interrupted by a crash or a
    // shutdown. Not a migration — it only puts back what was moved aside and
    // throws away what was never verified.
    let app_for_recovery = app.clone();
    tauri::async_runtime::spawn(async move {
        let roots = crate::audio::recording_preferences::recording_roots(&app_for_recovery).await;
        tauri::async_runtime::spawn_blocking(move || {
            crate::audio::archive_encryption::recover_interrupted(&roots);
        });
    });
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
        let directory = tempfile::tempdir().unwrap();
        let state = SecurityState::at(directory.path().join("keystore.json"));
        let error = state.with_keystore(|_| Ok(())).unwrap_err();
        assert_eq!(error.code, "notConfigured");
    }

    /// A state over a keystore file in a directory that lives as long as the test.
    fn temporary_state() -> (tempfile::TempDir, SecurityState) {
        let directory = tempfile::tempdir().unwrap();
        let state = SecurityState::at(directory.path().join("keystore.json"));
        (directory, state)
    }

    #[test]
    fn an_unprotected_archive_reports_itself_as_unconfigured() {
        let (_directory, state) = temporary_state();
        state.load().unwrap();
        assert_eq!(state.session.state(), LockState::Unconfigured);
    }

    #[test]
    fn setting_a_password_leaves_the_archive_open_with_the_key_in_memory() {
        let (_directory, state) = temporary_state();
        let code = state.create("correct horse", true).unwrap();

        assert!(code.is_some());
        assert_eq!(state.session.state(), LockState::Unlocked);
        assert!(state.session.with_dek(|key| key.len()).unwrap() == 32);
    }

    #[test]
    fn a_key_backup_opens_with_the_same_password_and_leaves_quick_unlock_behind() {
        let (directory, state) = temporary_state();
        state.create("correct horse", true).unwrap();
        let target = directory.path().join("backup").join("keystore.json");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();

        state.export_key_backup(&target).unwrap();

        let mut copy = Keystore::load_from(&target).unwrap().unwrap();
        assert!(copy.unlock_with_password("correct horse").is_ok());
        assert!(copy.recovery.is_some(), "the recovery envelope travels with it");
        #[cfg(windows)]
        assert!(copy.quick.is_none(), "quick unlock is bound to this machine");
    }

    #[test]
    fn an_unprotected_archive_has_no_key_to_back_up() {
        let (directory, state) = temporary_state();
        let target = directory.path().join("backup.json");

        let error = state.export_key_backup(&target).unwrap_err();

        assert_eq!(error.code, "notConfigured");
        assert!(!target.exists());
    }

    #[test]
    fn a_second_setup_is_refused_rather_than_orphaning_the_first_key() {
        let (_directory, state) = temporary_state();
        state.create("correct horse", false).unwrap();

        let error = state.create("another one", false).unwrap_err();
        assert_eq!(error.code, "alreadyConfigured");
    }

    #[test]
    fn a_restart_finds_the_archive_locked_and_the_password_opens_it() {
        let (directory, first_run) = temporary_state();
        let path = directory.path().join("keystore.json");
        first_run.create("correct horse", false).unwrap();

        // A new process reads the same file and starts with no key.
        let next_run = SecurityState::at(path);
        next_run.load().unwrap();
        assert_eq!(next_run.session.state(), LockState::Locked);
        assert!(next_run.session.with_dek(|_| ()).is_err());

        next_run
            .with_keystore(|keystore| keystore.unlock_with_password("correct horse"))
            .map(|dek| next_run.session.unlock(dek))
            .unwrap();
        assert_eq!(next_run.session.state(), LockState::Unlocked);
    }

    #[test]
    fn failed_attempts_survive_closing_the_window() {
        // Otherwise the throttle is worth nothing: restarting would reset it.
        let (directory, first_run) = temporary_state();
        let path = directory.path().join("keystore.json");
        first_run.create("correct horse", false).unwrap();

        for _ in 0..2 {
            let _ = first_run.with_keystore(|keystore| keystore.unlock_with_password("wrong"));
        }

        let next_run = SecurityState::at(path);
        next_run.load().unwrap();
        assert_eq!(security_status_of(&next_run).failed_attempts, 2);
    }

    #[test]
    fn locking_drops_the_key_but_leaves_the_archive_configured() {
        let (_directory, state) = temporary_state();
        state.create("correct horse", false).unwrap();

        state.session.lock();
        assert_eq!(state.session.state(), LockState::Locked);
        assert!(state.session.with_dek(|_| ()).is_err());
    }

    #[test]
    fn the_status_tells_the_window_whether_a_recovery_code_exists() {
        let (_directory, with_code) = temporary_state();
        with_code.create("correct horse", true).unwrap();
        assert!(security_status_of(&with_code).has_recovery);

        let (_other, without_code) = temporary_state();
        without_code.create("correct horse", false).unwrap();
        assert!(!security_status_of(&without_code).has_recovery);
    }

    #[test]
    fn a_new_keystore_starts_with_the_default_idle_timeout() {
        let (_directory, state) = temporary_state();
        state.create("correct horse", false).unwrap();
        assert_eq!(
            security_status_of(&state).auto_lock_minutes,
            DEFAULT_AUTO_LOCK_MINUTES
        );
    }

    /// The body of `security_status`, without the Tauri `State` wrapper the test
    /// has no way to build.
    fn security_status_of(state: &SecurityState) -> SecurityStatus {
        let guard = state.keystore_guard();
        SecurityStatus {
            state: state.session.state(),
            has_recovery: guard.as_ref().is_some_and(Keystore::has_recovery),
            wait_seconds: guard.as_ref().map_or(0, Keystore::wait_required),
            auto_lock_minutes: guard.as_ref().and_then(|store| store.auto_lock_minutes),
            failed_attempts: guard.as_ref().map_or(0, |store| store.failed_attempts),
            // Not asked of Windows here: these tests are about the keystore, and
            // the answer would depend on the machine they run on.
            quick_available: false,
            #[cfg(windows)]
            quick_enabled: guard.as_ref().is_some_and(Keystore::has_quick_unlock),
            #[cfg(not(windows))]
            quick_enabled: false,
        }
    }
}
