//! The key while the app is running.
//!
//! Locking is not "show a screen over the window": it drops the data key and
//! wipes the bytes. Nothing that needs the key can proceed afterwards, which is
//! the property that makes the lock worth anything once B3 and B4 put real
//! ciphertext behind it.
//!
//! One consequence, agreed with the owner: **there is no auto-lock during a
//! recording.** Wiping the key mid-session would end the session. Idle locking
//! resumes as soon as the recording stops.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::envelope::Dek;

/// The one session of this process, for code that cannot be handed the Tauri
/// state.
///
/// Reading a recording happens deep inside the audio pipeline, where there is
/// no `AppHandle` — and threading one down to the decoder would mean carrying
/// the key through a dozen signatures that have no business holding it. So the
/// session publishes itself once at startup and the readers ask for the key
/// through [`with_current_key`], which borrows it for the length of a closure
/// and never hands out a copy.
static CURRENT: OnceLock<Arc<KeySession>> = OnceLock::new();

/// Publishes the session. The first caller wins; later calls are ignored, which
/// is what keeps a test's temporary session from replacing the real one.
pub fn install(session: Arc<KeySession>) {
    let _ = CURRENT.set(session);
}

/// Runs `use_key` with the archive key, or returns `None` when the archive is
/// locked, has no password, or the session was never installed.
///
/// Honest about one limit: a cipher built inside the closure keeps an expanded
/// copy of the key for as long as it lives, so locking the archive stops new
/// readers rather than reaching into open ones. Readers are short-lived and
/// there is no auto-lock during a recording, which is what makes that
/// acceptable.
pub fn with_current_key<T>(use_key: impl FnOnce(&[u8]) -> T) -> Option<T> {
    CURRENT.get()?.with_dek(use_key).ok()
}

/// Whether an archive key exists to encrypt new recordings with.
pub fn archive_is_open() -> bool {
    CURRENT
        .get()
        .map(|session| session.is_unlocked())
        .unwrap_or(false)
}

/// Whether this machine's archive has a password at all — open or locked.
///
/// The difference between "no password" and "locked" is the difference between
/// writing plaintext on purpose and writing it by accident, so writers ask this
/// when [`with_current_key`] comes back empty.
pub fn archive_is_protected() -> bool {
    CURRENT
        .get()
        .map(|session| session.state() != LockState::Unconfigured)
        .unwrap_or(false)
}

/// Long jobs that write to the archive and are running right now.
static BUSY: AtomicUsize = AtomicUsize::new(0);

/// Held by a job that writes to the archive — a summary, a retranscription,
/// an import, a diarization, the save after Stop. While any is held the idle
/// lock waits, as it waits for a recording: those jobs run for many minutes
/// with nobody touching the window, and a lock in the middle would cost the
/// job its result.
#[must_use = "the job counts as running only while the guard is alive"]
pub struct BusyGuard(());

/// Marks a long job as running until the returned guard is dropped.
pub fn busy() -> BusyGuard {
    BUSY.fetch_add(1, Ordering::SeqCst);
    BusyGuard(())
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Whether a job holding a [`BusyGuard`] is running.
pub fn background_work_running() -> bool {
    BUSY.load(Ordering::SeqCst) > 0
}

/// How often the idle check runs. Fine-grained enough that a one-minute timeout
/// means roughly a minute, cheap enough to ignore.
pub const IDLE_TICK: Duration = Duration::from_secs(15);

/// What the window should be showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LockState {
    /// No password has ever been set on this machine.
    Unconfigured,
    /// A password exists and the key is not in memory.
    Locked,
    /// The key is in memory and data can be read.
    Unlocked,
}

/// Asking for the key when it is not there.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("the archive is locked")]
    Locked,
}

/// Holds the data key for as long as the archive is open.
pub struct KeySession {
    inner: Mutex<Inner>,
}

struct Inner {
    /// Present exactly while unlocked. Wiped on drop by `Zeroizing`.
    dek: Option<Dek>,
    /// Whether a password exists on disk, cached so the lock screen can be drawn
    /// before touching the filesystem.
    configured: bool,
    /// Last sign of the owner being present.
    last_activity: Instant,
    /// Idle time after which the key is dropped. `None` means never.
    auto_lock_after: Option<Duration>,
}

impl Default for KeySession {
    fn default() -> Self {
        Self::new()
    }
}

impl KeySession {
    /// A session with no key, not yet told whether a password exists.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                dek: None,
                configured: false,
                last_activity: Instant::now(),
                auto_lock_after: None,
            }),
        }
    }

    /// Records whether a keystore exists on disk. Called at startup and after
    /// the password is set up or removed.
    pub fn set_configured(&self, configured: bool) {
        let mut inner = self.lock_inner();
        inner.configured = configured;
        if !configured {
            inner.dek = None;
        }
    }

    /// What the window should show right now.
    pub fn state(&self) -> LockState {
        let inner = self.lock_inner();
        match (inner.configured, inner.dek.is_some()) {
            (false, _) => LockState::Unconfigured,
            (true, true) => LockState::Unlocked,
            (true, false) => LockState::Locked,
        }
    }

    /// Takes the key into memory and starts the idle clock.
    pub fn unlock(&self, dek: Dek) {
        let mut inner = self.lock_inner();
        inner.configured = true;
        inner.dek = Some(dek);
        inner.last_activity = Instant::now();
    }

    /// Drops the key. The bytes are wiped as the `Zeroizing` value falls out of
    /// scope at the end of this function.
    pub fn lock(&self) {
        let mut inner = self.lock_inner();
        inner.dek = None;
    }

    /// Whether the key is currently in memory.
    pub fn is_unlocked(&self) -> bool {
        self.lock_inner().dek.is_some()
    }

    /// Marks the owner as present, pushing the idle deadline out.
    pub fn touch(&self) {
        self.lock_inner().last_activity = Instant::now();
    }

    /// Sets the idle timeout. `None` disables automatic locking.
    pub fn set_auto_lock_after(&self, after: Option<Duration>) {
        let mut inner = self.lock_inner();
        inner.auto_lock_after = after;
        inner.last_activity = Instant::now();
    }

    /// The idle timeout currently in force.
    pub fn auto_lock_after(&self) -> Option<Duration> {
        self.lock_inner().auto_lock_after
    }

    /// Runs `use_key` over the data key, or reports that the archive is locked.
    ///
    /// The key is handed out by reference and never cloned, so no copy of it
    /// outlives the call. B3 and B4 reach the key through here.
    pub fn with_dek<T>(&self, use_key: impl FnOnce(&[u8]) -> T) -> Result<T, SessionError> {
        let inner = self.lock_inner();
        match inner.dek.as_ref() {
            Some(dek) => Ok(use_key(dek.as_ref())),
            None => Err(SessionError::Locked),
        }
    }

    /// Whether the key has been idle past its deadline.
    ///
    /// Answers only the idle question. Whether locking is *allowed* right now —
    /// no recording in progress — is the caller's to check, because this module
    /// has no business knowing about the audio pipeline.
    pub fn idle_expired(&self) -> bool {
        let inner = self.lock_inner();
        match (inner.dek.is_some(), inner.auto_lock_after) {
            (true, Some(after)) => inner.last_activity.elapsed() >= after,
            _ => false,
        }
    }

    /// A poisoned mutex here means another thread panicked while holding the
    /// key. Recovering the guard is right: the alternative is a panic that takes
    /// down a recording, and the state behind it is a plain `Option`.
    fn lock_inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Test seam: pretends the owner was last active `elapsed` ago.
    #[cfg(test)]
    fn backdate_activity(&self, elapsed: Duration) {
        let mut inner = self.lock_inner();
        inner.last_activity = Instant::now() - elapsed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::envelope::generate_dek;

    #[test]
    fn a_job_counts_as_running_until_its_guard_is_dropped() {
        // The counter is process-wide, so this checks the guard's own effect
        // rather than an absolute value another test could be holding.
        let before = BUSY.load(Ordering::SeqCst);
        let outer = busy();
        let inner = busy();
        assert!(background_work_running());
        assert_eq!(BUSY.load(Ordering::SeqCst), before + 2);
        drop(inner);
        drop(outer);
        assert_eq!(BUSY.load(Ordering::SeqCst), before);
    }

    #[test]
    fn a_fresh_session_reports_no_password_configured() {
        let session = KeySession::new();
        assert_eq!(session.state(), LockState::Unconfigured);
    }

    #[test]
    fn a_configured_session_starts_locked() {
        let session = KeySession::new();
        session.set_configured(true);
        assert_eq!(session.state(), LockState::Locked);
    }

    #[test]
    fn unlocking_makes_the_key_reachable_and_locking_takes_it_away() {
        let session = KeySession::new();
        let dek = generate_dek();
        let expected = *dek;

        session.unlock(dek);
        assert_eq!(session.state(), LockState::Unlocked);
        let seen = session.with_dek(|key| key.to_vec()).unwrap();
        assert_eq!(seen, expected.to_vec());

        session.lock();
        assert_eq!(session.state(), LockState::Locked);
        assert!(matches!(
            session.with_dek(|_| ()).unwrap_err(),
            SessionError::Locked
        ));
    }

    #[test]
    fn without_a_timeout_the_key_never_expires_on_its_own() {
        let session = KeySession::new();
        session.unlock(generate_dek());
        session.backdate_activity(Duration::from_secs(60 * 60 * 24));
        assert!(!session.idle_expired());
    }

    #[test]
    fn with_a_timeout_the_key_expires_after_it() {
        let session = KeySession::new();
        session.unlock(generate_dek());
        session.set_auto_lock_after(Some(Duration::from_secs(300)));

        assert!(!session.idle_expired());
        session.backdate_activity(Duration::from_secs(299));
        assert!(!session.idle_expired());
        session.backdate_activity(Duration::from_secs(301));
        assert!(session.idle_expired());
    }

    #[test]
    fn activity_pushes_the_deadline_out() {
        let session = KeySession::new();
        session.unlock(generate_dek());
        session.set_auto_lock_after(Some(Duration::from_secs(300)));
        session.backdate_activity(Duration::from_secs(301));
        assert!(session.idle_expired());

        session.touch();
        assert!(!session.idle_expired());
    }

    #[test]
    fn a_locked_session_has_nothing_left_to_expire() {
        let session = KeySession::new();
        session.set_configured(true);
        session.set_auto_lock_after(Some(Duration::from_secs(1)));
        session.backdate_activity(Duration::from_secs(60));
        assert!(!session.idle_expired());
    }

    #[test]
    fn removing_the_password_also_drops_the_key() {
        let session = KeySession::new();
        session.unlock(generate_dek());
        session.set_configured(false);
        assert_eq!(session.state(), LockState::Unconfigured);
        assert!(session.with_dek(|_| ()).is_err());
    }
}
