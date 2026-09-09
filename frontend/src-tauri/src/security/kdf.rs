//! Turning a secret the owner types into a key-encryption key.
//!
//! argon2id with OWASP's second recommended profile (19 MiB, 2 passes). The
//! memory cost is what makes a stolen keystore expensive to attack on a GPU;
//! the profile is stored in the keystore so a future increase can be applied to
//! new keystores without locking out old ones.

use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Length of every key this module produces or wraps.
pub const KEY_LEN: usize = 32;
/// Length of the salt stored beside each envelope.
pub const SALT_LEN: usize = 16;

/// A key-encryption key. Wiped when dropped.
pub type Kek = Zeroizing<[u8; KEY_LEN]>;

/// argon2id cost, recorded per envelope so old keystores keep opening after the
/// defaults are raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory cost in kibibytes.
    pub memory_kib: u32,
    /// Number of passes over that memory.
    pub iterations: u32,
    /// Lanes. One: the point is to be slow, not parallel.
    pub parallelism: u32,
}

impl Default for KdfParams {
    /// OWASP's `m=19456, t=2, p=1` profile for argon2id.
    fn default() -> Self {
        Self {
            memory_kib: 19 * 1024,
            iterations: 2,
            parallelism: 1,
        }
    }
}

/// Why a key could not be derived. Always a programming or config error — a
/// wrong password is not detected here but when the envelope fails to open.
#[derive(Debug, thiserror::Error)]
pub enum KdfError {
    #[error("argon2 parameters are out of range: {0}")]
    Params(String),
    #[error("argon2 could not derive a key: {0}")]
    Derive(String),
}

/// Derives a key-encryption key from a typed secret and its salt.
///
/// Used for both the password and the recovery code. The recovery code carries
/// far more entropy than a password and would be safe with a cheaper KDF, but
/// one code path is worth more here than the fraction of a second saved.
pub fn derive_kek(secret: &[u8], salt: &[u8], params: KdfParams) -> Result<Kek, KdfError> {
    let argon_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|error| KdfError::Params(error.to_string()))?;

    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut kek: Kek = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(secret, salt, kek.as_mut())
        .map_err(|error| KdfError::Derive(error.to_string()))?;

    Ok(kek)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Cheap parameters: these tests are about behaviour, not about how long an
    // attacker would wait.
    fn fast() -> KdfParams {
        KdfParams {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        }
    }

    #[test]
    fn same_secret_and_salt_give_the_same_key() {
        let salt = [7u8; SALT_LEN];
        let first = derive_kek(b"correct horse", &salt, fast()).unwrap();
        let second = derive_kek(b"correct horse", &salt, fast()).unwrap();
        assert_eq!(first.as_ref(), second.as_ref());
    }

    #[test]
    fn a_different_salt_gives_a_different_key() {
        let first = derive_kek(b"correct horse", &[1u8; SALT_LEN], fast()).unwrap();
        let second = derive_kek(b"correct horse", &[2u8; SALT_LEN], fast()).unwrap();
        assert_ne!(first.as_ref(), second.as_ref());
    }

    #[test]
    fn a_different_secret_gives_a_different_key() {
        let salt = [7u8; SALT_LEN];
        let first = derive_kek(b"correct horse", &salt, fast()).unwrap();
        let second = derive_kek(b"correct horsf", &salt, fast()).unwrap();
        assert_ne!(first.as_ref(), second.as_ref());
    }

    #[test]
    fn the_default_profile_is_accepted_by_argon2() {
        // Guards against a params tweak that argon2 would reject at runtime,
        // which would only surface when the owner first sets a password.
        let key = derive_kek(b"password", &[0u8; SALT_LEN], KdfParams::default()).unwrap();
        assert_eq!(key.len(), KEY_LEN);
    }
}
