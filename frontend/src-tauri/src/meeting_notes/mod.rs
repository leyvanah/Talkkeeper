//! The owner's own notes on a meeting, written while it is recorded or after.
//!
//! A note is a few lines of text and, when it was written during the
//! recording, the moment of the recording it belongs to. While the recording
//! runs the notes live in its crash journal
//! ([`crate::audio::transcript_journal`]), sealed line by line like the
//! transcript, so a crash that loses the meeting does not lose them. When the
//! meeting is saved they move into `meeting_notes.notes_json`, sealed whole,
//! in the same transaction as the meeting itself.
//!
//! A summary reads them beside the transcript: they are often exactly what the
//! owner wanted remembered.

pub mod commands;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::database::fields;

/// Longer than any note a person types; a paste of a whole document is
/// refused rather than sealed into every save.
pub const MAX_NOTE_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    /// Seconds into the recording, for a note written while it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<f64>,
    pub text: String,
    /// When it was written or last changed, RFC 3339.
    pub written_at: String,
}

impl Note {
    pub fn new(text: &str, at: Option<f64>) -> Result<Self, String> {
        Ok(Self {
            id: format!("note-{}", Uuid::new_v4()),
            at,
            text: checked_text(text)?,
            written_at: Utc::now().to_rfc3339(),
        })
    }

    /// The same note with new text; its moment in the recording stays.
    pub fn edited(&self, text: &str) -> Result<Self, String> {
        Ok(Self {
            text: checked_text(text)?,
            written_at: Utc::now().to_rfc3339(),
            ..self.clone()
        })
    }
}

/// Puts `note` right after the note `after`, or first when `after` is none —
/// the notes are lines of one text, and a line can be started anywhere in it.
/// An `after` that names no note puts it last. Returns where it went.
pub fn insert_after(notes: &mut Vec<Note>, note: Note, after: Option<&str>) -> usize {
    let index = match after {
        None => 0,
        Some(id) => notes
            .iter()
            .position(|existing| existing.id == id)
            .map_or(notes.len(), |found| found + 1),
    };
    notes.insert(index, note);
    index
}

fn checked_text(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("A note cannot be empty".to_string());
    }
    if text.chars().count() > MAX_NOTE_CHARS {
        return Err(format!("A note is limited to {MAX_NOTE_CHARS} characters"));
    }
    Ok(text.to_string())
}

/// A meeting's notes, in the order they were written.
pub async fn load(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<Note>, sqlx::Error> {
    let mut conn = pool.acquire().await?;
    load_in(&mut conn, meeting_id).await
}

/// [`load`] on a connection, so a change can read inside its transaction.
pub async fn load_in(conn: &mut SqliteConnection, meeting_id: &str) -> Result<Vec<Note>, sqlx::Error> {
    let stored: Option<Option<String>> =
        sqlx::query_scalar("SELECT notes_json FROM meeting_notes WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_optional(&mut *conn)
            .await?;
    let Some(json) = fields::open_opt(fields::MEETING_NOTES, stored.flatten())? else {
        return Ok(Vec::new());
    };
    serde_json::from_str(&json).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

/// Replaces a meeting's notes. No notes leaves no row.
pub async fn store(
    conn: &mut SqliteConnection,
    meeting_id: &str,
    notes: &[Note],
) -> Result<(), sqlx::Error> {
    if notes.is_empty() {
        sqlx::query("DELETE FROM meeting_notes WHERE meeting_id = ?")
            .bind(meeting_id)
            .execute(&mut *conn)
            .await?;
        return Ok(());
    }
    let json = serde_json::to_string(notes).map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO meeting_notes (meeting_id, notes_json, created_at, updated_at) \
         VALUES (?, ?, ?, ?) \
         ON CONFLICT(meeting_id) DO UPDATE SET \
             notes_json = excluded.notes_json, updated_at = excluded.updated_at",
    )
    .bind(meeting_id)
    .bind(fields::seal(fields::MEETING_NOTES, &json)?)
    .bind(&now)
    .bind(&now)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The notes as a model reads them after the transcript, or nothing when
/// there are none.
pub fn for_summary(notes: &[Note]) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    let mut out = String::from(
        "Notes the host wrote about this meeting (a time is how far into the recording it was written):\n",
    );
    for note in notes {
        let text = note.text.replace('\n', " ");
        match note.at {
            Some(at) => out.push_str(&format!("[{}] {}\n", crate::library_export::clock(at), text)),
            None => out.push_str(&format!("- {text}\n")),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::manager::MIGRATOR.run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) \
             VALUES ('m1', 't', '2026-09-27T10:00:00Z', '2026-09-27T10:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn an_empty_note_is_refused_and_text_is_trimmed() {
        assert!(Note::new("   \n", None).is_err());
        assert!(Note::new(&"я".repeat(MAX_NOTE_CHARS + 1), None).is_err());
        let note = Note::new("  спросить про сон \n", Some(12.5)).unwrap();
        assert_eq!(note.text, "спросить про сон");
        let edited = note.edited("спросить про сон и работу").unwrap();
        assert_eq!(edited.id, note.id);
        assert_eq!(edited.at, Some(12.5));
    }

    #[tokio::test]
    async fn notes_are_stored_replaced_and_cleared() {
        let pool = pool().await;
        assert!(load(&pool, "m1").await.unwrap().is_empty());

        let first = vec![Note::new("первая", Some(3.0)).unwrap()];
        let mut conn = pool.acquire().await.unwrap();
        store(&mut conn, "m1", &first).await.unwrap();
        drop(conn);
        assert_eq!(load(&pool, "m1").await.unwrap(), first);

        let both = vec![first[0].clone(), Note::new("вторая", None).unwrap()];
        let mut conn = pool.acquire().await.unwrap();
        store(&mut conn, "m1", &both).await.unwrap();
        drop(conn);
        assert_eq!(load(&pool, "m1").await.unwrap(), both);

        let mut conn = pool.acquire().await.unwrap();
        store(&mut conn, "m1", &[]).await.unwrap();
        drop(conn);
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_notes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn a_line_goes_where_it_was_started() {
        let note = |id: &str| Note {
            id: id.into(),
            at: None,
            text: id.into(),
            written_at: String::new(),
        };
        let mut notes = vec![note("a"), note("c")];
        assert_eq!(insert_after(&mut notes, note("b"), Some("a")), 1);
        assert_eq!(insert_after(&mut notes, note("first"), None), 0);
        assert_eq!(insert_after(&mut notes, note("last"), Some("gone")), 4);
        let order: Vec<_> = notes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(order, ["first", "a", "b", "c", "last"]);
    }

    #[test]
    fn the_summary_reads_each_note_with_its_moment() {
        assert_eq!(for_summary(&[]), None);
        let notes = vec![
            Note::new("устал\nна работе", Some(83.0)).unwrap(),
            Note::new("после записи", None).unwrap(),
        ];
        let text = for_summary(&notes).unwrap();
        assert!(text.contains("[01:23] устал на работе\n"));
        assert!(text.contains("- после записи\n"));
    }
}
