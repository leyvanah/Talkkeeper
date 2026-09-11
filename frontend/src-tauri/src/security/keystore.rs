//! The keystore file: envelopes on disk, plus how hard the next guess is.
//!
//! It holds no secret in the clear. Losing the file loses the archive; copying
//! it gains an attacker nothing but the right to run argon2id over and over.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

use super::envelope::{self, Dek, Envelope, EnvelopeError};
use super::kdf::KdfParams;
use super::recovery::{self, RecoveryCode, RecoveryError};

/// File name under the app data root.
pub const KEYSTORE_FILE: &str = "keystore.json";

/// Guesses allowed before the wait starts.
const FREE_ATTEMPTS: u32 = 3;
/// First wait imposed after those, in seconds. Doubles from here.
const FIRST_DELAY_SECS: i64 = 5;
/// Longest wait imposed. Beyond this, doubling only punishes the owner.
const MAX_DELAY_SECS: i64 = 300;

/// Idle minutes a new keystore starts with. Fifteen is long enough to read a
/// transcript and answer the door, short enough that a laptop left on a table
/// closes itself.
pub const DEFAULT_AUTO_LOCK_MINUTES: Option<u64> = Some(15);

/// What the keystore file contains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keystore {
    /// Format version, so a later change can be migrated rather than guessed at.
    pub version: u32,
    /// The data key sealed with the password.
    pub password: Envelope,
    /// The same data key sealed with the recovery code, when the owner kept one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<Envelope>,
    /// The same data key behind a Windows Hello prompt, when the owner turned
    /// quick unlock on. Absent by default: an archive is password-only until
    /// this is deliberately added, and removing it returns to password-only.
    #[cfg(windows)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quick: Option<super::quick::QuickUnlock>,
    /// Consecutive failed unlock attempts. Survives a restart on purpose.
    #[serde(default)]
    pub failed_attempts: u32,
    /// Unix seconds before which no attempt is accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked_until: Option<i64>,
    /// Idle minutes before the key is dropped; `None` means never. Kept here
    /// rather than in the database because the lock screen has to know it while
    /// the archive is still closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_lock_minutes: Option<u64>,
    /// When the archive was first protected.
    pub created_at: String,
    /// When any envelope last changed.
    pub updated_at: String,
}

/// Everything that can go wrong with the keystore, in the words the owner needs.
#[derive(Debug, thiserror::Error)]
pub enum KeystoreError {
    #[error("the archive is not protected by a password yet")]
    NotConfigured,
    #[error("the archive is already protected by a password")]
    AlreadyConfigured,
    #[error("wrong password")]
    WrongPassword,
    #[error("no recovery code was kept for this archive")]
    NoRecoveryCode,
    #[error("the recovery code does not open this archive")]
    WrongRecoveryCode,
    #[error(transparent)]
    RecoveryFormat(#[from] RecoveryError),
    #[error("too many attempts; wait {seconds} s")]
    TooManyAttempts { seconds: i64 },
    #[error("the password must be at least {minimum} characters")]
    PasswordTooShort { minimum: usize },
    #[error("the keystore file is damaged: {0}")]
    Corrupt(String),
    /// Windows refused to tie a key to this account. Kept apart from
    /// `Corrupt` because it says nothing about the keystore: the password and
    /// the recovery code still open the archive, and only the quick way in is
    /// unavailable. Reported as a damaged keystore, it read as "unknown error"
    /// and sent whoever hit it looking in the wrong place.
    #[cfg(windows)]
    #[error("the quick-unlock key could not be created: {0}")]
    QuickKeyUnusable(String),
    #[error("could not read or write the keystore: {0}")]
    Io(#[from] std::io::Error),
}

impl From<EnvelopeError> for KeystoreError {
    /// A wrong secret is recognised where it happens, so anything reaching here
    /// is a damaged keystore rather than a bad guess.
    fn from(error: EnvelopeError) -> Self {
        match error {
            EnvelopeError::WrongSecret => KeystoreError::WrongPassword,
            other => KeystoreError::Corrupt(other.to_string()),
        }
    }
}

/// Shortest password accepted. Deliberately modest: argon2id carries the weight,
/// and a rule the owner resents is a rule they write on a sticky note.
pub const MIN_PASSWORD_LEN: usize = 8;

impl Keystore {
    /// Where the keystore lives.
    pub fn path() -> PathBuf {
        crate::paths::install_data_root().join(KEYSTORE_FILE)
    }

    /// Whether this machine has a protected archive.
    pub fn exists() -> bool {
        Self::path().exists()
    }

    /// Reads the keystore, or `None` when the archive is not protected.
    pub fn load() -> Result<Option<Self>, KeystoreError> {
        Self::load_from(&Self::path())
    }

    /// Reads a keystore from an explicit path. Tests use this; the app uses
    /// [`Self::load`].
    pub fn load_from(path: &Path) -> Result<Option<Self>, KeystoreError> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)?;
        let keystore: Self =
            serde_json::from_str(&text).map_err(|error| KeystoreError::Corrupt(error.to_string()))?;
        Ok(Some(keystore))
    }

    /// Writes the keystore, replacing the file only once the new bytes are on
    /// disk. A crash mid-write must not cost the owner their archive.
    pub fn save_to(&self, path: &Path) -> Result<(), KeystoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let text = serde_json::to_string_pretty(self)
            .map_err(|error| KeystoreError::Corrupt(error.to_string()))?;

        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)?;
        Ok(())
    }

    /// Writes the keystore to its usual place.
    pub fn save(&self) -> Result<(), KeystoreError> {
        self.save_to(&Self::path())
    }

    /// Protects a fresh archive: a random data key, sealed with the password and,
    /// when asked for, with a recovery code shown to the owner exactly once.
    pub fn create(
        password: &str,
        with_recovery: bool,
    ) -> Result<(Self, Option<RecoveryCode>, Dek), KeystoreError> {
        if password.chars().count() < MIN_PASSWORD_LEN {
            return Err(KeystoreError::PasswordTooShort {
                minimum: MIN_PASSWORD_LEN,
            });
        }

        let dek = envelope::generate_dek();
        let params = KdfParams::default();
        let password_envelope = envelope::seal(password.as_bytes(), &dek, params)?;

        let (recovery_envelope, code) = if with_recovery {
            let code = recovery::generate();
            let secret = recovery::to_secret(&code)?;
            (Some(envelope::seal(&secret, &dek, params)?), Some(code))
        } else {
            (None, None)
        };

        let now = now_text();
        let keystore = Self {
            version: 1,
            password: password_envelope,
            recovery: recovery_envelope,
            #[cfg(windows)]
            quick: None,
            failed_attempts: 0,
            locked_until: None,
            auto_lock_minutes: DEFAULT_AUTO_LOCK_MINUTES,
            created_at: now.clone(),
            updated_at: now,
        };

        Ok((keystore, code, dek))
    }

    /// Opens the archive with the password.
    ///
    /// Takes `&mut self` because every attempt, right or wrong, changes how long
    /// the next wrong one has to wait. The caller saves afterwards.
    pub fn unlock_with_password(&mut self, password: &str) -> Result<Dek, KeystoreError> {
        self.check_not_throttled()?;

        match envelope::open(password.as_bytes(), &self.password) {
            Ok(dek) => {
                self.clear_attempts();
                Ok(dek)
            }
            Err(EnvelopeError::WrongSecret) => {
                self.record_failure();
                Err(KeystoreError::WrongPassword)
            }
            Err(other) => Err(KeystoreError::Corrupt(other.to_string())),
        }
    }

    /// Opens the archive with the recovery code.
    ///
    /// A code that fails its own checksum is a typo, not a guess: it is reported
    /// as such and does not count towards the throttle.
    pub fn unlock_with_recovery(&mut self, code: &str) -> Result<Dek, KeystoreError> {
        let envelope = self
            .recovery
            .clone()
            .ok_or(KeystoreError::NoRecoveryCode)?;

        let secret = recovery::to_secret(code)?;
        self.check_not_throttled()?;

        match envelope::open(&secret, &envelope) {
            Ok(dek) => {
                self.clear_attempts();
                Ok(dek)
            }
            Err(EnvelopeError::WrongSecret) => {
                self.record_failure();
                Err(KeystoreError::WrongRecoveryCode)
            }
            Err(other) => Err(KeystoreError::Corrupt(other.to_string())),
        }
    }

    /// Swaps the password, keeping the data key and therefore the archive.
    ///
    /// The recovery envelope is untouched: the code the owner filed away still
    /// works, which is the whole reason the key is not derived from the password.
    pub fn change_password(&mut self, current: &str, new: &str) -> Result<(), KeystoreError> {
        if new.chars().count() < MIN_PASSWORD_LEN {
            return Err(KeystoreError::PasswordTooShort {
                minimum: MIN_PASSWORD_LEN,
            });
        }

        let dek = self.unlock_with_password(current)?;
        self.password = envelope::seal(new.as_bytes(), &dek, KdfParams::default())?;
        self.updated_at = now_text();
        Ok(())
    }

    /// Sets a new password using the recovery code, for an owner who forgot it.
    pub fn reset_password_with_recovery(
        &mut self,
        code: &str,
        new: &str,
    ) -> Result<(), KeystoreError> {
        if new.chars().count() < MIN_PASSWORD_LEN {
            return Err(KeystoreError::PasswordTooShort {
                minimum: MIN_PASSWORD_LEN,
            });
        }

        let dek = self.unlock_with_recovery(code)?;
        self.password = envelope::seal(new.as_bytes(), &dek, KdfParams::default())?;
        self.updated_at = now_text();
        Ok(())
    }

    /// Issues a new recovery code, retiring the old one. Requires the password,
    /// so an unattended unlocked window cannot quietly mint a second key.
    pub fn regenerate_recovery(&mut self, password: &str) -> Result<RecoveryCode, KeystoreError> {
        let dek = self.unlock_with_password(password)?;
        let code = recovery::generate();
        let secret = recovery::to_secret(&code)?;
        self.recovery = Some(envelope::seal(&secret, &dek, KdfParams::default())?);
        self.updated_at = now_text();
        Ok(code)
    }

    /// Drops the recovery envelope. After this the password is the only way in.
    pub fn remove_recovery(&mut self, password: &str) -> Result<(), KeystoreError> {
        let _ = self.unlock_with_password(password)?;
        self.recovery = None;
        self.updated_at = now_text();
        Ok(())
    }

    /// Seconds the owner must wait before the next attempt is accepted.
    pub fn wait_required(&self) -> i64 {
        match self.locked_until {
            Some(until) => (until - now_unix()).max(0),
            None => 0,
        }
    }

    /// Whether a recovery code exists for this archive.
    pub fn has_recovery(&self) -> bool {
        self.recovery.is_some()
    }

    /// Whether quick unlock is turned on. False on a fresh archive.
    #[cfg(windows)]
    pub fn has_quick_unlock(&self) -> bool {
        self.quick.is_some()
    }

    /// Turns quick unlock on, proving the password first.
    ///
    /// Requires the password even though the archive may already be open: adding
    /// a door to the archive should cost the same proof as changing the password.
    #[cfg(windows)]
    pub fn enable_quick_unlock(&mut self, password: &str) -> Result<(), KeystoreError> {
        let dek = self.unlock_with_password(password)?;
        self.quick = Some(
            super::quick::enable(&dek)
                .map_err(|error| KeystoreError::QuickKeyUnusable(error.to_string()))?,
        );
        self.updated_at = now_text();
        Ok(())
    }

    /// Turns quick unlock off, leaving the password (and any recovery code).
    ///
    /// No password needed: removing a way in never weakens the archive, and
    /// demanding the password to close a door the owner regrets opening would be
    /// friction in the wrong direction.
    #[cfg(windows)]
    pub fn disable_quick_unlock(&mut self) {
        self.quick = None;
        self.updated_at = now_text();
    }


    fn check_not_throttled(&self) -> Result<(), KeystoreError> {
        let seconds = self.wait_required();
        if seconds > 0 {
            return Err(KeystoreError::TooManyAttempts { seconds });
        }
        Ok(())
    }

    fn record_failure(&mut self) {
        self.failed_attempts = self.failed_attempts.saturating_add(1);
        let delay = delay_for(self.failed_attempts);
        self.locked_until = if delay > 0 {
            Some(now_unix() + delay)
        } else {
            None
        };
    }

    fn clear_attempts(&mut self) {
        self.failed_attempts = 0;
        self.locked_until = None;
    }
}

/// How long to wait after `attempts` consecutive failures.
///
/// The first few are free — the owner mistypes like anyone else. After that the
/// wait doubles, which turns an offline-speed guessing run into a hopeless one
/// while costing an owner who remembers on the fifth try under a minute.
fn delay_for(attempts: u32) -> i64 {
    if attempts <= FREE_ATTEMPTS {
        return 0;
    }
    let steps = attempts - FREE_ATTEMPTS - 1;
    FIRST_DELAY_SECS
        .saturating_mul(1i64.checked_shl(steps.min(16)).unwrap_or(i64::MAX))
        .min(MAX_DELAY_SECS)
}

fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

fn now_text() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Keeps a password out of the logs when a struct holding one is printed.
pub type Secret = Zeroizing<String>;

#[cfg(test)]
mod tests {
    use super::*;

    /// Creating a keystore with the real argon2 profile costs a fraction of a
    /// second each time; these tests do it a handful of times, which is fine,
    /// but the throttle tests below fake the clock instead of sleeping.
    fn make() -> (Keystore, RecoveryCode, Dek) {
        let (keystore, code, dek) = Keystore::create("correct horse", true).unwrap();
        (keystore, code.unwrap(), dek)
    }

    #[test]
    fn the_password_opens_the_archive() {
        let (mut keystore, _, dek) = make();
        let opened = keystore.unlock_with_password("correct horse").unwrap();
        assert_eq!(opened.as_ref(), dek.as_ref());
    }

    #[test]
    fn the_recovery_code_opens_the_same_archive() {
        let (mut keystore, code, dek) = make();
        let opened = keystore.unlock_with_recovery(&code).unwrap();
        assert_eq!(opened.as_ref(), dek.as_ref());
    }

    #[test]
    fn a_wrong_password_is_refused() {
        let (mut keystore, _, _) = make();
        assert!(matches!(
            keystore.unlock_with_password("wrong horse").unwrap_err(),
            KeystoreError::WrongPassword
        ));
    }

    #[test]
    fn changing_the_password_keeps_the_data_key_and_the_recovery_code() {
        let (mut keystore, code, dek) = make();
        keystore.change_password("correct horse", "a longer one").unwrap();

        assert!(keystore.unlock_with_password("correct horse").is_err());
        keystore.clear_attempts();

        assert_eq!(
            keystore.unlock_with_password("a longer one").unwrap().as_ref(),
            dek.as_ref()
        );
        // The sheet of paper in the drawer still works. This is the point of
        // wrapping a data key instead of deriving one from the password.
        assert_eq!(keystore.unlock_with_recovery(&code).unwrap().as_ref(), dek.as_ref());
    }

    #[test]
    fn a_forgotten_password_is_reset_with_the_recovery_code() {
        let (mut keystore, code, dek) = make();
        keystore.reset_password_with_recovery(&code, "brand new one").unwrap();

        assert_eq!(
            keystore.unlock_with_password("brand new one").unwrap().as_ref(),
            dek.as_ref()
        );
    }

    #[test]
    fn a_new_recovery_code_retires_the_old_one() {
        let (mut keystore, old_code, dek) = make();
        let new_code = keystore.regenerate_recovery("correct horse").unwrap();

        assert_ne!(*old_code, *new_code);
        assert_eq!(keystore.unlock_with_recovery(&new_code).unwrap().as_ref(), dek.as_ref());

        assert!(matches!(
            keystore.unlock_with_recovery(&old_code).unwrap_err(),
            KeystoreError::WrongRecoveryCode
        ));
    }

    #[test]
    fn an_archive_can_be_created_without_a_recovery_code() {
        let (mut keystore, code, _) = Keystore::create("correct horse", false).unwrap();
        assert!(code.is_none());
        assert!(!keystore.has_recovery());
        assert!(matches!(
            keystore.unlock_with_recovery("0000-0000-0000-0000-0000-0000-0000-0000").unwrap_err(),
            KeystoreError::NoRecoveryCode
        ));
    }

    #[test]
    fn dropping_the_recovery_code_leaves_only_the_password() {
        let (mut keystore, code, _) = make();
        keystore.remove_recovery("correct horse").unwrap();
        assert!(!keystore.has_recovery());
        assert!(matches!(
            keystore.unlock_with_recovery(&code).unwrap_err(),
            KeystoreError::NoRecoveryCode
        ));
    }

    #[test]
    fn a_short_password_is_refused_before_any_work_is_done() {
        assert!(matches!(
            Keystore::create("short", true).unwrap_err(),
            KeystoreError::PasswordTooShort { minimum: MIN_PASSWORD_LEN }
        ));
    }

    #[test]
    fn a_short_password_is_refused_when_changing_too() {
        let (mut keystore, _, _) = make();
        assert!(matches!(
            keystore.change_password("correct horse", "tiny").unwrap_err(),
            KeystoreError::PasswordTooShort { .. }
        ));
    }

    #[test]
    fn the_first_few_wrong_guesses_are_free_and_then_the_wait_grows() {
        assert_eq!(delay_for(0), 0);
        assert_eq!(delay_for(FREE_ATTEMPTS), 0);
        assert_eq!(delay_for(FREE_ATTEMPTS + 1), FIRST_DELAY_SECS);
        assert_eq!(delay_for(FREE_ATTEMPTS + 2), FIRST_DELAY_SECS * 2);
        assert_eq!(delay_for(FREE_ATTEMPTS + 3), FIRST_DELAY_SECS * 4);
        // And it stops growing rather than locking the owner out for a day.
        assert_eq!(delay_for(FREE_ATTEMPTS + 40), MAX_DELAY_SECS);
    }

    #[test]
    fn wrong_guesses_eventually_impose_a_wait_that_blocks_the_next_one() {
        let (mut keystore, _, _) = make();
        for _ in 0..=FREE_ATTEMPTS {
            let _ = keystore.unlock_with_password("wrong");
        }

        assert!(keystore.wait_required() > 0);
        // Even the right password has to wait its turn.
        assert!(matches!(
            keystore.unlock_with_password("correct horse").unwrap_err(),
            KeystoreError::TooManyAttempts { .. }
        ));
    }

    #[test]
    fn the_counter_resets_once_the_owner_gets_in() {
        let (mut keystore, _, _) = make();
        let _ = keystore.unlock_with_password("wrong");
        assert_eq!(keystore.failed_attempts, 1);

        keystore.unlock_with_password("correct horse").unwrap();
        assert_eq!(keystore.failed_attempts, 0);
        assert_eq!(keystore.locked_until, None);
    }

    #[test]
    fn a_mistyped_recovery_code_is_not_counted_as_a_guess() {
        // Otherwise an owner reading a code off paper would throttle themselves
        // out of their own archive over a misread character.
        let (mut keystore, code, _) = make();
        let first = code.chars().next().unwrap();
        let replacement = if first == '0' { '1' } else { '0' };
        let mistyped: String = replacement.to_string() + &code[1..];

        assert!(matches!(
            keystore.unlock_with_recovery(&mistyped).unwrap_err(),
            KeystoreError::RecoveryFormat(RecoveryError::ChecksumMismatch)
        ));
        assert_eq!(keystore.failed_attempts, 0);
    }

    #[test]
    fn the_keystore_survives_a_round_trip_through_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(KEYSTORE_FILE);

        let (keystore, code, dek) = make();
        keystore.save_to(&path).unwrap();

        let mut reloaded = Keystore::load_from(&path).unwrap().unwrap();
        assert_eq!(reloaded.unlock_with_password("correct horse").unwrap().as_ref(), dek.as_ref());
        assert_eq!(reloaded.unlock_with_recovery(&code).unwrap().as_ref(), dek.as_ref());
    }

    #[test]
    fn no_secret_is_written_to_the_file_in_the_clear() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(KEYSTORE_FILE);

        let (keystore, code, dek) = make();
        keystore.save_to(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();

        assert!(!text.contains("correct horse"));
        assert!(!text.contains(code.as_str()));
        // Nor the data key, in any encoding the file uses.
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(dek.as_ref());
        assert!(!text.contains(&encoded));
    }

    #[test]
    fn a_missing_file_means_the_archive_is_simply_not_protected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(KEYSTORE_FILE);
        assert!(Keystore::load_from(&path).unwrap().is_none());
    }

    #[test]
    fn a_damaged_file_is_reported_rather_than_treated_as_absent() {
        // Silently treating a corrupt keystore as "no password" would offer to
        // set up a new one and orphan the archive.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(KEYSTORE_FILE);
        std::fs::write(&path, "{ not json").unwrap();

        assert!(matches!(
            Keystore::load_from(&path).unwrap_err(),
            KeystoreError::Corrupt(_)
        ));
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(KEYSTORE_FILE);
        let (keystore, _, _) = make();
        keystore.save_to(&path).unwrap();
        keystore.save_to(&path).unwrap();

        let leftovers: Vec<_> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
    }
}
