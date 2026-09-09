//! The recovery code: a second door to the same data key.
//!
//! Shown once when the archive is protected, meant to be written down or printed
//! and kept somewhere other than the computer. This is not a backdoor — it is a
//! second envelope holding the same key, and only the owner has it. For an
//! archive of years of sessions, forgetting a password is the likelier
//! catastrophe.
//!
//! The code is Crockford base32: no `I`, `L`, `O` or `U`, so a handwritten `1`
//! and `l` cannot be confused, and a `0` read as `O` still decodes. The last
//! character is a checksum, so a mistyped code is reported as a typo instead of
//! being counted as a failed unlock attempt.

use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

/// Bytes behind a code: 19 random, 1 checksum.
const CODE_BYTES: usize = 20;
/// Of which this many carry entropy — 152 bits.
const ENTROPY_BYTES: usize = 19;
/// Characters in the code, ungrouped. 20 bytes * 8 / 5.
const CODE_CHARS: usize = 32;
/// Characters between dashes, for reading aloud and typing back.
const GROUP: usize = 4;

/// Crockford's alphabet, in value order.
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// A recovery code in the form the owner sees. Wiped when dropped.
pub type RecoveryCode = Zeroizing<String>;

/// Why a typed code could not be used.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RecoveryError {
    #[error("a recovery code has {expected} characters, this one has {actual}")]
    WrongLength { expected: usize, actual: usize },
    #[error("'{0}' is not a character a recovery code can contain")]
    BadCharacter(char),
    #[error("this code has a typo in it")]
    ChecksumMismatch,
}

/// Makes a fresh recovery code. Shown once and never stored in the clear.
pub fn generate() -> RecoveryCode {
    let mut bytes = Zeroizing::new([0u8; CODE_BYTES]);
    rand::thread_rng().fill_bytes(&mut bytes[..ENTROPY_BYTES]);
    bytes[ENTROPY_BYTES] = checksum(&bytes[..ENTROPY_BYTES]);
    Zeroizing::new(group(&encode(bytes.as_ref())))
}

/// Turns a code the owner typed into the bytes the key derivation expects.
///
/// Accepts any spacing and case, and forgives the substitutions Crockford was
/// designed around: `O` for zero, `I` or `L` for one.
pub fn to_secret(typed: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryError> {
    let cleaned = normalize(typed)?;
    let bytes = decode(&cleaned)?;

    if bytes[ENTROPY_BYTES] != checksum(&bytes[..ENTROPY_BYTES]) {
        return Err(RecoveryError::ChecksumMismatch);
    }

    Ok(Zeroizing::new(bytes.to_vec()))
}

/// Strips formatting and maps look-alike characters onto their real values.
fn normalize(typed: &str) -> Result<Zeroizing<Vec<u8>>, RecoveryError> {
    let mut out = Vec::with_capacity(CODE_CHARS);

    for character in typed.chars() {
        // Dashes, spaces and the odd non-breaking space come from copy-paste.
        if character.is_whitespace() || character == '-' || character == '\u{2013}' {
            continue;
        }

        let upper = character.to_ascii_uppercase();
        let value = match upper {
            'O' => 0,
            'I' | 'L' => 1,
            'U' => return Err(RecoveryError::BadCharacter(character)),
            _ => match ALPHABET.iter().position(|&c| c == upper as u8) {
                Some(index) => index as u8,
                None => return Err(RecoveryError::BadCharacter(character)),
            },
        };
        out.push(value);
    }

    if out.len() != CODE_CHARS {
        return Err(RecoveryError::WrongLength {
            expected: CODE_CHARS,
            actual: out.len(),
        });
    }

    Ok(Zeroizing::new(out))
}

/// Packs bytes into base32 characters, most significant bit first.
fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(CODE_CHARS);
    let mut buffer: u16 = 0;
    let mut bits: u32 = 0;

    for &byte in bytes {
        buffer = (buffer << 8) | byte as u16;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(ALPHABET[index] as char);
        }
    }

    out
}

/// Unpacks base32 values back into bytes. Input is already validated to be
/// exactly [`CODE_CHARS`] values in range.
fn decode(values: &[u8]) -> Result<[u8; CODE_BYTES], RecoveryError> {
    let mut out = [0u8; CODE_BYTES];
    let mut buffer: u16 = 0;
    let mut bits: u32 = 0;
    let mut written = 0;

    for &value in values {
        buffer = (buffer << 5) | value as u16;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out[written] = ((buffer >> bits) & 0xff) as u8;
            written += 1;
        }
    }

    debug_assert_eq!(written, CODE_BYTES);
    Ok(out)
}

/// One byte of SHA-256 over the entropy. Enough to catch a typo; it is not a
/// security property, and the envelope's authentication tag is what actually
/// decides whether a code is right.
fn checksum(entropy: &[u8]) -> u8 {
    let mut hasher = Sha256::new();
    hasher.update(b"meetily/recovery/v1");
    hasher.update(entropy);
    hasher.finalize()[0]
}

/// Inserts dashes so the code can be read off a sheet of paper.
fn group(code: &str) -> String {
    code.as_bytes()
        .chunks(GROUP)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_code_reads_as_eight_groups_of_four() {
        let code = generate();
        assert_eq!(code.len(), CODE_CHARS + 7);
        let groups: Vec<&str> = code.split('-').collect();
        assert_eq!(groups.len(), 8);
        assert!(groups.iter().all(|part| part.len() == GROUP));
    }

    #[test]
    fn a_generated_code_round_trips() {
        let code = generate();
        assert!(to_secret(&code).is_ok());
    }

    #[test]
    fn the_secret_is_stable_across_formatting_and_case() {
        let code = generate();
        let canonical = to_secret(&code).unwrap();

        let lowercase = code.to_lowercase();
        let spaced = code.replace('-', " ");
        let squashed = code.replace('-', "");

        assert_eq!(to_secret(&lowercase).unwrap(), canonical);
        assert_eq!(to_secret(&spaced).unwrap(), canonical);
        assert_eq!(to_secret(&squashed).unwrap(), canonical);
    }

    #[test]
    fn look_alike_characters_are_forgiven() {
        // Whoever writes the code down by hand will confuse these; Crockford's
        // alphabet leaves them out precisely so they can be mapped back.
        let code = "0000-0000-0000-0000-0000-0000-0000-0000";
        let with_letter_o = "OOOO-OOOO-OOOO-OOOO-OOOO-OOOO-OOOO-OOOO";
        // Both decode to the same bytes; both fail the checksum, and that is the
        // point of the comparison - the two inputs behave identically.
        assert_eq!(to_secret(code), to_secret(with_letter_o));

        let ones = "1111-1111-1111-1111-1111-1111-1111-1111";
        let letters = "IIII-LLLL-iiii-llll-1111-1111-1111-1111";
        assert_eq!(to_secret(ones), to_secret(letters));
    }

    #[test]
    fn a_single_wrong_character_is_reported_as_a_typo() {
        let code = generate();
        let first = code.chars().next().unwrap();
        // Move one character to a different value inside the alphabet.
        let replacement = if first == '0' { '1' } else { '0' };
        let mistyped: String = replacement.to_string() + &code[1..];

        assert_eq!(to_secret(&mistyped), Err(RecoveryError::ChecksumMismatch));
    }

    #[test]
    fn a_short_code_is_reported_as_short_not_as_wrong() {
        let code = generate();
        let truncated = &code[..code.len() - 5];
        match to_secret(truncated).unwrap_err() {
            RecoveryError::WrongLength { expected, actual } => {
                assert_eq!(expected, CODE_CHARS);
                assert!(actual < CODE_CHARS);
            }
            other => panic!("expected a length complaint, got {other:?}"),
        }
    }

    #[test]
    fn characters_outside_the_alphabet_are_named() {
        let code = generate();
        let poisoned: String = "U".to_string() + &code[1..];
        assert_eq!(to_secret(&poisoned), Err(RecoveryError::BadCharacter('U')));

        let punctuated: String = "@".to_string() + &code[1..];
        assert_eq!(to_secret(&punctuated), Err(RecoveryError::BadCharacter('@')));
    }

    #[test]
    fn two_codes_are_never_the_same() {
        let first = generate();
        let second = generate();
        assert_ne!(*first, *second);
    }

    #[test]
    fn encoding_and_decoding_are_inverses() {
        let mut bytes = [0u8; CODE_BYTES];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(37).wrapping_add(11);
        }
        let text = encode(&bytes);
        let values = normalize(&text).unwrap();
        assert_eq!(decode(&values).unwrap(), bytes);
    }
}
