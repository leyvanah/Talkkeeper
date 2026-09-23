//! Bringing an archive that was written before B4 under the key, and back out.
//!
//! [`fields`] handles rows as they are written from now on. This handles the
//! rows that are already there — and the reverse, because removing the password
//! has to leave a database the application can still read without one.
//!
//! ## Why this is easier than B3 was, and where the care went instead
//!
//! Converting the recordings meant rewriting files one at a time, each one
//! verified before it replaced the original, with a recovery pass for a crash
//! in the middle. A database gives that for free: the whole conversion runs in
//! one transaction, so it either finishes or never happened. There is no
//! half-converted state to recover from.
//!
//! What it does not give for free is a way back if the *key* is wrong, so the
//! order inside the transaction matters. Every value is read and transformed
//! before anything is written; a value that will not open aborts the pass with
//! the transaction untouched. That is what makes "remove the password" safe:
//! it decrypts everything first and only then deletes the key.
//!
//! ## The lookup columns
//!
//! `clients.normalized_name` and `people.normalized_name` are blinded rather
//! than sealed, and a blind index cannot be reversed. Going back, the
//! normalized name is recomputed from the display name, which is where it came
//! from in the first place.

use std::path::Path;

use sqlx::{Row, Sqlite, SqlitePool, Transaction};

use super::fields;
use crate::security::field::{self, Field};

/// Which way the conversion runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Plaintext values become sealed ones.
    Encrypt,
    /// Sealed values become plaintext, so the archive stays readable once the
    /// password is gone.
    Decrypt,
}

/// What one pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldReport {
    /// Values this pass changed.
    pub converted: usize,
    /// Values that were already the way they should be.
    pub untouched: usize,
}

impl FieldReport {
    pub fn summary(&self) -> String {
        format!(
            "{} values converted, {} already so",
            self.converted, self.untouched
        )
    }
}

/// How much of the archive is sealed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldCounts {
    pub sealed: usize,
    pub plaintext: usize,
}

/// One column to convert: where it lives and how its value is protected.
struct Column {
    table: &'static str,
    /// The primary key to address a row by. Every table here has a single one.
    key: &'static str,
    name: &'static str,
    field: Field,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A fresh nonce per write.
    Sealed,
    /// A nonce derived from the value, because SQL compares this column.
    Joinable,
    /// A keyed hash of the normalized name, recomputed from `source` on the way
    /// back because a hash cannot be reversed.
    Blinded { source: &'static str },
}

/// Every column B4 protects. The single list the conversion, the counter and
/// the tests all read from, so a column added to [`fields`] and forgotten here
/// would show up as a plaintext count that never reaches zero.
fn columns() -> Vec<Column> {
    vec![
        Column {
            table: "meetings",
            key: "id",
            name: "title",
            field: fields::MEETING_TITLE,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcripts",
            key: "id",
            name: "transcript",
            field: fields::TRANSCRIPT_TEXT,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcripts",
            key: "id",
            name: "speaker",
            field: fields::TRANSCRIPT_SPEAKER,
            kind: Kind::Joinable,
        },
        Column {
            table: "transcripts",
            key: "id",
            name: "summary",
            field: fields::TRANSCRIPT_SUMMARY,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcripts",
            key: "id",
            name: "action_items",
            field: fields::TRANSCRIPT_ACTION_ITEMS,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcripts",
            key: "id",
            name: "key_points",
            field: fields::TRANSCRIPT_KEY_POINTS,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcripts",
            key: "id",
            name: "words",
            field: fields::TRANSCRIPT_WORDS,
            kind: Kind::Sealed,
        },
        Column {
            table: "privacy_settings",
            key: "id",
            name: "hidden_terms",
            field: fields::PRIVACY_HIDDEN_TERMS,
            kind: Kind::Sealed,
        },
        Column {
            table: "summary_processes",
            key: "meeting_id",
            name: "result",
            field: fields::SUMMARY_RESULT,
            kind: Kind::Sealed,
        },
        Column {
            table: "summary_processes",
            key: "meeting_id",
            name: "result_backup",
            field: fields::SUMMARY_RESULT_BACKUP,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcript_chunks",
            key: "meeting_id",
            name: "transcript_text",
            field: fields::CHUNK_TEXT,
            kind: Kind::Sealed,
        },
        Column {
            table: "transcript_chunks",
            key: "meeting_id",
            name: "meeting_name",
            field: fields::CHUNK_MEETING_NAME,
            kind: Kind::Sealed,
        },
        Column {
            table: "clients",
            key: "id",
            name: "display_name",
            field: fields::CLIENT_DISPLAY_NAME,
            kind: Kind::Sealed,
        },
        Column {
            table: "clients",
            key: "id",
            name: "notes",
            field: fields::CLIENT_NOTES,
            kind: Kind::Sealed,
        },
        Column {
            table: "clients",
            key: "id",
            name: "normalized_name",
            field: fields::CLIENT_LOOKUP,
            kind: Kind::Blinded {
                source: "display_name",
            },
        },
        Column {
            table: "people",
            key: "id",
            name: "display_name",
            field: fields::PERSON_NAME,
            kind: Kind::Sealed,
        },
        Column {
            table: "people",
            key: "id",
            name: "notes",
            field: fields::PERSON_NOTES,
            kind: Kind::Sealed,
        },
        Column {
            table: "people",
            key: "id",
            name: "normalized_name",
            field: fields::PERSON_LOOKUP,
            kind: Kind::Blinded {
                source: "display_name",
            },
        },
        Column {
            table: "person_speakers",
            key: "rowid",
            name: "speaker_label",
            field: fields::SPEAKER_LABEL,
            kind: Kind::Joinable,
        },
        Column {
            table: "meeting_speaker_roles",
            key: "rowid",
            name: "speaker_label",
            field: fields::SPEAKER_LABEL,
            kind: Kind::Joinable,
        },
    ]
}

/// Counts values that are protected and values that are not.
///
/// A blinded lookup column counts as protected when it holds a blind index. A
/// `NULL` counts as neither: there is nothing there to read.
pub async fn count(pool: &SqlitePool) -> Result<FieldCounts, sqlx::Error> {
    let mut counts = FieldCounts::default();
    for column in columns() {
        let rows: Vec<Option<String>> = sqlx::query_scalar(&format!(
            "SELECT {} FROM {}",
            column.name, column.table
        ))
        .fetch_all(pool)
        .await?;
        for value in rows.into_iter().flatten() {
            let protected = match column.kind {
                Kind::Blinded { .. } => field::is_blinded(&value),
                _ => field::is_sealed(&value),
            };
            if protected {
                counts.sealed += 1;
            } else {
                counts.plaintext += 1;
            }
        }
    }
    Ok(counts)
}

/// Writes a copy of the whole database to `destination`, unless a file is
/// already there.
///
/// `VACUUM INTO` rather than copying the file: the database runs in WAL mode,
/// so the `.sqlite` file on its own can be missing the most recent
/// transactions. This asks SQLite for a consistent copy instead of guessing
/// which three files to take together.
///
/// Refusing to overwrite is deliberate. The copy exists to hold the archive as
/// it was *before* B4 touched it; a second run would replace that with the
/// converted state and quietly destroy the thing being kept.
pub async fn back_up(pool: &SqlitePool, destination: &Path) -> Result<bool, sqlx::Error> {
    if destination.exists() {
        return Ok(false);
    }
    // VACUUM INTO takes no bound parameters, so the path goes in as a quoted
    // literal with its own quotes doubled, the way SQLite escapes them.
    let quoted = destination.to_string_lossy().replace('\'', "''");
    sqlx::query(&format!("VACUUM INTO '{quoted}'"))
        .execute(pool)
        .await?;
    Ok(true)
}

/// Converts every listed column, in one transaction.
///
/// Needs the archive to be unlocked: `Encrypt` has to seal with the key and
/// `Decrypt` has to open with it. Returns an error with nothing written if any
/// value fails, which is the property "remove the password" depends on.
pub async fn convert_all(
    pool: &SqlitePool,
    direction: Direction,
) -> Result<FieldReport, sqlx::Error> {
    let mut report = FieldReport::default();
    let mut tx = pool.begin().await?;

    for column in columns() {
        convert_column(&mut tx, &column, direction, &mut report).await?;
    }

    tx.commit().await?;
    Ok(report)
}

async fn convert_column(
    tx: &mut Transaction<'_, Sqlite>,
    column: &Column,
    direction: Direction,
    report: &mut FieldReport,
) -> Result<(), sqlx::Error> {
    // A blinded column is rebuilt from the name it was derived from, so both
    // are read together.
    let select = match column.kind {
        Kind::Blinded { source } => format!(
            "SELECT {} AS row_key, {} AS value, {} AS source FROM {}",
            column.key, column.name, source, column.table
        ),
        _ => format!(
            "SELECT {} AS row_key, {} AS value, NULL AS source FROM {}",
            column.key, column.name, column.table
        ),
    };

    let rows = sqlx::query(&select).fetch_all(&mut **tx).await?;

    let mut updates: Vec<(String, String)> = Vec::new();
    for row in rows {
        let key: String = row.try_get::<String, _>("row_key").or_else(|_| {
            // `person_speakers` has no text id, so it is addressed by rowid.
            row.try_get::<i64, _>("row_key").map(|id| id.to_string())
        })?;
        let Some(value) = row.try_get::<Option<String>, _>("value")? else {
            continue;
        };
        let source: Option<String> = row.try_get("source")?;

        let protected = match column.kind {
            Kind::Blinded { .. } => field::is_blinded(&value),
            _ => field::is_sealed(&value),
        };
        let wants_change = match direction {
            Direction::Encrypt => !protected,
            Direction::Decrypt => protected,
        };
        if !wants_change {
            report.untouched += 1;
            continue;
        }

        let next = match (direction, column.kind) {
            (Direction::Encrypt, Kind::Sealed) => fields::seal(column.field, &value)?,
            (Direction::Encrypt, Kind::Joinable) => fields::seal_joinable(column.field, &value)?,
            (Direction::Encrypt, Kind::Blinded { .. }) => {
                fields::lookup(column.field, &normalize(&value))?
            }
            (Direction::Decrypt, Kind::Sealed | Kind::Joinable) => {
                fields::open(column.field, &value)?
            }
            (Direction::Decrypt, Kind::Blinded { .. }) => {
                // The index itself says nothing. The name it was made from is
                // in this same row, and by now it has already been decrypted by
                // an earlier column in the list.
                let name = source.unwrap_or_default();
                normalize(&fields::open(column.field_source(), &name)?)
            }
        };

        // With no archive key every transform is the identity, and rewriting
        // a row to the value it already holds is not a conversion. Comparing
        // is also what keeps the count honest on a second pass.
        if next == value {
            report.untouched += 1;
            continue;
        }
        updates.push((key, next));
    }

    for (key, value) in updates {
        // `rowid` is a valid column name in a WHERE clause for any table that
        // has one, so the same statement shape serves both kinds of key.
        sqlx::query(&format!(
            "UPDATE {} SET {} = ? WHERE {} = ?",
            column.table, column.name, column.key
        ))
        .bind(value)
        .bind(key)
        .execute(&mut **tx)
        .await?;
        report.converted += 1;
    }

    Ok(())
}

impl Column {
    /// The column a blinded value's source name is sealed under, so the reverse
    /// pass can open it. Only meaningful for [`Kind::Blinded`].
    fn field_source(&self) -> Field {
        match self.table {
            "clients" => fields::CLIENT_DISPLAY_NAME,
            _ => fields::PERSON_NAME,
        }
    }
}

/// The same folding the repositories use for a lookup value. Kept here as a
/// one-liner rather than imported from two different repositories, which
/// disagree about nothing but where the function lives.
fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool_with_rows() -> SqlitePool {
        let pool = SqlitePool::connect(":memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT NOT NULL); \
             CREATE TABLE transcripts (id TEXT PRIMARY KEY, transcript TEXT NOT NULL, \
                 speaker TEXT, summary TEXT, action_items TEXT, key_points TEXT, words TEXT); \
             CREATE TABLE summary_processes (meeting_id TEXT PRIMARY KEY, result TEXT, \
                 result_backup TEXT); \
             CREATE TABLE transcript_chunks (meeting_id TEXT PRIMARY KEY, \
                 transcript_text TEXT NOT NULL, meeting_name TEXT); \
             CREATE TABLE clients (id TEXT PRIMARY KEY, display_name TEXT NOT NULL, \
                 normalized_name TEXT NOT NULL, notes TEXT); \
             CREATE TABLE people (id TEXT PRIMARY KEY, display_name TEXT NOT NULL, \
                 normalized_name TEXT NOT NULL, notes TEXT); \
             CREATE TABLE person_speakers (person_id TEXT NOT NULL, meeting_id TEXT NOT NULL, \
                 speaker_label TEXT NOT NULL); \
             CREATE TABLE meeting_speaker_roles (meeting_id TEXT NOT NULL, \
                 speaker_label TEXT NOT NULL, role TEXT NOT NULL); \
             CREATE TABLE privacy_settings (id TEXT PRIMARY KEY, \
                 anonymize_cloud INTEGER NOT NULL DEFAULT 1, hidden_terms TEXT); \
             INSERT INTO meetings VALUES ('m1', 'Встреча'); \
             INSERT INTO transcripts VALUES ('t1', 'первая реплика', 'Анна', NULL, NULL, NULL, \
                 '[{\"w\":\"первая\",\"s\":0.0,\"e\":0.4}]'); \
             INSERT INTO clients VALUES ('c1', 'Анна', 'анна', NULL); \
             INSERT INTO people VALUES ('p1', 'Анна', 'анна', NULL); \
             INSERT INTO person_speakers VALUES ('p1', 'm1', 'Анна');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn without_a_key_a_pass_changes_nothing() {
        // The state the library tests run in. Encrypting would be a no-op that
        // still rewrote every row, so it must not count anything as converted.
        let pool = pool_with_rows().await;
        let report = convert_all(&pool, Direction::Encrypt).await.unwrap();
        assert_eq!(report.converted, 0);
        assert!(report.untouched > 0);

        let title: String = sqlx::query_scalar("SELECT title FROM meetings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(title, "Встреча");
    }

    #[tokio::test]
    async fn counting_finds_the_values_that_are_still_plaintext() {
        let pool = pool_with_rows().await;
        let counts = count(&pool).await.unwrap();
        assert_eq!(counts.sealed, 0);
        // Six names and lines, one line's word timings, plus the two lookup
        // columns.
        assert_eq!(counts.plaintext, 9);
    }

    #[tokio::test]
    async fn a_null_is_neither_sealed_nor_plaintext() {
        let pool = pool_with_rows().await;
        let before = count(&pool).await.unwrap();
        sqlx::query("UPDATE clients SET notes = NULL")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(count(&pool).await.unwrap(), before);
    }
}
