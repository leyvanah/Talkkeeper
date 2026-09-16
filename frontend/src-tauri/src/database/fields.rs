//! Which database columns hold what was said, and how they are read and written.
//!
//! [`crate::security::field`] knows how to seal a value; this module knows
//! *which* values and owns the one rule that keeps B4 honest — **every read of
//! a listed column goes through [`open`] and every write through [`seal`]**.
//! There is no schema flag and no second table saying which rows are sealed: a
//! value carries its own marker, so a column can hold plaintext written before
//! the password existed and ciphertext written after it, in any mixture, and
//! both read back correctly.
//!
//! ## Why the key is fetched here and not passed in
//!
//! Threading the key through the repositories would mean an extra argument on
//! every query function and on everything that calls one, up through the Tauri
//! commands — for a value that is the same for the whole process and that most
//! of those functions have no business holding. B3 hit this already and
//! answered it with [`crate::security::session::with_current_key`], which lends
//! the key for the length of a closure. This module is the database's use of
//! the same door.
//!
//! ## The three states a call can be in
//!
//! * **no password on this machine** — there is no key, values pass through in
//!   the clear, and the archive behaves exactly as it did before phase B;
//! * **unlocked** — values are sealed on write and opened on read;
//! * **locked** — the key is gone. Writing still works (and writes plaintext,
//!   which is why the lock screen stops recordings from starting), but reading
//!   a sealed value fails rather than handing back `tkf1:…` to be drawn as a
//!   meeting title. Nothing should be reading in that state; if something does,
//!   the error says so.

use crate::security::field::{self, Field};
use crate::security::session;

/// The meeting's name, as the owner or the model wrote it.
pub const MEETING_TITLE: Field = Field::new("meetings", "title");
/// A line of transcript: the words themselves.
pub const TRANSCRIPT_TEXT: Field = Field::new("transcripts", "transcript");
/// Who said the line. A renamed speaker is a person's name — and this column
/// is joined against `person_speakers.speaker_label`, so it is sealed in the
/// joinable form.
pub const TRANSCRIPT_SPEAKER: Field = Field::new("transcripts", "speaker");
/// Per-line summary fields kept by the upstream schema.
pub const TRANSCRIPT_SUMMARY: Field = Field::new("transcripts", "summary");
pub const TRANSCRIPT_ACTION_ITEMS: Field = Field::new("transcripts", "action_items");
pub const TRANSCRIPT_KEY_POINTS: Field = Field::new("transcripts", "key_points");
/// When each word of the line was said, as JSON. It holds the words, so it is
/// sealed like the line itself.
pub const TRANSCRIPT_WORDS: Field = Field::new("transcripts", "words");
/// The generated summary, as a JSON document. Sealed whole: picking the text
/// out of it and sealing only that would leave the rest — which includes the
/// English cache of the same text — readable.
pub const SUMMARY_RESULT: Field = Field::new("summary_processes", "result");
/// The summary as it stood before it was regenerated — and deliberately the
/// *same* context as [`SUMMARY_RESULT`], not one of its own. Regeneration
/// copies the column to its backup inside a single SQL statement
/// (`result_backup = result`), so the two have to be interchangeable. It is
/// the one place the column-binding rule would cost something real and
/// protect nothing: it is the same value, about the same meeting.
pub const SUMMARY_RESULT_BACKUP: Field = SUMMARY_RESULT;
/// The transcript as it was handed to the model, plus the meeting's name at
/// the time.
pub const CHUNK_TEXT: Field = Field::new("transcript_chunks", "transcript_text");
pub const CHUNK_MEETING_NAME: Field = Field::new("transcript_chunks", "meeting_name");
/// A client's name and the owner's notes about them.
pub const CLIENT_DISPLAY_NAME: Field = Field::new("clients", "display_name");
pub const CLIENT_NOTES: Field = Field::new("clients", "notes");
/// A durable speaker profile: the same material as a client.
pub const PERSON_NAME: Field = Field::new("people", "display_name");
pub const PERSON_NOTES: Field = Field::new("people", "notes");
/// The label a person answers to inside one meeting — and deliberately the
/// *same* context as [`TRANSCRIPT_SPEAKER`], because the two columns are
/// compared to each other: `JOIN ... ON t.speaker = ps.speaker_label` is how
/// a durable profile finds the lines a person said. Two contexts would seal
/// the same name into two different values and the join would match
/// nothing — silently, which is the worst way for it to fail.
pub const SPEAKER_LABEL: Field = TRANSCRIPT_SPEAKER;

// `meeting_notes` is deliberately absent. The table exists in the schema
// from upstream, but nothing in this application reads or writes it — the
// notes editor keeps its content on the front end. Sealing a table no code
// opens would encrypt rows nothing can decrypt again.

/// Lookup columns. These are not sealed — they are blinded, so SQL can still
/// match a row by exact name without being able to read the name.
pub const CLIENT_LOOKUP: Field = Field::new("clients", "normalized_name");
pub const PERSON_LOOKUP: Field = Field::new("people", "normalized_name");

/// Seals a value for storage, or returns it unchanged when the archive has no
/// password.
pub fn seal(column: Field, plaintext: &str) -> String {
    session::with_current_key(|key| field::seal(key, column, plaintext))
        .unwrap_or_else(|| plaintext.to_string())
}

/// Seals a value that SQL still has to compare and join on, so the same text
/// always gives the same stored value. Only the two speaker columns use it —
/// see [`crate::security::field::seal_deterministic`] for what that costs.
pub fn seal_joinable(column: Field, plaintext: &str) -> String {
    session::with_current_key(|key| field::seal_deterministic(key, column, plaintext))
        .unwrap_or_else(|| plaintext.to_string())
}

/// The joinable form of an optional value.
pub fn seal_joinable_opt(column: Field, plaintext: Option<&str>) -> Option<String> {
    plaintext.map(|value| seal_joinable(column, value))
}

/// Seals an optional value. `NULL` stays `NULL`: the schema and half the
/// queries treat "no note" and "an empty note" as different things, and a
/// sealed empty string is not `NULL`.
pub fn seal_opt(column: Field, plaintext: Option<&str>) -> Option<String> {
    plaintext.map(|value| seal(column, value))
}

/// Reads a stored value back.
///
/// Reported as [`sqlx::Error::Decode`], which is what it is: the column could
/// not be turned into the value it stands for. Callers already handle that
/// error, so a locked archive surfaces as a failed query rather than as garbage
/// on the screen.
pub fn open(column: Field, stored: &str) -> Result<String, sqlx::Error> {
    let opened =
        match session::with_current_key(|key| field::open_or_passthrough(Some(key), column, stored))
        {
            Some(opened) => opened,
            None => field::open_or_passthrough(None, column, stored),
        };
    opened.map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

/// Reads an optional stored value back.
pub fn open_opt(column: Field, stored: Option<String>) -> Result<Option<String>, sqlx::Error> {
    stored.map(|value| open(column, &value)).transpose()
}

/// Turns an already-normalized name into what the lookup column stores: a blind
/// index when there is a key, the normalized name itself when there is not.
pub fn lookup(column: Field, normalized: &str) -> String {
    session::with_current_key(|key| field::blind_index(key, column, normalized))
        .unwrap_or_else(|| normalized.to_string())
}

/// Whether the archive currently has a key to seal with. Used by the conversion
/// in [`crate::database::field_encryption`] and by the settings screen, not by
/// ordinary queries — those just call [`seal`] and [`open`].
pub fn sealing_is_on() -> bool {
    session::archive_is_open()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_key_values_pass_through_untouched() {
        // The state a machine with no password is in, and the one every test in
        // this crate runs in: nothing about the archive changes.
        assert_eq!(seal(MEETING_TITLE, "Встреча"), "Встреча");
        assert_eq!(open(MEETING_TITLE, "Встреча").unwrap(), "Встреча");
        assert_eq!(seal_opt(CLIENT_NOTES, None), None);
        assert_eq!(lookup(CLIENT_LOOKUP, "анна"), "анна");
    }

    #[test]
    fn a_sealed_value_read_without_a_key_is_a_decode_error() {
        // What a locked archive looks like from a query's point of view. The
        // ciphertext is never returned as if it were the title.
        let sealed = field::seal(&[4u8; 32], MEETING_TITLE, "Встреча");
        let error = open(MEETING_TITLE, &sealed).unwrap_err();
        assert!(matches!(error, sqlx::Error::Decode(_)));
    }
}
