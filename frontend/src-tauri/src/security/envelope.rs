//! Wrapping the data key in a key-encryption key.
//!
//! AES-256-GCM. The authentication tag is what tells a wrong password from a
//! right one: unwrapping with the wrong KEK fails to verify rather than
//! returning garbage, so no separate password verifier has to be stored.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::kdf::{Kek, KdfParams, KEY_LEN, SALT_LEN};

/// Length of a GCM nonce.
pub const NONCE_LEN: usize = 12;

/// The random key that data is encrypted with. Wiped when dropped.
pub type Dek = Zeroizing<[u8; KEY_LEN]>;

/// Bound into every envelope so a wrapped key cannot be replayed as some other
/// kind of ciphertext later on.
const AAD: &[u8] = b"meetily/keystore/dek/v1";

/// One sealed copy of the data key, plus everything needed to open it given the
/// secret it was sealed with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// argon2id salt for this envelope's secret.
    pub salt: String,
    /// AES-GCM nonce.
    pub nonce: String,
    /// The data key, encrypted.
    pub wrapped_key: String,
    /// The cost this envelope's KEK was derived at.
    pub kdf: KdfParams,
}

/// What can go wrong opening or sealing an envelope.
#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("the secret does not open this envelope")]
    WrongSecret,
    #[error("the keystore field {field} is not valid base64")]
    NotBase64 { field: &'static str },
    #[error("the keystore field {field} has the wrong length: expected {expected}, got {actual}")]
    BadLength {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error(transparent)]
    Kdf(#[from] super::kdf::KdfError),
}

/// A fresh random data key. This is the only place one is created.
pub fn generate_dek() -> Dek {
    let mut dek: Dek = Zeroizing::new([0u8; KEY_LEN]);
    rand::thread_rng().fill_bytes(dek.as_mut());
    dek
}

/// Seals `dek` with a KEK derived from `secret`, choosing a fresh salt and nonce.
pub fn seal(secret: &[u8], dek: &Dek, kdf: KdfParams) -> Result<Envelope, EnvelopeError> {
    let mut salt = [0u8; SALT_LEN];
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    rand::thread_rng().fill_bytes(&mut nonce);

    let kek = super::kdf::derive_kek(secret, &salt, kdf)?;
    let wrapped = encrypt(&kek, &nonce, dek.as_ref())?;

    Ok(Envelope {
        salt: encode(&salt),
        nonce: encode(&nonce),
        wrapped_key: encode(&wrapped),
        kdf,
    })
}

/// Seals `dek` under a KEK that has already been derived, reusing this
/// envelope's salt. Used when the password is being changed and the old KEK is
/// not available — a fresh nonce is still drawn, because reusing a nonce under
/// the same key would leak the key stream.
pub fn reseal_with_kek(kek: &Kek, dek: &Dek, salt: &str, kdf: KdfParams) -> Result<Envelope, EnvelopeError> {
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    let wrapped = encrypt(kek, &nonce, dek.as_ref())?;

    Ok(Envelope {
        salt: salt.to_string(),
        nonce: encode(&nonce),
        wrapped_key: encode(&wrapped),
        kdf,
    })
}

/// Opens the envelope with `secret`, returning the data key.
///
/// A wrong secret is reported as [`EnvelopeError::WrongSecret`]; the caller
/// decides how to slow the next attempt down.
pub fn open(secret: &[u8], envelope: &Envelope) -> Result<Dek, EnvelopeError> {
    let salt = decode_exact(&envelope.salt, "salt", SALT_LEN)?;
    let nonce = decode_exact(&envelope.nonce, "nonce", NONCE_LEN)?;
    let wrapped = decode(&envelope.wrapped_key, "wrapped_key")?;

    let kek = super::kdf::derive_kek(secret, &salt, envelope.kdf)?;
    let plain = decrypt(&kek, &nonce, &wrapped)?;

    if plain.len() != KEY_LEN {
        return Err(EnvelopeError::BadLength {
            field: "wrapped_key",
            expected: KEY_LEN,
            actual: plain.len(),
        });
    }

    let mut dek: Dek = Zeroizing::new([0u8; KEY_LEN]);
    dek.copy_from_slice(&plain);
    Ok(dek)
}

/// Derives the KEK for an existing envelope without opening it. Needed when the
/// password changes: the new envelope keeps its own salt.
pub fn derive_for(secret: &[u8], envelope: &Envelope) -> Result<Kek, EnvelopeError> {
    let salt = decode_exact(&envelope.salt, "salt", SALT_LEN)?;
    Ok(super::kdf::derive_kek(secret, &salt, envelope.kdf)?)
}

fn encrypt(kek: &Kek, nonce: &[u8], plain: &[u8]) -> Result<Vec<u8>, EnvelopeError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek.as_ref()));
    cipher
        .encrypt(Nonce::from_slice(nonce), Payload { msg: plain, aad: AAD })
        // Encryption only fails on absurd input sizes; 32 bytes is not that.
        .map_err(|_| EnvelopeError::WrongSecret)
}

fn decrypt(kek: &Kek, nonce: &[u8], sealed: &[u8]) -> Result<Zeroizing<Vec<u8>>, EnvelopeError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(kek.as_ref()));
    cipher
        .decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: AAD })
        .map(Zeroizing::new)
        .map_err(|_| EnvelopeError::WrongSecret)
}

fn encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str, field: &'static str) -> Result<Vec<u8>, EnvelopeError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| EnvelopeError::NotBase64 { field })
}

fn decode_exact(text: &str, field: &'static str, expected: usize) -> Result<Vec<u8>, EnvelopeError> {
    let bytes = decode(text, field)?;
    if bytes.len() != expected {
        return Err(EnvelopeError::BadLength {
            field,
            expected,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        }
    }

    #[test]
    fn the_right_secret_gives_back_the_same_key() {
        let dek = generate_dek();
        let envelope = seal(b"open sesame", &dek, fast()).unwrap();
        let opened = open(b"open sesame", &envelope).unwrap();
        assert_eq!(opened.as_ref(), dek.as_ref());
    }

    #[test]
    fn the_wrong_secret_is_refused_rather_than_returning_garbage() {
        let dek = generate_dek();
        let envelope = seal(b"open sesame", &dek, fast()).unwrap();
        let error = open(b"open sesamf", &envelope).unwrap_err();
        assert!(matches!(error, EnvelopeError::WrongSecret));
    }

    #[test]
    fn two_envelopes_hold_the_same_key_behind_different_secrets() {
        // This is what makes a recovery code possible: one key, two doors.
        let dek = generate_dek();
        let by_password = seal(b"password", &dek, fast()).unwrap();
        let by_recovery = seal(b"recovery code", &dek, fast()).unwrap();

        assert_ne!(by_password.wrapped_key, by_recovery.wrapped_key);
        assert_eq!(open(b"password", &by_password).unwrap().as_ref(), dek.as_ref());
        assert_eq!(open(b"recovery code", &by_recovery).unwrap().as_ref(), dek.as_ref());
        // Neither secret opens the other's envelope.
        assert!(open(b"password", &by_recovery).is_err());
        assert!(open(b"recovery code", &by_password).is_err());
    }

    #[test]
    fn sealing_twice_never_repeats_a_nonce_or_a_salt() {
        let dek = generate_dek();
        let first = seal(b"password", &dek, fast()).unwrap();
        let second = seal(b"password", &dek, fast()).unwrap();
        assert_ne!(first.salt, second.salt);
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.wrapped_key, second.wrapped_key);
    }

    #[test]
    fn tampering_with_the_wrapped_key_is_detected() {
        let dek = generate_dek();
        let mut envelope = seal(b"password", &dek, fast()).unwrap();

        let mut raw = decode(&envelope.wrapped_key, "wrapped_key").unwrap();
        raw[0] ^= 0x01;
        envelope.wrapped_key = encode(&raw);

        assert!(matches!(
            open(b"password", &envelope).unwrap_err(),
            EnvelopeError::WrongSecret
        ));
    }

    #[test]
    fn resealing_keeps_the_salt_but_moves_the_nonce() {
        let dek = generate_dek();
        let envelope = seal(b"password", &dek, fast()).unwrap();
        let kek = derive_for(b"password", &envelope).unwrap();
        let resealed = reseal_with_kek(&kek, &dek, &envelope.salt, envelope.kdf).unwrap();

        assert_eq!(envelope.salt, resealed.salt);
        assert_ne!(envelope.nonce, resealed.nonce);
        assert_eq!(open(b"password", &resealed).unwrap().as_ref(), dek.as_ref());
    }

    #[test]
    fn a_corrupt_keystore_field_names_itself() {
        let dek = generate_dek();
        let mut envelope = seal(b"password", &dek, fast()).unwrap();
        envelope.salt = "not base64!!".to_string();

        match open(b"password", &envelope).unwrap_err() {
            EnvelopeError::NotBase64 { field } => assert_eq!(field, "salt"),
            other => panic!("expected a base64 complaint about the salt, got {other:?}"),
        }
    }

    #[test]
    fn a_salt_of_the_wrong_length_is_rejected() {
        let dek = generate_dek();
        let mut envelope = seal(b"password", &dek, fast()).unwrap();
        envelope.salt = encode(&[0u8; 4]);

        match open(b"password", &envelope).unwrap_err() {
            EnvelopeError::BadLength { field, expected, actual } => {
                assert_eq!((field, expected, actual), ("salt", SALT_LEN, 4));
            }
            other => panic!("expected a length complaint, got {other:?}"),
        }
    }
}
