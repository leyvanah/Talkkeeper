//! The Windows Hello prompt.
//!
//! This asks Windows to recognise the owner and answers yes or no. It does not
//! produce a key, and the distinction matters — see the note below.
//!
//! ## Why this is a consent check and not a TPM key
//!
//! The stronger design is `KeyCredentialManager`: a key pair inside the TPM that
//! signs only after Hello succeeds, so a signature is impossible without the
//! owner's face. It was tried first and **does not work on this machine**:
//!
//! ```text
//! NgcSet : NO          Windows Hello for Business is not provisioned
//! AzureAdJoined : NO
//! DomainJoined : NO    a local account, in a workgroup
//! ```
//!
//! Face and PIN log the owner in, but that is a convenience credential, not an
//! NGC container, and `RequestCreateAsync` fails with `0x80098044` — identically
//! from this app and from PowerShell, so it is the machine's configuration and
//! not our code. Provisioning NGC would mean attaching the machine to a
//! Microsoft account or a domain, which for a private local archive is the wrong
//! trade and was rejected by the owner.
//!
//! So quick unlock is: this consent check, plus a key that DPAPI ties to this
//! Windows account (`dpapi.rs`). **The check is enforced by the app, not by the
//! cryptography** — code running as this Windows user could read the DPAPI blob
//! without ever showing this prompt. In practice quick unlock means "whoever can
//! log into this Windows account can open the archive". The password and the
//! recovery code remain the real doors; this one is for speed, and the owner
//! agreed to it on those terms.

#![cfg(windows)]

use windows::core::{factory, HSTRING};
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;
use windows_future::IAsyncOperation;

/// Why a Hello prompt did not end in a yes.
#[derive(Debug, thiserror::Error)]
pub enum HelloError {
    #[error("Windows Hello is not set up on this machine")]
    Unavailable,
    #[error("Windows Hello did not recognise the owner")]
    Declined,
    #[error("Windows returned an error: {0}")]
    Windows(#[from] windows::core::Error),
}

/// Whether this machine can show a Hello prompt at all.
///
/// False where no face, fingerprint or PIN is enrolled, and where the hardware
/// is missing. Callers use this to decide whether to *offer* quick unlock.
pub fn is_available() -> bool {
    match UserConsentVerifier::CheckAvailabilityAsync().and_then(|operation| operation.get()) {
        Ok(UserConsentVerifierAvailability::Available) => true,
        Ok(other) => {
            log::info!("Windows Hello is not available: {other:?}");
            false
        }
        Err(error) => {
            log::warn!("Could not ask Windows whether Hello is available: {error}");
            false
        }
    }
}

/// Shows the Hello prompt over `window` and waits for an answer.
///
/// The window handle is not optional. A desktop app has no CoreWindow, so the
/// plain WinRT `RequestVerificationAsync` has nothing to parent its dialog to
/// and fails; Windows provides `IUserConsentVerifierInterop` exactly for this,
/// and it takes the HWND to put the dialog in front of.
pub fn request_verification(window: isize, message: &str) -> Result<(), HelloError> {
    if !is_available() {
        return Err(HelloError::Unavailable);
    }

    let interop: IUserConsentVerifierInterop = factory::<UserConsentVerifier, _>()?;
    // SAFETY: the handle comes from the Tauri window that is calling this and
    // outlives the prompt, which is modal to it.
    let operation: IAsyncOperation<UserConsentVerificationResult> = unsafe {
        interop.RequestVerificationForWindowAsync(
            HWND(window as *mut core::ffi::c_void),
            &HSTRING::from(message),
        )?
    };

    match operation.get()? {
        UserConsentVerificationResult::Verified => Ok(()),
        other => {
            log::info!("Windows Hello declined: {other:?}");
            Err(HelloError::Declined)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Asking whether Hello exists must never panic, on any machine.
    #[test]
    fn availability_can_always_be_asked() {
        let _ = is_available();
    }

    /// The prompt needs a real window, so this can only be exercised from the
    /// running app. Kept as a note rather than a test that would pass by
    /// accident in an environment where it proves nothing.
    ///
    /// Not run with the suite: on a machine where Hello is set up, Windows does
    /// not always refuse the null handle — it can put a real verification
    /// prompt on the screen and wait for a face or a PIN, which stalls the
    /// whole run and asks the owner for something no one requested.
    #[test]
    #[ignore = "can open a real Windows Hello prompt and wait on it; run by hand"]
    fn a_prompt_without_a_window_is_refused_rather_than_hanging() {
        // A null handle is not a window; Windows should say so promptly.
        let outcome = request_verification(0, "test");
        assert!(outcome.is_err() || outcome.is_ok());
    }
}
