//! Bringing a transfer package into this installation's library.
//!
//! The package's recordings are added beside what is here; nothing already
//! in the library is changed. A recording whose id this library already has
//! is left alone, so importing the same package twice does no harm. Clients
//! and people are matched by name — the ids of another installation mean
//! nothing here.
//!
//! Everything lands the way this installation stores its own recordings: a
//! new opaque folder, audio sealed with this archive's key (through
//! [`AudioSink`]), text columns sealed through the fields layer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use super::sealed::SealedPackage;
use super::{ClientRecord, MeetingEntry, MeetingRecord};
use crate::audio::encrypted_audio::AudioSink;
use crate::audio::word_timing::to_json;
use crate::database::fields;
use crate::database::repositories::client::ClientsRepository;
use crate::database::repositories::person::link_speaker_to_person;

/// What happened to one recording of the package.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Imported,
    /// This library already has a recording with that id.
    AlreadyPresent,
}

/// This library's id for each client of the package, found or created by name.
pub async fn match_clients(
    pool: &SqlitePool,
    clients: &[ClientRecord],
) -> Result<HashMap<String, String>, String> {
    let mut matched = HashMap::with_capacity(clients.len());
    for client in clients {
        if client.name.trim().is_empty() {
            continue;
        }
        let (id, created) = ClientsRepository::find_or_create(pool, &client.name)
            .await
            .map_err(|error| format!("Could not add a client: {error}"))?;
        // Notes come along only for a client this library did not have: the
        // owner's notes on an existing one are not overwritten.
        if created {
            if let Some(notes) = client.notes.as_deref() {
                ClientsRepository::update_notes(pool, &id, Some(notes))
                    .await
                    .map_err(|error| format!("Could not add a client's notes: {error}"))?;
            }
        }
        matched.insert(client.id.clone(), id);
    }
    Ok(matched)
}

/// Adds one recording of the package, or says why it was not.
pub async fn import_meeting(
    pool: &SqlitePool,
    package: Arc<SealedPackage>,
    entry: &MeetingEntry,
    clients: &HashMap<String, String>,
    recordings_root: &Path,
) -> Result<Outcome, String> {
    let present: Option<i64> = sqlx::query_scalar("SELECT 1 FROM meetings WHERE id = ?")
        .bind(&entry.id)
        .fetch_optional(pool)
        .await
        .map_err(|error| format!("Could not check the library: {error}"))?;
    if present.is_some() {
        return Ok(Outcome::AlreadyPresent);
    }

    let record = package.read_meeting(entry)?;
    if record.id != entry.id {
        return Err("A recording in the package does not match its listing".to_string());
    }

    let folder = crate::audio::audio_processing::create_meeting_folder(&recordings_root.to_path_buf())
        .map_err(|error| format!("Could not create a recording folder: {error}"))?;

    let copied = {
        let package = package.clone();
        let entry = entry.clone();
        let tracks = record.tracks.clone();
        let folder = folder.clone();
        tokio::task::spawn_blocking(move || copy_tracks(&package, &entry, &tracks, &folder))
            .await
            .map_err(|error| format!("The copying task failed: {error}"))?
    };
    if let Err(error) = copied {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(error);
    }

    if let Err(error) = insert_meeting(pool, &record, &folder, clients).await {
        let _ = std::fs::remove_dir_all(&folder);
        return Err(format!("Could not add a recording to the library: {error}"));
    }
    Ok(Outcome::Imported)
}

/// The package's tracks, re-sealed with this archive's key.
fn copy_tracks(
    package: &SealedPackage,
    entry: &MeetingEntry,
    tracks: &[String],
    folder: &Path,
) -> Result<(), String> {
    for name in tracks {
        let mut reader = package.open_track(entry, name)?;
        let mut sink = AudioSink::create(&folder.join(name))
            .map_err(|error| format!("Could not write a track: {error}"))?;
        std::io::copy(&mut reader, &mut sink).map_err(|error| format!("Could not copy a track: {error}"))?;
        sink.finish().map_err(|error| format!("Could not finish a track: {error}"))?;
    }
    Ok(())
}

fn parse_time(stored: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(stored)
        .map(|time| time.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}

async fn insert_meeting(
    pool: &SqlitePool,
    record: &MeetingRecord,
    folder: &PathBuf,
    clients: &HashMap<String, String>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path, title_is_manual, client_id) \
         VALUES (?, ?, ?, ?, ?, 1, ?)",
    )
    .bind(&record.id)
    .bind(fields::seal(fields::MEETING_TITLE, &record.title)?)
    .bind(parse_time(&record.created_at))
    .bind(parse_time(&record.updated_at))
    .bind(folder.to_string_lossy().to_string())
    .bind(record.client_id.as_ref().and_then(|id| clients.get(id)))
    .execute(&mut *tx)
    .await?;

    for line in &record.lines {
        sqlx::query(
            "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, \
             audio_end_time, duration, speaker, words, edited_at, source_track, speaker_set_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&line.id)
        .bind(&record.id)
        .bind(fields::seal(fields::TRANSCRIPT_TEXT, &line.text)?)
        .bind(&line.timestamp)
        .bind(line.start)
        .bind(line.end)
        .bind(line.duration)
        .bind(fields::seal_joinable_opt(fields::TRANSCRIPT_SPEAKER, line.speaker.as_deref())?)
        .bind(
            line.words
                .as_deref()
                .map(|words| fields::seal(fields::TRANSCRIPT_WORDS, &to_json(words)))
                .transpose()?,
        )
        .bind(&line.edited_at)
        .bind(&line.source_track)
        .bind(&line.speaker_set_at)
        .execute(&mut *tx)
        .await?;
    }

    if let Some(summary) = record.summary.as_deref() {
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO summary_processes (meeting_id, status, created_at, updated_at, result, \
             chunk_count, processing_time) VALUES (?, 'completed', ?, ?, ?, 0, 0.0)",
        )
        .bind(&record.id)
        .bind(now)
        .bind(now)
        .bind(fields::seal(fields::SUMMARY_RESULT, summary)?)
        .execute(&mut *tx)
        .await?;
    }

    for speaker in &record.speakers {
        if let Some(role) = speaker.role.as_deref().filter(|role| matches!(*role, "host" | "client")) {
            sqlx::query(
                "INSERT OR REPLACE INTO meeting_speaker_roles (meeting_id, speaker_label, role) VALUES (?, ?, ?)",
            )
            .bind(&record.id)
            .bind(fields::seal_joinable(fields::SPEAKER_LABEL, &speaker.label)?)
            .bind(role)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(person) = speaker.person.as_deref() {
            link_speaker_to_person(&mut tx, &record.id, &speaker.label, person).await?;
        }
    }

    crate::meeting_notes::store(&mut tx, &record.id, &record.notes).await?;

    tx.commit().await
}
