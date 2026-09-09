//! Tying a secret to this Windows account.
//!
//! DPAPI encrypts with a key Windows derives from the logged-in account. The
//! ciphertext is useless on another machine or under another account, so a
//! stolen disk yields nothing — but anything running *as this user* can decrypt
//! it. That is the whole security model of quick unlock, and the reason the
//! password remains the real door.
//!
//! Callers pass extra entropy, which binds a blob to one archive: a blob lifted
//! out of another keystore will not open this one even under the same account.

#![cfg(windows)]

use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use zeroize::Zeroizing;

/// What can go wrong asking Windows to protect or reveal a secret.
#[derive(Debug, thiserror::Error)]
pub enum DpapiError {
    #[error("Windows could not protect the secret: {0}")]
    Protect(windows::core::Error),
    #[error("Windows could not read the protected secret: {0}")]
    Unprotect(windows::core::Error),
}

/// Encrypts `secret` for this Windows account.
pub fn protect(secret: &[u8], entropy: &[u8]) -> Result<Vec<u8>, DpapiError> {
    let mut input = blob(secret);
    let mut extra = blob(entropy);
    let mut output = CRYPT_INTEGER_BLOB::default();

    // SAFETY: all three blobs point at live slices for the duration of the call,
    // and the output blob is copied out and freed before returning.
    unsafe {
        CryptProtectData(
            &mut input,
            None,
            Some(&mut extra),
            None,
            None,
            // No UI from inside DPAPI: the Hello prompt is ours to show.
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(DpapiError::Protect)?;
    }

    Ok(take(&mut output))
}

/// Decrypts what [`protect`] produced, on the same machine under the same account.
pub fn unprotect(sealed: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>, DpapiError> {
    let mut input = blob(sealed);
    let mut extra = blob(entropy);
    let mut output = CRYPT_INTEGER_BLOB::default();

    // SAFETY: as above.
    unsafe {
        CryptUnprotectData(
            &mut input,
            None,
            Some(&mut extra),
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(DpapiError::Unprotect)?;
    }

    Ok(Zeroizing::new(take(&mut output)))
}

fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
    CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr() as *mut u8,
    }
}

/// Copies a blob Windows allocated, then frees it.
fn take(output: &mut CRYPT_INTEGER_BLOB) -> Vec<u8> {
    // SAFETY: Windows filled this blob and owns the allocation until LocalFree.
    let bytes = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
    unsafe {
        let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData as *mut core::ffi::c_void,
        )));
    }
    output.pbData = std::ptr::null_mut();
    output.cbData = 0;
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_comes_back_unchanged() {
        let secret = b"a thirty-two byte key would go here";
        let entropy = b"keystore salt";

        let sealed = protect(secret, entropy).unwrap();
        assert_ne!(sealed.as_slice(), secret.as_slice());

        let opened = unprotect(&sealed, entropy).unwrap();
        assert_eq!(opened.as_slice(), secret.as_slice());
    }

    #[test]
    fn the_wrong_entropy_does_not_open_it() {
        // This is what stops a blob copied out of one archive's keystore from
        // opening another's.
        let sealed = protect(b"secret", b"keystore A").unwrap();
        assert!(unprotect(&sealed, b"keystore B").is_err());
    }

    #[test]
    fn tampering_with_the_blob_is_detected() {
        let mut sealed = protect(b"secret", b"entropy").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;
        assert!(unprotect(&sealed, b"entropy").is_err());
    }

    #[test]
    fn an_empty_secret_round_trips() {
        let sealed = protect(&[], b"entropy").unwrap();
        assert!(unprotect(&sealed, b"entropy").unwrap().is_empty());
    }
}
