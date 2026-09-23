//! Undo and redo for corrections to a transcript.
//!
//! Every correction (a new text, a removal, a split, a merge, a new speaker)
//! records, before it changes anything, how the rows it is about to touch
//! stood: each one as it was, or its absence. Undoing puts them back; the
//! state it replaced becomes the step to redo. So one mechanism serves every
//! kind of correction, including ones added later, and a column added to the
//! table later is carried along without anyone remembering to.
//!
//! The history lives in memory, per meeting, for as long as the application
//! runs — the way an editor's does. What it holds is the rows as stored, so
//! the text in it is sealed like the database's.
//!
//! When the machine rewrites a meeting's transcript (re-recognition, speaker
//! identification, a speaker renamed across the meeting), the rows the
//! history remembers may be gone or changed beyond what it knew, and putting
//! them back would mix two versions of the transcript. The meeting's history
//! is dropped then ([`forget`]).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use serde::Serialize;
use sqlx::sqlite::SqliteRow;
use sqlx::{Column, Error as SqlxError, Row, Sqlite, SqlitePool, Transaction, TypeInfo, ValueRef};

use crate::state::AppState;

/// Steps kept per meeting. Enough for an evening's corrections.
const DEPTH: usize = 200;

/// One stored value, whatever its type.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// A row as stored: column names and values.
type Stored = Vec<(String, Cell)>;

/// The tables a correction touches. Named here, never taken from outside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
    Transcripts,
    Removals,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Transcripts => "transcripts",
            Self::Removals => "transcript_removals",
        }
    }
}

/// How some rows stood at one moment: each as it was, or `None` when it did
/// not exist.
#[derive(Debug, Clone, Default)]
pub(crate) struct Step {
    rows: Vec<(Table, String, Option<Stored>)>,
}

#[derive(Default)]
struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

static HISTORY: LazyLock<Mutex<HashMap<String, History>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn histories() -> std::sync::MutexGuard<'static, HashMap<String, History>> {
    HISTORY.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn cells(row: &SqliteRow) -> Result<Stored, SqlxError> {
    let mut stored = Vec::with_capacity(row.columns().len());
    for column in row.columns() {
        let index = column.ordinal();
        let raw = row.try_get_raw(index)?;
        let cell = if raw.is_null() {
            Cell::Null
        } else {
            // The storage class of this value, not the column's declared type:
            // SQLite keeps what it was given.
            match raw.type_info().name() {
                "INTEGER" => Cell::Int(row.try_get(index)?),
                "REAL" => Cell::Real(row.try_get(index)?),
                "BLOB" => Cell::Blob(row.try_get(index)?),
                _ => Cell::Text(row.try_get(index)?),
            }
        };
        stored.push((column.name().to_string(), cell));
    }
    Ok(stored)
}

async fn state_of(
    tx: &mut Transaction<'_, Sqlite>,
    table: Table,
    id: &str,
) -> Result<Option<Stored>, SqlxError> {
    let query = format!("SELECT * FROM {} WHERE id = ?", table.name());
    sqlx::query(&query)
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .map(|row| cells(&row))
        .transpose()
}

/// How the given rows stand now, inside the transaction about to change them.
/// Rows that do not exist yet — a line a split is about to add, a removal
/// about to be recorded — are remembered as absent.
pub(crate) async fn capture(
    tx: &mut Transaction<'_, Sqlite>,
    transcript_ids: &[String],
    removal_ids: &[String],
) -> Result<Step, SqlxError> {
    let mut step = Step::default();
    for (table, ids) in [(Table::Transcripts, transcript_ids), (Table::Removals, removal_ids)] {
        for id in ids {
            if step.rows.iter().any(|(t, known, _)| *t == table && known == id) {
                continue;
            }
            let state = state_of(tx, table, id).await?;
            step.rows.push((table, id.clone(), state));
        }
    }
    Ok(step)
}

/// Puts the rows back the way `step` remembers them, and returns how they
/// stood before — the step that undoes this one.
async fn apply(tx: &mut Transaction<'_, Sqlite>, step: &Step) -> Result<Step, SqlxError> {
    let mut inverse = Step::default();
    for (table, id, _) in &step.rows {
        let state = state_of(tx, *table, id).await?;
        inverse.rows.push((*table, id.clone(), state));
    }
    for (table, id, state) in &step.rows {
        match state {
            None => {
                let query = format!("DELETE FROM {} WHERE id = ?", table.name());
                sqlx::query(&query).bind(id).execute(&mut **tx).await?;
            }
            Some(stored) => {
                let columns: Vec<String> =
                    stored.iter().map(|(name, _)| format!("\"{}\"", name.replace('"', "\"\""))).collect();
                let marks = vec!["?"; stored.len()].join(", ");
                let query = format!(
                    "INSERT OR REPLACE INTO {} ({}) VALUES ({marks})",
                    table.name(),
                    columns.join(", ")
                );
                let mut statement = sqlx::query(&query);
                for (_, cell) in stored {
                    statement = match cell {
                        Cell::Null => statement.bind(None::<String>),
                        Cell::Int(value) => statement.bind(*value),
                        Cell::Real(value) => statement.bind(*value),
                        Cell::Text(value) => statement.bind(value.clone()),
                        Cell::Blob(value) => statement.bind(value.clone()),
                    };
                }
                statement.execute(&mut **tx).await?;
            }
        }
    }
    Ok(inverse)
}

/// Remembers a correction just committed. A new correction ends what could
/// be redone, as in any editor.
pub(crate) fn record(meeting_id: &str, step: Step) {
    let mut all = histories();
    let history = all.entry(meeting_id.to_string()).or_default();
    history.undo.push(step);
    if history.undo.len() > DEPTH {
        history.undo.remove(0);
    }
    history.redo.clear();
}

/// Drops a meeting's history: its transcript was rewritten by other means.
pub(crate) fn forget(meeting_id: &str) {
    histories().remove(meeting_id);
}

/// Whether there is something to undo and something to redo.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EditHistory {
    pub can_undo: bool,
    pub can_redo: bool,
}

pub fn state(meeting_id: &str) -> EditHistory {
    histories()
        .get(meeting_id)
        .map(|history| EditHistory {
            can_undo: !history.undo.is_empty(),
            can_redo: !history.redo.is_empty(),
        })
        .unwrap_or_default()
}

#[derive(Clone, Copy)]
enum Direction {
    Undo,
    Redo,
}

/// Takes one step back or forward. `false` when there was none to take.
async fn walk(pool: &SqlitePool, meeting_id: &str, direction: Direction) -> Result<bool, SqlxError> {
    let step = {
        let mut all = histories();
        let Some(history) = all.get_mut(meeting_id) else {
            return Ok(false);
        };
        let stack = match direction {
            Direction::Undo => &mut history.undo,
            Direction::Redo => &mut history.redo,
        };
        match stack.pop() {
            Some(step) => step,
            None => return Ok(false),
        }
    };

    let outcome = async {
        let mut tx = pool.begin().await?;
        let inverse = apply(&mut tx, &step).await?;
        tx.commit().await?;
        Ok::<_, SqlxError>(inverse)
    }
    .await;

    let mut all = histories();
    let history = all.entry(meeting_id.to_string()).or_default();
    match outcome {
        Ok(inverse) => {
            match direction {
                Direction::Undo => history.redo.push(inverse),
                Direction::Redo => history.undo.push(inverse),
            }
            Ok(true)
        }
        Err(error) => {
            // Nothing changed: the step goes back where it was.
            match direction {
                Direction::Undo => history.undo.push(step),
                Direction::Redo => history.redo.push(step),
            }
            Err(error)
        }
    }
}

pub async fn undo(pool: &SqlitePool, meeting_id: &str) -> Result<bool, SqlxError> {
    walk(pool, meeting_id, Direction::Undo).await
}

pub async fn redo(pool: &SqlitePool, meeting_id: &str) -> Result<bool, SqlxError> {
    walk(pool, meeting_id, Direction::Redo).await
}

#[tauri::command]
pub async fn api_undo_transcript_edit(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<EditHistory, String> {
    undo(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| format!("Failed to undo: {error}"))?;
    Ok(self::state(&meeting_id))
}

#[tauri::command]
pub async fn api_redo_transcript_edit(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<EditHistory, String> {
    redo(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| format!("Failed to redo: {error}"))?;
    Ok(self::state(&meeting_id))
}

#[tauri::command]
pub fn api_transcript_edit_history(meeting_id: String) -> EditHistory {
    state(&meeting_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE transcripts (id TEXT PRIMARY KEY, meeting_id TEXT, transcript TEXT, \
                 audio_start_time REAL, words TEXT, n INTEGER, raw BLOB); \
             CREATE TABLE transcript_removals (id TEXT PRIMARY KEY, meeting_id TEXT); \
             INSERT INTO transcripts VALUES ('a', 'h1', 'один', 1.5, NULL, 7, x'00ff');",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn all(pool: &SqlitePool) -> Vec<(String, String, f64)> {
        sqlx::query_as("SELECT id, transcript, audio_start_time FROM transcripts ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_change_is_undone_and_redone_row_for_row() {
        let pool = pool().await;
        let ids = ["a".to_string(), "b".to_string()];
        let removals = ["r".to_string()];

        // A correction: rewrite 'a', add 'b', record a removal.
        let mut tx = pool.begin().await.unwrap();
        let step = capture(&mut tx, &ids, &removals).await.unwrap();
        sqlx::raw_sql(
            "UPDATE transcripts SET transcript = 'два', audio_start_time = 2.0 WHERE id = 'a'; \
             INSERT INTO transcripts (id, meeting_id, transcript, audio_start_time) VALUES ('b', 'h1', 'три', 3.0); \
             INSERT INTO transcript_removals VALUES ('r', 'h1');",
        )
        .execute(&mut *tx)
        .await
        .unwrap();
        tx.commit().await.unwrap();
        record("h1", step);
        assert_eq!(state("h1"), EditHistory { can_undo: true, can_redo: false });

        assert!(undo(&pool, "h1").await.unwrap());
        assert_eq!(all(&pool).await, vec![("a".into(), "один".into(), 1.5)]);
        let raw: (i64, Vec<u8>) = sqlx::query_as("SELECT n, raw FROM transcripts WHERE id = 'a'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(raw, (7, vec![0x00, 0xff]), "every column comes back as it was");
        let removals: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transcript_removals")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(removals, 0);
        assert_eq!(state("h1"), EditHistory { can_undo: false, can_redo: true });

        assert!(redo(&pool, "h1").await.unwrap());
        assert_eq!(
            all(&pool).await,
            vec![("a".into(), "два".into(), 2.0), ("b".into(), "три".into(), 3.0)]
        );
        assert!(!redo(&pool, "h1").await.unwrap(), "nothing more to redo");

        forget("h1");
        assert_eq!(state("h1"), EditHistory::default());
        assert!(!undo(&pool, "h1").await.unwrap());
    }

    #[tokio::test]
    async fn a_new_correction_ends_what_could_be_redone() {
        let pool = pool().await;
        let ids = ["a".to_string()];
        for text in ["x", "y"] {
            let mut tx = pool.begin().await.unwrap();
            let step = capture(&mut tx, &ids, &[]).await.unwrap();
            sqlx::query("UPDATE transcripts SET transcript = ? WHERE id = 'a'")
                .bind(text)
                .execute(&mut *tx)
                .await
                .unwrap();
            tx.commit().await.unwrap();
            record("h2", step);
        }
        undo(&pool, "h2").await.unwrap();
        assert!(state("h2").can_redo);

        let mut tx = pool.begin().await.unwrap();
        let step = capture(&mut tx, &ids, &[]).await.unwrap();
        tx.commit().await.unwrap();
        record("h2", step);
        assert!(!state("h2").can_redo);
        forget("h2");
    }
}
