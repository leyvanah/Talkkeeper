//! The whole library written out of the application.
//!
//! Two uses share one shape. A **readable** export is a folder a person can
//! open without the application: one folder per recording, its audio as it
//! was recorded, the transcript and summary as text. A **sealed** export is
//! the same content for another installation of the application, every file
//! encrypted with a password chosen for the transfer — never with this
//! installation's key, which does not leave the machine.
//!
//! Both are read through the archive's own paths: text through the
//! repositories (which open sealed columns with the session key) and audio
//! through [`AudioSource`], which decrypts as it reads. Nothing is decrypted
//! to a temporary file on the way.

pub mod commands;
pub mod plain;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use crate::audio::constants::AUDIO_EXTENSIONS;
use crate::audio::word_timing::WordTiming;
use crate::database::fields;
use crate::database::models::MeetingModel;
use crate::database::repositories::client::ClientsRepository;
use crate::database::repositories::speaker_role::SpeakerRolesRepository;
use crate::database::repositories::summary::SummaryProcessesRepository;

/// Written into every export; an importer refuses a format it does not know.
pub const FORMAT_VERSION: u32 = 1;

/// The file at the root of an export that lists everything in it.
pub const MANIFEST_NAME: &str = "library.json";
/// The file in each recording's folder with its text.
pub const MEETING_NAME: &str = "meeting.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ClientRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// One recording as the manifest lists it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingEntry {
    pub id: String,
    /// The recording's folder inside the export, with forward slashes.
    pub folder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub app_version: String,
    pub exported_at: String,
    pub clients: Vec<ClientRecord>,
    pub meetings: Vec<MeetingEntry>,
}

/// One transcript line, with everything a later import needs to rebuild it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LineRecord {
    pub id: String,
    pub text: String,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_track: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Vec<WordTiming>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_set_at: Option<String>,
}

/// A speaker of a recording: the role a person gave it, and who it is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerRecord {
    pub label: String,
    /// "host" or "client", when a person chose it rather than the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// The person in the library this label is linked to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingRecord {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    pub lines: Vec<LineRecord>,
    /// The summary as the application stores it (JSON), when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub speakers: Vec<SpeakerRecord>,
    /// Audio files of the recording, by name inside its folder.
    pub tracks: Vec<String>,
}

impl MeetingRecord {
    /// The summary's text, where the stored JSON carries one.
    pub fn summary_markdown(&self) -> Option<String> {
        let raw = self.summary.as_deref()?;
        let value: serde_json::Value = serde_json::from_str(raw).ok()?;
        value
            .get("markdown")
            .and_then(|markdown| markdown.as_str())
            .map(str::to_string)
            .filter(|markdown| !markdown.trim().is_empty())
    }
}

/// A recording read out of the archive, and where its audio lives.
pub struct CollectedMeeting {
    pub record: MeetingRecord,
    /// Each track's name in the export and its file on disk.
    pub audio: Vec<(String, PathBuf)>,
}

/// The library's clients, readable.
pub async fn clients(pool: &SqlitePool) -> Result<Vec<ClientRecord>, String> {
    let clients = ClientsRepository::list(pool)
        .await
        .map_err(|error| format!("Could not read the clients: {error}"))?;
    Ok(clients
        .into_iter()
        .map(|client| ClientRecord {
            id: client.id,
            name: client.display_name,
            notes: client.notes,
        })
        .collect())
}

/// Every recording's id, oldest first, so an export reads like the archive grew.
pub async fn meeting_ids(pool: &SqlitePool) -> Result<Vec<String>, String> {
    sqlx::query_scalar("SELECT id FROM meetings ORDER BY created_at ASC")
        .fetch_all(pool)
        .await
        .map_err(|error| format!("Could not list the recordings: {error}"))
}

/// One recording with everything it holds, opened.
pub async fn collect_meeting(pool: &SqlitePool, meeting_id: &str) -> Result<CollectedMeeting, String> {
    let meeting: MeetingModel = sqlx::query_as(
        "SELECT id, title, created_at, updated_at, folder_path, client_id FROM meetings WHERE id = ?",
    )
    .bind(meeting_id)
    .fetch_one(pool)
    .await
    .map_err(|error| format!("Could not read a recording: {error}"))?;

    let lines = lines_of(pool, meeting_id).await?;

    let summary = SummaryProcessesRepository::get_summary_data(pool, meeting_id)
        .await
        .map_err(|error| format!("Could not read a summary: {error}"))?
        .and_then(|process| process.result)
        .filter(|result| !result.trim().is_empty());

    let speakers = speakers_of(pool, meeting_id).await?;

    let audio = match meeting.folder_path.as_deref() {
        Some(folder) => audio_files(Path::new(folder)),
        None => Vec::new(),
    };

    Ok(CollectedMeeting {
        record: MeetingRecord {
            id: meeting.id,
            title: meeting.title,
            created_at: meeting.created_at.0.to_rfc3339(),
            updated_at: meeting.updated_at.0.to_rfc3339(),
            client_id: meeting.client_id,
            lines,
            summary,
            speakers,
            tracks: audio.iter().map(|(name, _)| name.clone()).collect(),
        },
        audio,
    })
}

async fn lines_of(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<LineRecord>, String> {
    let rows = sqlx::query(
        "SELECT id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker, \
         source_track, words, edited_at, speaker_set_at FROM transcripts \
         WHERE meeting_id = ? ORDER BY audio_start_time IS NULL, audio_start_time, timestamp",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("Could not read a transcript: {error}"))?;

    let mut lines = Vec::with_capacity(rows.len());
    for row in rows {
        let read = |error: sqlx::Error| format!("Could not read a transcript line: {error}");
        let words = fields::open_opt(fields::TRANSCRIPT_WORDS, row.try_get("words").map_err(read)?)
            .map_err(read)?
            .and_then(|json| serde_json::from_str::<Vec<WordTiming>>(&json).ok());
        lines.push(LineRecord {
            id: row.try_get("id").map_err(read)?,
            text: fields::open(fields::TRANSCRIPT_TEXT, &row.try_get::<String, _>("transcript").map_err(read)?)
                .map_err(read)?,
            timestamp: row.try_get("timestamp").map_err(read)?,
            start: row.try_get("audio_start_time").map_err(read)?,
            end: row.try_get("audio_end_time").map_err(read)?,
            duration: row.try_get("duration").map_err(read)?,
            speaker: fields::open_opt(fields::TRANSCRIPT_SPEAKER, row.try_get("speaker").map_err(read)?)
                .map_err(read)?,
            source_track: row.try_get("source_track").map_err(read)?,
            words,
            edited_at: row.try_get("edited_at").map_err(read)?,
            speaker_set_at: row.try_get("speaker_set_at").map_err(read)?,
        });
    }
    Ok(lines)
}

async fn speakers_of(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<SpeakerRecord>, String> {
    let sides = SpeakerRolesRepository::sides(pool, meeting_id)
        .await
        .map_err(|error| format!("Could not read the speakers: {error}"))?;

    let linked: Vec<(String, String)> = sqlx::query_as(
        "SELECT ps.speaker_label, p.display_name FROM person_speakers ps \
         JOIN people p ON p.id = ps.person_id WHERE ps.meeting_id = ?",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("Could not read who the speakers are: {error}"))?;
    let mut people = Vec::with_capacity(linked.len());
    for (label, name) in linked {
        let open = |error: sqlx::Error| format!("Could not read who the speakers are: {error}");
        people.push((
            fields::open(fields::SPEAKER_LABEL, &label).map_err(open)?,
            fields::open(fields::PERSON_NAME, &name).map_err(open)?,
        ));
    }

    Ok(sides
        .into_iter()
        .map(|side| {
            let role = side.assigned.then(|| {
                serde_json::to_value(side.side)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_default()
            });
            let person = people
                .iter()
                .find(|(label, _)| *label == side.speaker)
                .map(|(_, name)| name.clone());
            SpeakerRecord { label: side.speaker, role, person }
        })
        .collect())
}

/// The audio files of a recording folder, finished or cut short.
///
/// Only the folder's own level: the working copies under `.work` are the
/// application's scratch, not the recording. A track that never got its
/// final name is taken under the name it would have had.
pub fn audio_files(folder: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut finished = Vec::new();
    let mut partial = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0) == 0 {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()).map(str::to_string) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        let extension = |name: &str| {
            Path::new(name)
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_ascii_lowercase)
        };
        if let Some(base) = name.strip_suffix(&format!(".{}", crate::audio::streaming_encoder::PARTIAL_EXT)) {
            if extension(base).is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str())) {
                partial.push((base.to_string(), path));
            }
            continue;
        }
        if extension(&name).is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str())) {
            finished.push((name, path));
        }
    }
    for (base, path) in partial {
        if !finished.iter().any(|(name, _)| *name == base) {
            finished.push((base, path));
        }
    }
    finished.sort_by(|a, b| a.0.cmp(&b.0));
    finished
}

/// A name Windows accepts as a folder: no reserved characters, no trailing
/// dot or space, not a device name, not too long.
pub fn safe_name(name: &str, fallback: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
        "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut cleaned: String = cleaned.chars().take(80).collect();
    while cleaned.ends_with(['.', ' ']) {
        cleaned.pop();
    }
    let reserved = RESERVED.iter().any(|device| {
        cleaned.eq_ignore_ascii_case(device)
            || cleaned
                .split('.')
                .next()
                .is_some_and(|stem| stem.eq_ignore_ascii_case(device))
    });
    if cleaned.is_empty() || reserved {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// `[MM:SS]` or `[H:MM:SS]` for a moment of the recording.
pub fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes:02}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_becomes_a_folder_name_windows_accepts() {
        assert_eq!(safe_name("Встреча: план / итоги?", "x"), "Встреча план итоги");
        assert_eq!(safe_name("  точка в конце.  ", "x"), "точка в конце");
        assert_eq!(safe_name("CON", "fallback"), "fallback");
        assert_eq!(safe_name("con.txt", "fallback"), "fallback");
        assert_eq!(safe_name("***", "fallback"), "fallback");
        assert_eq!(safe_name(&"д".repeat(200), "x").chars().count(), 80);
    }

    #[test]
    fn moments_read_like_a_player_shows_them() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(83.4), "01:23");
        assert_eq!(clock(3725.0), "1:02:05");
    }

    #[test]
    fn the_summary_text_is_taken_from_the_stored_json() {
        let mut record = MeetingRecord {
            id: "m".into(),
            title: "t".into(),
            created_at: String::new(),
            updated_at: String::new(),
            client_id: None,
            lines: Vec::new(),
            summary: Some(r#"{"markdown":"итоги"}"#.into()),
            speakers: Vec::new(),
            tracks: Vec::new(),
        };
        assert_eq!(record.summary_markdown().as_deref(), Some("итоги"));
        record.summary = Some("not json".into());
        assert_eq!(record.summary_markdown(), None);
    }

    #[tokio::test]
    async fn a_recording_is_collected_with_its_lines_summary_and_speakers() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::manager::MIGRATOR.run(&pool).await.unwrap();
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("audio.mp4"), b"a").unwrap();
        sqlx::raw_sql(&format!(
            "INSERT INTO clients (id, display_name, normalized_name, created_at, updated_at) \
                 VALUES ('c1', 'Анна', 'анна', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z'); \
             INSERT INTO meetings (id, title, created_at, updated_at, folder_path, client_id) \
                 VALUES ('m1', 'Встреча', '2026-09-12T10:00:00Z', '2026-09-12T11:00:00Z', '{}', 'c1'); \
             INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker, source_track, \
                 audio_start_time, audio_end_time, words, edited_at) VALUES \
                 ('t2', 'm1', 'вторая', '10:00:05', 'Анна', 'system', 5.0, 6.0, NULL, '2026-09-12T12:00:00Z'), \
                 ('t1', 'm1', 'первая', '10:00:01', 'You', 'mic', 1.0, 2.0, \
                  '[{{\"w\":\"первая\",\"s\":1.0,\"e\":1.5}}]', NULL); \
             INSERT INTO summary_processes (meeting_id, status, created_at, updated_at, result) \
                 VALUES ('m1', 'completed', '2026-09-12T11:00:00Z', '2026-09-12T11:00:00Z', '{{\"markdown\":\"итоги\"}}'); \
             INSERT INTO meeting_speaker_roles (meeting_id, speaker_label, role) VALUES ('m1', 'Анна', 'client'); \
             INSERT INTO people (id, display_name, normalized_name, created_at, updated_at) \
                 VALUES ('p1', 'Анна', 'анна', '2026-09-01T00:00:00Z', '2026-09-01T00:00:00Z'); \
             INSERT INTO person_speakers (person_id, meeting_id, speaker_label) VALUES ('p1', 'm1', 'Анна');",
            folder.path().to_string_lossy().replace('\'', "''")
        ))
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(clients(&pool).await.unwrap()[0].name, "Анна");
        assert_eq!(meeting_ids(&pool).await.unwrap(), vec!["m1"]);

        let collected = collect_meeting(&pool, "m1").await.unwrap();
        let record = &collected.record;
        assert_eq!(record.title, "Встреча");
        assert_eq!(record.client_id.as_deref(), Some("c1"));
        let texts: Vec<&str> = record.lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, vec!["первая", "вторая"]);
        assert_eq!(record.lines[0].words.as_ref().unwrap()[0].text, "первая");
        assert_eq!(record.lines[1].source_track.as_deref(), Some("system"));
        assert!(record.lines[1].edited_at.is_some());
        assert_eq!(record.summary_markdown().as_deref(), Some("итоги"));
        let anna = record.speakers.iter().find(|speaker| speaker.label == "Анна").unwrap();
        assert_eq!(anna.role.as_deref(), Some("client"));
        assert_eq!(anna.person.as_deref(), Some("Анна"));
        let you = record.speakers.iter().find(|speaker| speaker.label == "You").unwrap();
        assert_eq!(you.role, None);
        assert_eq!(record.tracks, vec!["audio.mp4"]);
    }

    #[test]
    fn audio_is_taken_finished_first_and_cut_short_when_that_is_all() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("audio.mp4"), b"a").unwrap();
        std::fs::write(dir.path().join("audio.mp4.part"), b"old").unwrap();
        std::fs::write(dir.path().join("mic.mp4.part"), b"m").unwrap();
        std::fs::write(dir.path().join("metadata.json"), b"{}").unwrap();
        std::fs::write(dir.path().join("empty.wav"), b"").unwrap();
        std::fs::create_dir(dir.path().join(".work")).unwrap();
        std::fs::write(dir.path().join(".work").join("mic.flac"), b"w").unwrap();

        let names: Vec<String> = audio_files(dir.path()).into_iter().map(|(name, _)| name).collect();
        assert_eq!(names, vec!["audio.mp4", "mic.mp4"]);
    }
}
