//! Sealing one database column value.
//!
//! B3 put the recordings behind the archive key; the database still held the
//! words. A meeting title, a client's name, a line of transcript and a summary
//! are the same material as the audio — often a more convenient form of it,
//! because they are already text. This module is what B4 seals them with.
//!
//! ## Why a value and not the file
//!
//! SQLCipher would encrypt the whole database and was rejected when the
//! architecture was agreed: it means a patched build of SQLite underneath
//! `sqlx`, a second copy of the engine in the process, and an all-or-nothing
//! key that the application cannot hold open for a row at a time. Sealing
//! values keeps the schema, the indexes on ids and timestamps, the foreign
//! keys and every query that does not touch text exactly as they were.
//!
//! What it costs is honest and worth writing down: **the shape of the archive
//! stays visible.** How many meetings there are, when each was recorded, how
//! long it ran, which client it belongs to and how many lines of transcript it
//! has are all still readable in the file. What was said is not. That is the
//! trade the owner agreed to — metadata for the ability to keep using an
//! ordinary SQLite database.
//!
//! ## Two shapes of stored value
//!
//! * a **sealed value** ([`MARKER`]) — AES-256-GCM, random nonce per write, so
//!   the same word written twice looks different both times;
//! * a **blind index** ([`INDEX_MARKER`]) — a keyed hash, deterministic on
//!   purpose, for the one thing a sealed value cannot do: let SQL find a row by
//!   exact name. `clients.normalized_name` and `people.normalized_name` exist
//!   only to answer "is this person already here?", and they still can.
//!
//! A blind index leaks equality, and nothing more: two rows with the same hash
//! hold the same name. That is the whole point of it. It does not leak the
//! name, and it is not reversible without the key — but an attacker who guesses
//! a name can confirm the guess, which is why it is used for the lookup column
//! and never as a substitute for sealing the name itself.
//!
//! ## What the AAD binds
//!
//! Each value's AAD names its table and column, so a ciphertext cannot be moved
//! from `clients.notes` into `meetings.title` and read out somewhere it would
//! be shown differently. It deliberately does **not** bind the row id. Binding
//! it would only stop a permutation of values within one column, and an
//! attacker who can write to the database file can already overwrite any row
//! with any other content, or restore last week's file wholesale. Paying for
//! that with a rule that a value can never be copied between rows would make
//! ordinary work — restoring a backed-up summary, re-keying a row — fail for
//! no security gained.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;

use super::envelope::NONCE_LEN;

/// Prefix of a sealed value. Chosen to be something no title, name, note or
/// transcript line would start with by accident, so a value read back can be
/// told apart from plaintext written before B4 without a schema flag.
pub const MARKER: &str = "tkf1:";

/// Prefix of a blind index.
pub const INDEX_MARKER: &str = "tkb1:";

/// Bytes of a blind index. Sixteen: the index has to survive birthday
/// collisions across an archive of thousands of names, not resist a
/// preimage — the key does that.
const INDEX_LEN: usize = 16;

/// Which column a value belongs to. Bound into the AAD, so the pair is part of
/// what the tag authenticates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Field {
    pub table: &'static str,
    pub column: &'static str,
}

impl Field {
    pub const fn new(table: &'static str, column: &'static str) -> Self {
        Self { table, column }
    }

    /// The AAD for this column. Versioned, so a later format can coexist with
    /// values already written.
    fn aad(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(32 + self.table.len() + self.column.len());
        aad.extend_from_slice(b"talkkeeper/field/v1:");
        aad.extend_from_slice(self.table.as_bytes());
        aad.push(b'.');
        aad.extend_from_slice(self.column.as_bytes());
        aad
    }
}

/// What can go wrong with a stored value.
#[derive(Debug, thiserror::Error)]
pub enum FieldError {
    #[error("the value in {table}.{column} does not open with this key")]
    WrongKey {
        table: &'static str,
        column: &'static str,
    },
    #[error("the value in {table}.{column} is sealed but malformed")]
    Malformed {
        table: &'static str,
        column: &'static str,
    },
    #[error("a sealed value in {table}.{column} is not valid text")]
    NotText {
        table: &'static str,
        column: &'static str,
    },
}

/// Seals `plaintext` so that the same text always produces the same
/// ciphertext under the same key.
///
/// Needed by exactly two columns: `transcripts.speaker` and
/// `person_speakers.speaker_label`. They are joined on
/// (`t.speaker = ps.speaker_label`), matched on (`WHERE speaker = ?`) and made
/// unique together with the meeting id, so a random nonce would break the
/// schema rather than the reader.
///
/// What it gives up is equality — the file shows which lines share a speaker.
/// That is not a loss in practice: any scheme that keeps those joins working
/// leaks the same fact, including storing a blind index in a second column,
/// and the count of distinct speakers is legible from the row structure
/// anyway. What stays hidden is the one thing that matters, the name itself.
///
/// The nonce is derived from the plaintext through a keyed hash rather than
/// drawn at random (the SIV construction). Repeating a nonce under one key is
/// fatal for GCM when the messages differ — here an equal nonce means an equal
/// message, which is the property being asked for.
pub fn seal_deterministic(key: &[u8], field: Field, plaintext: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&nonce_key(key))
        .expect("HMAC accepts a key of any length");
    mac.update(field.aad().as_slice());
    mac.update(b"\0");
    mac.update(plaintext.as_bytes());
    let digest = mac.finalize().into_bytes();
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&digest[..NONCE_LEN]);
    seal_with_nonce(key, field, plaintext, nonce)
}

/// Seals `plaintext` for `field`.
///
/// An empty string is sealed like any other: leaving it alone would say "this
/// meeting has no title" in the clear, and the difference between an empty note
/// and a written one is itself information.
pub fn seal(key: &[u8], field: Field, plaintext: &str) -> String {
    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);
    seal_with_nonce(key, field, plaintext, nonce)
}

fn seal_with_nonce(key: &[u8], field: Field, plaintext: &str, nonce: [u8; NONCE_LEN]) -> String {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let sealed = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: &field.aad(),
            },
        )
        // AES-GCM only refuses input near its 64 GiB message limit. A column
        // value is not that, and there is no sensible fallback if it were:
        // returning the plaintext would defeat the whole module.
        .expect("AES-GCM refused a database field");

    let mut body = Vec::with_capacity(NONCE_LEN + sealed.len());
    body.extend_from_slice(&nonce);
    body.extend_from_slice(&sealed);

    format!("{MARKER}{}", encode(&body))
}

/// Opens a value that [`is_sealed`] said was sealed.
pub fn open(key: &[u8], field: Field, stored: &str) -> Result<String, FieldError> {
    let body = stored.strip_prefix(MARKER).ok_or(FieldError::Malformed {
        table: field.table,
        column: field.column,
    })?;

    let bytes = decode(body).ok_or(FieldError::Malformed {
        table: field.table,
        column: field.column,
    })?;
    if bytes.len() < NONCE_LEN {
        return Err(FieldError::Malformed {
            table: field.table,
            column: field.column,
        });
    }
    let (nonce, sealed) = bytes.split_at(NONCE_LEN);

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let plain = cipher
        .decrypt(
            Nonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: &field.aad(),
            },
        )
        .map_err(|_| FieldError::WrongKey {
            table: field.table,
            column: field.column,
        })?;

    String::from_utf8(plain).map_err(|_| FieldError::NotText {
        table: field.table,
        column: field.column,
    })
}

/// Whether a stored value is sealed rather than plaintext written before B4.
pub fn is_sealed(stored: &str) -> bool {
    stored.starts_with(MARKER)
}

/// Whether a stored lookup value is a blind index rather than a plain
/// normalized name.
pub fn is_blinded(stored: &str) -> bool {
    stored.starts_with(INDEX_MARKER)
}

/// Opens `stored` if it is sealed, and passes it through if it is not.
///
/// Both shapes are live at once and will stay that way: an archive with no
/// password holds plaintext, and an archive that has just gained one holds a
/// mixture until the conversion runs. Every read goes through here.
pub fn open_or_passthrough(
    key: Option<&[u8]>,
    field: Field,
    stored: &str,
) -> Result<String, FieldError> {
    if !is_sealed(stored) {
        return Ok(stored.to_string());
    }
    match key {
        Some(key) => open(key, field, stored),
        // The archive is locked and something is still reading. Saying so is
        // better than handing back the ciphertext, which would be shown to the
        // owner as if it were the title.
        None => Err(FieldError::WrongKey {
            table: field.table,
            column: field.column,
        }),
    }
}

/// The blind index of an already-normalized lookup value.
///
/// Keyed with a value derived from the archive key rather than the key itself,
/// so the index cannot be used as an oracle against the sealing key.
pub fn blind_index(key: &[u8], field: Field, normalized: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&index_key(key))
        .expect("HMAC accepts a key of any length");
    mac.update(field.aad().as_slice());
    mac.update(b"\0");
    mac.update(normalized.as_bytes());
    let digest = mac.finalize().into_bytes();

    format!("{INDEX_MARKER}{}", encode(&digest[..INDEX_LEN]))
}

/// Turns a lookup value into whatever the archive stores for it: a blind index
/// when there is a key, the normalized text when there is not.
pub fn lookup_value(key: Option<&[u8]>, field: Field, normalized: &str) -> String {
    match key {
        Some(key) => blind_index(key, field, normalized),
        None => normalized.to_string(),
    }
}

/// Separates the deterministic-nonce key from the sealing key, for the same
/// reason [`index_key`] does: a value derived with one must not be usable
/// against the other.
fn nonce_key(key: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(b"talkkeeper/deterministic-nonce/v1");
    let digest = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// Separates the blind-index key from the sealing key. One HMAC, not argon2:
/// the input is already 32 random bytes, so there is nothing to slow down.
fn index_key(key: &[u8]) -> [u8; 32] {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(b"talkkeeper/blind-index/v1");
    let digest = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(bytes)
}

fn decode(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD_NO_PAD.decode(text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TITLE: Field = Field::new("meetings", "title");
    const NOTES: Field = Field::new("clients", "notes");

    fn key() -> [u8; 32] {
        [9u8; 32]
    }

    #[test]
    fn a_sealed_value_comes_back_word_for_word() {
        let sealed = seal(&key(), TITLE, "Планёрка 12 сентября");
        assert_eq!(open(&key(), TITLE, &sealed).unwrap(), "Планёрка 12 сентября");
    }

    #[test]
    fn the_plaintext_is_not_in_the_stored_value() {
        // The point of the module in one assertion: whatever ends up in the
        // database file, `grep` must not find the words in it.
        let sealed = seal(&key(), TITLE, "смета на ремонт");
        assert!(!sealed.contains("смета"));
        assert!(!sealed.contains("атака"));
    }

    #[test]
    fn the_same_text_seals_differently_every_time() {
        // Otherwise the file would show which meetings share a title, and how
        // often a word recurs across an archive.
        let first = seal(&key(), TITLE, "первая поставка");
        let second = seal(&key(), TITLE, "первая поставка");
        assert_ne!(first, second);
        assert_eq!(open(&key(), TITLE, &first).unwrap(), open(&key(), TITLE, &second).unwrap());
    }

    #[test]
    fn a_value_moved_to_another_column_does_not_open() {
        let sealed = seal(&key(), NOTES, "рабочая заметка");
        assert!(matches!(
            open(&key(), TITLE, &sealed),
            Err(FieldError::WrongKey { .. })
        ));
    }

    #[test]
    fn another_key_does_not_open_it() {
        let sealed = seal(&key(), TITLE, "встреча");
        assert!(matches!(
            open(&[1u8; 32], TITLE, &sealed),
            Err(FieldError::WrongKey { .. })
        ));
    }

    #[test]
    fn an_altered_value_does_not_open() {
        let sealed = seal(&key(), TITLE, "встреча");
        let mut broken: Vec<char> = sealed.chars().collect();
        let last = broken.len() - 1;
        broken[last] = if broken[last] == 'A' { 'B' } else { 'A' };
        let broken: String = broken.into_iter().collect();
        assert!(open(&key(), TITLE, &broken).is_err());
    }

    #[test]
    fn an_empty_string_is_sealed_like_any_other() {
        let sealed = seal(&key(), NOTES, "");
        assert!(is_sealed(&sealed));
        assert_eq!(open(&key(), NOTES, &sealed).unwrap(), "");
    }

    #[test]
    fn plaintext_written_before_b4_passes_through() {
        let value = open_or_passthrough(Some(&key()), TITLE, "Meeting 2026-09-01").unwrap();
        assert_eq!(value, "Meeting 2026-09-01");
    }

    #[test]
    fn a_sealed_value_read_without_a_key_is_an_error_not_ciphertext() {
        // The failure that matters: showing the owner `tkf1:AAAA…` as a title
        // would look like data loss and hide that the archive is locked.
        let sealed = seal(&key(), TITLE, "встреча");
        assert!(open_or_passthrough(None, TITLE, &sealed).is_err());
    }

    #[test]
    fn a_blind_index_is_stable_and_hides_the_name() {
        let field = Field::new("clients", "normalized_name");
        let first = blind_index(&key(), field, "иванов иван");
        let second = blind_index(&key(), field, "иванов иван");
        assert_eq!(first, second);
        assert!(!first.contains("иванов"));
        assert!(is_blinded(&first));
    }

    #[test]
    fn different_names_get_different_indexes() {
        let field = Field::new("clients", "normalized_name");
        assert_ne!(
            blind_index(&key(), field, "иванов иван"),
            blind_index(&key(), field, "иванов игорь")
        );
    }

    #[test]
    fn the_index_key_is_not_the_archive_key() {
        // A guard against a refactor that drops the derivation: an index
        // computed straight from the sealing key would let anyone holding an
        // index test guesses against the same key the values are sealed with.
        let field = Field::new("clients", "normalized_name");
        let derived = index_key(&key());
        assert_ne!(derived, key());
        assert!(blind_index(&key(), field, "имя").len() > INDEX_MARKER.len());
    }

    #[test]
    fn an_index_from_another_archive_does_not_match() {
        let field = Field::new("clients", "normalized_name");
        assert_ne!(
            blind_index(&key(), field, "имя"),
            blind_index(&[3u8; 32], field, "имя")
        );
    }

    #[test]
    fn without_a_key_the_lookup_value_stays_the_normalized_name() {
        let field = Field::new("clients", "normalized_name");
        assert_eq!(lookup_value(None, field, "иванов иван"), "иванов иван");
    }

    #[test]
    fn a_deterministic_value_is_stable_and_still_opens() {
        // What the speaker columns need: two rows written at different times
        // must compare equal in SQL, and still read back as the name.
        let field = Field::new("transcripts", "speaker");
        let first = seal_deterministic(&key(), field, "Анна");
        let second = seal_deterministic(&key(), field, "Анна");
        assert_eq!(first, second);
        assert_eq!(open(&key(), field, &first).unwrap(), "Анна");
        assert!(!first.contains("Анна"));
    }

    #[test]
    fn deterministic_values_differ_by_name_column_and_key() {
        let speaker = Field::new("transcripts", "speaker");
        let label = Field::new("person_speakers", "speaker_label");
        assert_ne!(
            seal_deterministic(&key(), speaker, "Анна"),
            seal_deterministic(&key(), speaker, "Борис")
        );
        assert_ne!(
            seal_deterministic(&key(), speaker, "Анна"),
            seal_deterministic(&key(), label, "Анна")
        );
        assert_ne!(
            seal_deterministic(&key(), speaker, "Анна"),
            seal_deterministic(&[5u8; 32], speaker, "Анна")
        );
    }

    #[test]
    fn the_deterministic_nonce_key_is_neither_the_archive_key_nor_the_index_key() {
        // Three keys are derived from one; a refactor that collapses any two of
        // them would let a value derived for one purpose be tested against
        // another.
        assert_ne!(nonce_key(&key()), key());
        assert_ne!(nonce_key(&key()), index_key(&key()));
    }
}
