//! Corrections to a transcript, and keeping them.
//!
//! A person can rewrite a line or remove it. Both are decisions about what was
//! said, and re-recognition — which replaces the whole transcript — must not
//! undo them:
//!
//! * an edited line is marked (`edited_at`) and survives re-recognition as it
//!   is; nothing new is written over its stretch of time on its track;
//! * a removed line is deleted, and only its place is kept
//!   (`transcript_removals`), so the same words are not recognized back into
//!   it.
//!
//! "Its track" matters because the two sides are recorded separately and
//! overlap: correcting what the client said must not stop the host's words at
//! the same moment from being recognized.
//!
//! An edited line keeps its word timings, carried over to the new text by
//! [`crate::audio::word_timing::realign`], so playback still marks words in it.

use chrono::Utc;
use serde::Serialize;
use sqlx::{Error as SqlxError, Row, Sqlite, SqlitePool, Transaction};
use uuid::Uuid;

use crate::audio::word_timing::{realign, to_json, WordTiming};
use crate::database::fields;
use crate::database::repositories::speaker_role::is_local_speaker;
use crate::state::AppState;

/// Which recorded track a line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    /// The local microphone.
    Mic,
    /// Everything that came through the speakers.
    System,
    /// Not known — a recording without separate tracks. Matches both.
    Any,
}

impl Track {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mic => "mic",
            Self::System => "system",
            Self::Any => "any",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "mic" => Self::Mic,
            "system" => Self::System,
            _ => Self::Any,
        }
    }

    /// The track a stored line came from, judged by its speaker label.
    pub fn of_speaker(label: Option<&str>) -> Self {
        match label {
            Some(label) if is_local_speaker(label) => Self::Mic,
            Some(_) => Self::System,
            None => Self::Any,
        }
    }

    fn meets(self, other: Track) -> bool {
        self == Track::Any || other == Track::Any || self == other
    }
}

/// A stretch of a track a person has decided about.
#[derive(Debug, Clone, PartialEq)]
pub struct ProtectedSpan {
    pub start: f64,
    pub end: f64,
    pub track: Track,
}

/// A recognized line about to be saved.
#[derive(Debug, Clone, PartialEq)]
pub struct NewLine {
    pub text: String,
    pub start: f64,
    pub end: f64,
    pub track: Track,
    pub words: Option<Vec<WordTiming>>,
}

/// What is left of a recognized line once the protected stretches are taken
/// out of it.
///
/// With word timings, the words said inside a protected stretch are dropped
/// and the rest kept — as one line, or several if the stretch fell in the
/// middle. Without them there is no telling which words those were, so a line
/// mostly inside a protected stretch is dropped and one mostly outside kept.
pub fn outside_protected(line: NewLine, spans: &[ProtectedSpan]) -> Vec<NewLine> {
    let relevant: Vec<&ProtectedSpan> = spans
        .iter()
        .filter(|span| span.track.meets(line.track) && span.start < line.end && span.end > line.start)
        .collect();
    if relevant.is_empty() {
        return vec![line];
    }

    let Some(words) = line.words.clone().filter(|words| !words.is_empty()) else {
        let length = (line.end - line.start).max(f64::EPSILON);
        let covered: f64 = relevant
            .iter()
            .map(|span| (span.end.min(line.end) - span.start.max(line.start)).max(0.0))
            .sum();
        return if covered / length > 0.5 { Vec::new() } else { vec![line] };
    };

    let inside = |word: &WordTiming| {
        let middle = (word.start + word.end) / 2.0;
        relevant.iter().any(|span| span.start <= middle && middle < span.end)
    };
    let mut runs: Vec<Vec<WordTiming>> = Vec::new();
    let mut current: Vec<WordTiming> = Vec::new();
    for word in words {
        if inside(&word) {
            if !current.is_empty() {
                runs.push(std::mem::take(&mut current));
            }
        } else {
            current.push(word);
        }
    }
    if !current.is_empty() {
        runs.push(current);
    }
    runs.into_iter()
        .map(|run| NewLine {
            text: run.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
            start: run.first().map(|w| w.start).unwrap_or(line.start).max(line.start),
            end: run.last().map(|w| w.end).unwrap_or(line.end).min(line.end),
            track: line.track,
            words: Some(run),
        })
        .collect()
}

/// What an edit left behind, for the interface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditedLine {
    pub id: String,
    pub text: String,
    pub edited_at: String,
    pub words: Option<Vec<WordTiming>>,
}

pub struct TranscriptEditsRepository;

impl TranscriptEditsRepository {
    /// Replace a line's text and mark it as a person's.
    pub async fn edit(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_id: &str,
        text: &str,
    ) -> Result<EditedLine, SqlxError> {
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            return Err(SqlxError::Protocol(
                "an edited line cannot be empty; remove it instead".into(),
            ));
        }

        let row = sqlx::query(
            "SELECT words, audio_start_time, audio_end_time FROM transcripts \
             WHERE id = ? AND meeting_id = ?",
        )
        .bind(transcript_id)
        .bind(meeting_id)
        .fetch_optional(pool)
        .await?
        .ok_or(SqlxError::RowNotFound)?;

        let old_words: Vec<WordTiming> = row
            .try_get::<Option<String>, _>("words")?
            .and_then(|stored| fields::open(fields::TRANSCRIPT_WORDS, &stored).ok())
            .and_then(|json| crate::audio::word_timing::from_json(&json))
            .unwrap_or_default();
        let start: f64 = row.try_get::<Option<f64>, _>("audio_start_time")?.unwrap_or(0.0);
        let end: f64 = row.try_get::<Option<f64>, _>("audio_end_time")?.unwrap_or(start);
        let words = realign(&old_words, &text, start, end);
        let words = (!words.is_empty()).then_some(words);
        let edited_at = Utc::now().to_rfc3339();

        sqlx::query(
            "UPDATE transcripts SET transcript = ?, words = ?, edited_at = ? \
             WHERE id = ? AND meeting_id = ?",
        )
        .bind(fields::seal(fields::TRANSCRIPT_TEXT, &text))
        .bind(
            words
                .as_deref()
                .map(|words| fields::seal(fields::TRANSCRIPT_WORDS, &to_json(words))),
        )
        .bind(&edited_at)
        .bind(transcript_id)
        .bind(meeting_id)
        .execute(pool)
        .await?;

        Ok(EditedLine {
            id: transcript_id.to_string(),
            text,
            edited_at,
            words,
        })
    }

    /// Remove a line, keeping only where it was.
    pub async fn remove(
        pool: &SqlitePool,
        meeting_id: &str,
        transcript_id: &str,
    ) -> Result<(), SqlxError> {
        let mut tx = pool.begin().await?;
        let row = sqlx::query(
            "SELECT speaker, audio_start_time, audio_end_time FROM transcripts \
             WHERE id = ? AND meeting_id = ?",
        )
        .bind(transcript_id)
        .bind(meeting_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(SqlxError::RowNotFound)?;
        let speaker = fields::open_opt(fields::TRANSCRIPT_SPEAKER, row.try_get("speaker")?)?;
        let start: Option<f64> = row.try_get("audio_start_time")?;
        let end: Option<f64> = row.try_get("audio_end_time")?;

        // A line with no place in the recording has nothing to protect.
        if let (Some(start), Some(end)) = (start, end) {
            sqlx::query(
                "INSERT INTO transcript_removals \
                 (id, meeting_id, audio_start_time, audio_end_time, track, removed_at) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(format!("removal-{}", Uuid::new_v4()))
            .bind(meeting_id)
            .bind(start)
            .bind(end.max(start))
            .bind(Track::of_speaker(speaker.as_deref()).as_str())
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query("DELETE FROM transcripts WHERE id = ? AND meeting_id = ?")
            .bind(transcript_id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    /// Every stretch of a meeting a person has decided about.
    pub(crate) async fn protected_spans(
        tx: &mut Transaction<'_, Sqlite>,
        meeting_id: &str,
    ) -> Result<Vec<ProtectedSpan>, SqlxError> {
        let mut spans = Vec::new();
        let edited = sqlx::query(
            "SELECT speaker, audio_start_time, audio_end_time FROM transcripts \
             WHERE meeting_id = ? AND edited_at IS NOT NULL",
        )
        .bind(meeting_id)
        .fetch_all(&mut **tx)
        .await?;
        for row in edited {
            let speaker = fields::open_opt(fields::TRANSCRIPT_SPEAKER, row.try_get("speaker")?)?;
            if let (Some(start), Some(end)) = (
                row.try_get::<Option<f64>, _>("audio_start_time")?,
                row.try_get::<Option<f64>, _>("audio_end_time")?,
            ) {
                spans.push(ProtectedSpan {
                    start,
                    end: end.max(start),
                    track: Track::of_speaker(speaker.as_deref()),
                });
            }
        }
        let removed = sqlx::query(
            "SELECT audio_start_time, audio_end_time, track FROM transcript_removals \
             WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(&mut **tx)
        .await?;
        for row in removed {
            spans.push(ProtectedSpan {
                start: row.try_get("audio_start_time")?,
                end: row.try_get("audio_end_time")?,
                track: Track::parse(&row.try_get::<String, _>("track")?),
            });
        }
        Ok(spans)
    }

    /// Before re-recognition writes its lines: drop the machine's lines and
    /// keep the person's, giving each kept line the capture label of its track
    /// so the transcript reads consistently until it is diarized again.
    pub(crate) async fn clear_machine_lines(
        tx: &mut Transaction<'_, Sqlite>,
        meeting_id: &str,
        tracks_known: bool,
    ) -> Result<(), SqlxError> {
        sqlx::query("DELETE FROM transcripts WHERE meeting_id = ? AND edited_at IS NULL")
            .bind(meeting_id)
            .execute(&mut **tx)
            .await?;
        if !tracks_known {
            return Ok(());
        }
        let kept = sqlx::query("SELECT id, speaker FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_all(&mut **tx)
            .await?;
        for row in kept {
            let id: String = row.try_get("id")?;
            let speaker = fields::open_opt(fields::TRANSCRIPT_SPEAKER, row.try_get("speaker")?)?;
            let label = match Track::of_speaker(speaker.as_deref()) {
                Track::Mic => "You",
                _ => "Guest",
            };
            sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ?")
                .bind(fields::seal_joinable(fields::TRANSCRIPT_SPEAKER, label))
                .bind(&id)
                .execute(&mut **tx)
                .await?;
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn api_edit_transcript_line(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    transcript_id: String,
    text: String,
) -> Result<EditedLine, String> {
    TranscriptEditsRepository::edit(state.db_manager.pool(), &meeting_id, &transcript_id, &text)
        .await
        .map_err(|error| match error {
            SqlxError::RowNotFound => "Transcript line not found".to_string(),
            other => format!("Failed to edit transcript line: {other}"),
        })
}

#[tauri::command]
pub async fn api_remove_transcript_line(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    transcript_id: String,
) -> Result<(), String> {
    TranscriptEditsRepository::remove(state.db_manager.pool(), &meeting_id, &transcript_id)
        .await
        .map_err(|error| match error {
            SqlxError::RowNotFound => "Transcript line not found".to_string(),
            other => format!("Failed to remove transcript line: {other}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn word(text: &str, start: f64, end: f64) -> WordTiming {
        WordTiming { text: text.into(), start, end }
    }

    fn line(start: f64, end: f64, track: Track, words: Option<Vec<WordTiming>>) -> NewLine {
        let text = words
            .as_ref()
            .map(|w| w.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "))
            .unwrap_or_else(|| "text".into());
        NewLine { text, start, end, track, words }
    }

    fn span(start: f64, end: f64, track: Track) -> ProtectedSpan {
        ProtectedSpan { start, end, track }
    }

    #[test]
    fn a_line_away_from_any_decision_is_kept_whole() {
        let new = line(0.0, 2.0, Track::Mic, None);
        assert_eq!(outside_protected(new.clone(), &[span(5.0, 6.0, Track::Mic)]), vec![new]);
    }

    /// Correcting the client must not stop the host being recognized.
    #[test]
    fn a_decision_on_one_track_leaves_the_other_alone() {
        let new = line(0.0, 2.0, Track::Mic, None);
        assert_eq!(outside_protected(new.clone(), &[span(0.0, 2.0, Track::System)]), vec![new]);
    }

    #[test]
    fn a_recording_without_tracks_is_protected_by_any_decision() {
        let new = line(0.0, 2.0, Track::Any, None);
        assert!(outside_protected(new, &[span(0.0, 2.0, Track::System)]).is_empty());
    }

    #[test]
    fn with_words_only_the_words_inside_are_dropped() {
        let words = vec![
            word("раз", 0.0, 0.5),
            word("два", 0.5, 1.0),
            word("три", 1.0, 1.5),
            word("четыре", 1.5, 2.0),
        ];
        let kept = outside_protected(line(0.0, 2.0, Track::Mic, Some(words)), &[span(0.4, 1.4, Track::Mic)]);
        let texts: Vec<&str> = kept.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, ["раз", "четыре"], "the stretch in the middle splits the line");
        assert_eq!((kept[1].start, kept[1].end), (1.5, 2.0));
    }

    #[test]
    fn without_words_a_mostly_covered_line_is_dropped() {
        assert!(outside_protected(line(0.0, 2.0, Track::Mic, None), &[span(0.0, 1.5, Track::Mic)]).is_empty());
        assert_eq!(
            outside_protected(line(0.0, 2.0, Track::Mic, None), &[span(0.0, 0.5, Track::Mic)]).len(),
            1
        );
    }

    #[test]
    fn a_speaker_label_names_its_track() {
        assert_eq!(Track::of_speaker(Some("You")), Track::Mic);
        assert_eq!(Track::of_speaker(Some("Speaker 2")), Track::System);
        assert_eq!(Track::of_speaker(None), Track::Any);
    }

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE meetings (id TEXT PRIMARY KEY); \
             CREATE TABLE transcripts (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, \
                 transcript TEXT NOT NULL, speaker TEXT, audio_start_time REAL, \
                 audio_end_time REAL, words TEXT); \
             INSERT INTO meetings VALUES ('m1'); \
             INSERT INTO transcripts VALUES \
                 ('t1', 'm1', 'отчет будет готов', 'Guest', 1.0, 2.5, \
                  '[{\"w\":\"отчет\",\"s\":1.0,\"e\":1.4},{\"w\":\"будет\",\"s\":1.4,\"e\":1.9},{\"w\":\"готов\",\"s\":1.9,\"e\":2.5}]'), \
                 ('t2', 'm1', 'угу', 'You', 1.5, 1.8, NULL);",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../../../migrations/20260918000000_add_transcript_edits.sql"))
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn an_edit_replaces_the_text_and_keeps_the_timings_it_can() {
        let pool = pool().await;
        let edited = TranscriptEditsRepository::edit(&pool, "m1", "t1", "  Отчёт  будет готов в пятницу ")
            .await
            .unwrap();
        assert_eq!(edited.text, "Отчёт будет готов в пятницу");
        let words = edited.words.expect("timings carried over");
        assert_eq!(words.len(), 5);
        assert_eq!((words[1].start, words[1].end), (1.4, 1.9), "an untouched word keeps its time");

        let row = sqlx::query("SELECT transcript, edited_at FROM transcripts WHERE id = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("transcript"), "Отчёт будет готов в пятницу");
        assert!(row.get::<Option<String>, _>("edited_at").is_some());
    }

    #[tokio::test]
    async fn an_empty_edit_is_refused_and_a_foreign_line_is_not_found() {
        let pool = pool().await;
        assert!(TranscriptEditsRepository::edit(&pool, "m1", "t1", "   ").await.is_err());
        assert!(matches!(
            TranscriptEditsRepository::edit(&pool, "other", "t1", "x").await,
            Err(SqlxError::RowNotFound)
        ));
    }

    #[tokio::test]
    async fn decisions_survive_the_clearing_before_re_recognition() {
        let pool = pool().await;
        TranscriptEditsRepository::edit(&pool, "m1", "t1", "исправлено").await.unwrap();
        TranscriptEditsRepository::remove(&pool, "m1", "t2").await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript) VALUES ('t3', 'm1', 'машина')")
            .execute(&mut *tx)
            .await
            .unwrap();
        let spans = TranscriptEditsRepository::protected_spans(&mut tx, "m1").await.unwrap();
        TranscriptEditsRepository::clear_machine_lines(&mut tx, "m1", true).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(
            spans,
            vec![span(1.0, 2.5, Track::System), span(1.5, 1.8, Track::Mic)],
            "the edited line and the removed one are both protected"
        );
        let left: Vec<(String, String)> =
            sqlx::query_as("SELECT id, transcript FROM transcripts ORDER BY id")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(left, vec![("t1".into(), "исправлено".into())]);
    }
}
