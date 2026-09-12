//! Keys for the private archive.
//!
//! The password never becomes the key that data is encrypted with. It derives a
//! key-encryption key (KEK) through argon2id; a separate random data key (DEK)
//! does the actual encrypting and is stored on disk wrapped in the KEK. Two
//! consequences matter:
//!
//! * changing the password rewraps 32 bytes instead of re-encrypting an archive;
//! * a recovery code can wrap the *same* DEK in a second envelope, so a forgotten
//!   password is recoverable without a backdoor — the second envelope only opens
//!   with a code the owner holds.
//!
//! This module owns the key material and the on-disk keystore. What is done with
//! that key lives beside it: [`stream`] seals the recordings (B3) and [`field`]
//! seals the text columns of the database (B4).

pub mod commands;
#[cfg(windows)]
pub mod dpapi;
pub mod envelope;
pub mod field;
#[cfg(windows)]
pub mod hello;
pub mod kdf;
#[cfg(windows)]
pub mod quick;
pub mod keystore;
pub mod recovery;
pub mod session;
pub mod stream;

pub use keystore::{Keystore, KeystoreError};
pub use session::{KeySession, LockState, SessionError};
