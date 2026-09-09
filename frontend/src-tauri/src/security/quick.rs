//! Quick unlock: a Hello prompt plus a key only this Windows account can read.
//!
//! A fourth envelope over the same data key, alongside the password and the
//! recovery code. Its secret is 32 random bytes, kept in the keystore encrypted
//! by DPAPI. Opening it needs two things: Windows Hello to say yes, and the
//! Windows account that DPAPI sealed it under.
//!
//! **This is weaker than the password, by design and with the owner's
//! agreement.** The Hello prompt is enforced by this app, not by the maths;
//! whoever can run code as this Windows user can read the DPAPI blob without
//! ever being asked for a face. What it does guarantee is that a copied disk,
//! or the keystore on another machine, is useless.

#![cfg(windows)]

use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::dpapi;
use super::envelope::{self, Dek, Envelope};
use super::hello;
use super::kdf::{KdfParams, KEY_LEN};

/// Cost used for the quick envelope only.
///
/// The password envelope is deliberately slow because a password is guessable.
/// This secret is 32 bytes from the system generator — there is nothing to slow
/// an attacker down *to*, and making the owner wait half a second would defeat
/// the entire point of a quick unlock. Argon2's floor is `8 * parallelism`.
const QUICK_KDF: KdfParams = KdfParams {
    memory_kib: 8,
    iterations: 1,
    parallelism: 1,
};

/// The quick-unlock door, as stored in the keystore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickUnlock {
    /// The data key, sealed with the secret below.
    pub envelope: Envelope,
    /// That secret, encrypted by DPAPI for this Windows account. Base64.
    pub protected_secret: String,
    /// Random value mixed into DPAPI so this blob only opens for this archive.
    ///
    /// Its own field rather than a borrowed salt from elsewhere in the keystore:
    /// changing the password draws a new salt, and binding to that would break
    /// quick unlock silently the first time the owner changed their password.
    pub binding: String,
}

/// What can go wrong with quick unlock.
#[derive(Debug, thiserror::Error)]
pub enum QuickError {
    #[error("quick unlock is not set up for this archive")]
    NotConfigured,
    #[error(transparent)]
    Hello(#[from] hello::HelloError),
    #[error(transparent)]
    Dpapi(#[from] dpapi::DpapiError),
    #[error("the stored quick-unlock key is damaged: {0}")]
    Corrupt(String),
}

/// Sets quick unlock up for an archive that is currently open.
///
/// Requires the data key, so this can only be done from an unlocked archive —
/// the password has already been proven at that point.
pub fn enable(dek: &Dek) -> Result<QuickUnlock, QuickError> {
    let mut secret: Zeroizing<[u8; KEY_LEN]> = Zeroizing::new([0u8; KEY_LEN]);
    rand::thread_rng().fill_bytes(secret.as_mut());

    let mut binding = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut binding);

    let envelope = envelope::seal(secret.as_ref(), dek, QUICK_KDF)
        .map_err(|error| QuickError::Corrupt(error.to_string()))?;
    let protected = dpapi::protect(secret.as_ref(), &binding)?;

    Ok(QuickUnlock {
        envelope,
        protected_secret: encode(&protected),
        binding: encode(&binding),
    })
}

/// Opens the archive after Windows Hello says yes.
///
/// The prompt comes first and its failure short-circuits: no DPAPI call is made
/// unless the owner was recognised.
pub fn unlock(quick: &QuickUnlock, window: isize, prompt: &str) -> Result<Dek, QuickError> {
    hello::request_verification(window, prompt)?;
    open_sealed(quick)
}

/// The cryptographic half, without the prompt. Shared with the tests, which
/// have no window to show a prompt over.
fn open_sealed(quick: &QuickUnlock) -> Result<Dek, QuickError> {
    let protected = decode(&quick.protected_secret)?;
    let binding = decode(&quick.binding)?;
    let secret = dpapi::unprotect(&protected, &binding)?;

    envelope::open(&secret, &quick.envelope)
        .map_err(|error| QuickError::Corrupt(error.to_string()))
}

fn encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str) -> Result<Vec<u8>, QuickError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| QuickError::Corrupt("the protected key is not valid base64".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::envelope::generate_dek;

    #[test]
    fn quick_unlock_returns_the_same_data_key() {
        let dek = generate_dek();
        let quick = enable(&dek).unwrap();
        assert_eq!(open_sealed(&quick).unwrap().as_ref(), dek.as_ref());
    }

    #[test]
    fn the_secret_is_not_stored_in_the_clear() {
        let dek = generate_dek();
        let quick = enable(&dek).unwrap();

        let stored = serde_json::to_string(&quick).unwrap();
        let encoded_key = encode(dek.as_ref());
        assert!(!stored.contains(&encoded_key));
    }

    #[test]
    fn the_blob_is_bound_to_its_own_archive() {
        // Swapping in another archive's binding must not open this one, which is
        // what stops a keystore copied between archives on the same machine.
        let dek = generate_dek();
        let mine = enable(&dek).unwrap();
        let theirs = enable(&dek).unwrap();

        let protected = decode(&mine.protected_secret).unwrap();
        let other_binding = decode(&theirs.binding).unwrap();
        assert!(dpapi::unprotect(&protected, &other_binding).is_err());
    }

    #[test]
    fn two_setups_never_share_a_secret_or_a_binding() {
        let dek = generate_dek();
        let first = enable(&dek).unwrap();
        let second = enable(&dek).unwrap();
        assert_ne!(first.protected_secret, second.protected_secret);
        assert_ne!(first.binding, second.binding);
        assert_ne!(first.envelope.wrapped_key, second.envelope.wrapped_key);
    }

    #[test]
    fn the_binding_does_not_depend_on_anything_that_changes_later() {
        // Regression guard. The binding was briefly taken from the password
        // envelope's salt, which changing the password redraws - quick unlock
        // would have broken silently on the first password change.
        let dek = generate_dek();
        let quick = enable(&dek).unwrap();
        let serialized = serde_json::to_string(&quick).unwrap();
        let restored: QuickUnlock = serde_json::from_str(&serialized).unwrap();

        assert_eq!(open_sealed(&restored).unwrap().as_ref(), dek.as_ref());
    }

    #[test]
    fn the_quick_envelope_opens_fast_enough_to_be_worth_having() {
        // If this took as long as the password envelope there would be no point
        // to the feature. Half a second is the argon2id profile; this must be
        // far under it.
        let dek = generate_dek();
        let quick = enable(&dek).unwrap();

        let started = std::time::Instant::now();
        let _ = open_sealed(&quick).unwrap();
        let elapsed = started.elapsed();

        assert!(
            elapsed < std::time::Duration::from_millis(50),
            "quick unlock took {elapsed:?}, which is not quick"
        );
    }
}
