//! Recordings on disk that the library does not know about.
//!
//! A recording folder gets its tracks the moment capture starts; the meeting
//! row only comes at the end, when the transcript is saved. Anything that
//! breaks in between — a crash, the window closed while stopping, a save that
//! failed — leaves audio nobody can reach from the application.
//!
//! When the transcript journal survived, the recovery dialog restores the
//! meeting from it (see [`super::transcript_journal`]). This module covers the
//! rest: a folder with tracks and no transcript worth the name. Such a
//! recording is restored as a meeting with no lines, and the post-call
//! processing recognises it again from the audio.

use std::path::{Path, PathBuf};

use log::{info, warn};
use serde::Serialize;

use super::archive_encryption::rekey_folder;
use super::encrypted_audio::AudioSource;
use super::streaming_encoder::partial_path_for;
use crate::database::repositories::transcript::TranscriptsRepository;
use crate::security::envelope::Dek;
use crate::security::keystore::{Keystore, KeystoreError};
use crate::security::session;
use crate::security::stream::file_looks_encrypted;

/// Left in a folder when the owner chose not to restore it. The audio stays;
/// the folder is simply no longer offered.
pub const DISMISSED_MARKER: &str = ".restore-dismissed";

/// The tracks a recording is made of, in the order they are worth probing.
const TRACKS: [&str; 3] = ["audio.mp4", "mic.mp4", "system.mp4"];

/// What the recovery dialog is told about a recording that has only audio.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOnly {
    /// Bytes across the tracks, finished or not.
    pub size_bytes: u64,
    /// From `metadata.json`; a recording that never stopped does not have it.
    pub duration_seconds: Option<f64>,
    /// The audio opens with the key this archive holds now. `false` means it
    /// was sealed with another key — restoring it would give a meeting that
    /// cannot play and cannot be transcribed.
    pub readable: bool,
}

fn non_empty(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// Each track as it is on disk: the finished file, or the one still under its
/// temporary name when the recording never stopped.
fn track_files(folder: &Path) -> Vec<PathBuf> {
    TRACKS
        .iter()
        .map(|name| folder.join(name))
        .filter_map(|finished| {
            if non_empty(&finished) {
                return Some(finished);
            }
            let partial = partial_path_for(&finished);
            non_empty(&partial).then_some(partial)
        })
        .collect()
}

/// Pieces an older build wrote while recording, still waiting to be joined.
fn has_checkpoint_pieces(folder: &Path) -> bool {
    let root = folder.join(".checkpoints");
    let holds_files = |dir: &Path| {
        std::fs::read_dir(dir)
            .map(|entries| entries.flatten().any(|entry| entry.path().is_file()))
            .unwrap_or(false)
    };
    holds_files(&root) || holds_files(&root.join("audio"))
}

/// Whether `folder` holds a recording at all.
pub(super) fn looks_like_recording(folder: &Path) -> bool {
    !track_files(folder).is_empty() || has_checkpoint_pieces(folder)
}

/// Whether the owner already said this folder is not to be restored.
pub(super) fn is_dismissed(folder: &Path) -> bool {
    folder.join(DISMISSED_MARKER).exists()
}

/// Marks `folder` as not to be offered again.
pub(super) fn dismiss(folder: &Path) {
    if let Err(error) = std::fs::write(folder.join(DISMISSED_MARKER), b"") {
        warn!("Could not mark a recording as dismissed: {error}");
    }
}

fn metadata_duration(folder: &Path) -> Option<f64> {
    let text = std::fs::read_to_string(folder.join("metadata.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("duration_seconds")
        .and_then(|seconds| seconds.as_f64())
        .filter(|seconds| *seconds > 0.0)
}

/// Whether the audio in `path` opens with the key this archive holds.
///
/// Only the header and the first frame are read: an encrypted track refuses a
/// wrong key there already.
fn opens(path: &Path) -> bool {
    AudioSource::open(path).is_ok()
}

/// Describes a folder with audio, or `None` when there is none.
///
/// `archive_open` says whether an encrypted track can be tried at all; with
/// the archive locked, the answer would be "unreadable" for the wrong reason.
pub(super) fn probe(folder: &Path, archive_open: bool) -> Option<AudioOnly> {
    let tracks = track_files(folder);
    if tracks.is_empty() && !has_checkpoint_pieces(folder) {
        return None;
    }
    let size_bytes = tracks
        .iter()
        .filter_map(|path| std::fs::metadata(path).ok())
        .map(|meta| meta.len())
        .sum();
    let readable = match tracks.first() {
        // Pieces from an older build predate encryption.
        None => true,
        Some(track) if file_looks_encrypted(track) && !archive_open => false,
        Some(track) => opens(track),
    };
    Some(AudioOnly {
        size_bytes,
        duration_seconds: metadata_duration(folder),
        readable,
    })
}

/// Opens the key file of another archive — its `keystore.json`, or a key
/// backup exported from it — with its password or its recovery code.
///
/// The file is only read, never written back: the attempt counter it keeps
/// guards that archive's lock screen, and whoever holds a copy of the file can
/// guess offline anyway, so it is reset here rather than honoured.
fn open_foreign_key(path: &Path, secret: &str) -> Result<Dek, String> {
    let mut keystore = Keystore::load_from(path)
        .map_err(|error| format!("Could not read the key file: {error}"))?
        .ok_or_else(|| "There is no key file at that path".to_string())?;
    keystore.failed_attempts = 0;
    keystore.locked_until = None;
    match keystore.unlock_with_password(secret) {
        Ok(dek) => Ok(dek),
        Err(KeystoreError::WrongPassword) => keystore
            .unlock_with_recovery(secret)
            .map_err(|_| "The password or recovery code does not open this key file".to_string()),
        Err(error) => Err(error.to_string()),
    }
}

/// Brings a recording sealed with another archive's key under this one's, so
/// that it can be restored here. The files are re-encrypted one by one, each
/// verified before it replaces the original (see
/// [`super::archive_encryption::rekey_folder`]); a failure leaves them as they
/// were.
#[tauri::command]
pub async fn open_recording_with_other_key<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_path: String,
    keystore_path: String,
    secret: String,
) -> Result<(), String> {
    let folder = super::transcript_journal::checked_folder(&app, &folder_path).await?;
    let folder = super::transcript_journal::plain(&folder);
    if super::transcript_journal::is_active(&folder)
        || super::recording_commands::is_recording_active()
    {
        return Err("A recording is running".to_string());
    }
    if session::archive_is_protected() && !session::archive_is_open() {
        return Err("The archive is locked".to_string());
    }
    let secret = zeroize::Zeroizing::new(secret);
    let keystore_path = PathBuf::from(keystore_path);

    let task = tauri::async_runtime::spawn_blocking(move || {
        // Not to be locked half-way through rewriting a recording.
        let _busy = session::busy();
        let theirs = open_foreign_key(&keystore_path, &secret)?;

        super::streaming_encoder::recover_partial_tracks(&folder);
        let sealed = track_files(&folder)
            .into_iter()
            .find(|track| file_looks_encrypted(track))
            .ok_or_else(|| "This recording is not encrypted".to_string())?;
        if AudioSource::open_with_key(&sealed, theirs.as_ref()).is_err() {
            return Err("This key does not open this recording".to_string());
        }

        let report = match session::with_current_key(|ours| {
            rekey_folder(&folder, theirs.as_ref(), Some(ours))
        }) {
            Some(report) => report,
            // This archive has no password: its recordings are kept plain.
            None if !session::archive_is_protected() => {
                rekey_folder(&folder, theirs.as_ref(), None)
            }
            None => return Err("The archive is locked".to_string()),
        };
        if !report.is_complete() {
            for (path, error) in &report.failed {
                warn!("Could not re-encrypt {}: {error}", path.display());
            }
            return Err(format!(
                "{} file(s) could not be re-encrypted and were left as they were",
                report.failed.len()
            ));
        }
        info!("A recording from another archive was re-encrypted: {} file(s)", report.converted);
        Ok(())
    });
    task.await.map_err(|error| error.to_string())?
}

/// A recording made into a meeting again.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredRecording {
    pub meeting_id: String,
    pub folder_path: String,
}

/// Brings the tracks of an interrupted recording under their final names and
/// makes sure the one retranscription starts from can be read.
async fn prepare_audio(folder: &Path) -> Result<(), String> {
    let folder_text = folder.to_string_lossy().into_owned();
    let recovered =
        super::checkpoint_recovery::recover_audio_from_checkpoints(folder_text.clone(), 48_000)
            .await?;
    if recovered.status == "failed" {
        return Err(recovered.message);
    }
    if let Err(error) = super::checkpoint_recovery::cleanup_checkpoints(folder_text).await {
        warn!("Could not clean up checkpoints after restoring: {error}");
    }

    let audio = super::retranscription::find_audio_file(folder).map_err(|error| error.to_string())?;
    match AudioSource::open(&audio) {
        Ok(_) => Ok(()),
        Err(_) if file_looks_encrypted(&audio) => Err(
            "This recording was encrypted with a different key and cannot be opened with the current one"
                .to_string(),
        ),
        Err(error) => Err(error.to_string()),
    }
}

/// Makes a meeting of a recording that has audio on disk and no row in the
/// library. The meeting starts with no lines: the caller runs the post-call
/// processing on it, which transcribes the audio as it would after a stop.
///
/// `title` is used when the recording's journal did not keep one.
#[tauri::command]
pub async fn restore_recording_without_meeting<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, crate::state::AppState>,
    folder_path: String,
    title: String,
) -> Result<RestoredRecording, String> {
    let folder = super::transcript_journal::checked_folder(&app, &folder_path).await?;
    let folder = super::transcript_journal::plain(&folder);
    // Restoring runs a retranscription, which is not something to start while
    // capture has the processor.
    if super::transcript_journal::is_active(&folder)
        || super::recording_commands::is_recording_active()
    {
        return Err("A recording is running".to_string());
    }
    if crate::security::session::archive_is_protected()
        && !crate::security::session::archive_is_open()
    {
        return Err("The archive is locked".to_string());
    }
    let pool = state.db_manager.pool();
    let folder_text = folder.to_string_lossy().into_owned();
    if super::transcript_journal::saved_folders(pool)
        .await?
        .contains(&folder)
    {
        return Err("This recording is already in the library".to_string());
    }

    prepare_audio(&folder).await?;

    // What the recording itself remembers comes first; the caller's title is
    // a fallback, and like any default it may be replaced by a generated one.
    let journal = super::transcript_journal::header(&folder);
    let kept_title = journal
        .as_ref()
        .map(|(title, _)| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let started_at = journal
        .as_ref()
        .and_then(|(_, started)| chrono::DateTime::parse_from_rfc3339(started).ok())
        .map(|started| started.with_timezone(&chrono::Utc))
        .or_else(|| crate::api::api::recording_started_at_from_folder(&folder_text))
        .or_else(|| {
            std::fs::metadata(&folder)
                .and_then(|meta| meta.created())
                .ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
        });

    let meeting_id = TranscriptsRepository::save_transcript(
        pool,
        kept_title.as_deref().unwrap_or(&title),
        &[],
        Some(folder_text.clone()),
        started_at,
    )
    .await
    .map_err(|error| format!("Could not create the meeting: {error}"))?;
    if kept_title.is_none() {
        if let Err(error) = sqlx::query("UPDATE meetings SET title_is_manual = 0 WHERE id = ?")
            .bind(&meeting_id)
            .execute(pool)
            .await
        {
            warn!("Could not mark the restored title as generated: {error}");
        }
    }

    super::transcript_journal::remove(&folder);
    let _ = std::fs::remove_file(folder.join(DISMISSED_MARKER));
    info!("Restored a recording without a meeting as {meeting_id}");
    Ok(RestoredRecording {
        meeting_id,
        folder_path: folder_text,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_without_tracks_is_not_a_recording() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("metadata.json"), b"{}").unwrap();
        assert!(!looks_like_recording(dir.path()));
        assert!(probe(dir.path(), true).is_none());

        // An encoder that never wrote a fragment leaves an empty file.
        std::fs::write(partial_path_for(&dir.path().join("mic.mp4")), b"").unwrap();
        assert!(!looks_like_recording(dir.path()));
    }

    #[test]
    fn an_unfinished_track_counts_and_its_size_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(partial_path_for(&dir.path().join("mic.mp4")), vec![0u8; 700]).unwrap();
        std::fs::write(dir.path().join("system.mp4"), vec![0u8; 300]).unwrap();
        std::fs::write(
            dir.path().join("metadata.json"),
            br#"{"duration_seconds": 61.5}"#,
        )
        .unwrap();
        assert!(looks_like_recording(dir.path()));
        let audio = probe(dir.path(), true).unwrap();
        assert_eq!(audio.size_bytes, 1000);
        assert_eq!(audio.duration_seconds, Some(61.5));
    }

    #[test]
    fn checkpoint_pieces_from_an_older_build_count() {
        let dir = tempfile::tempdir().unwrap();
        let pieces = dir.path().join(".checkpoints").join("audio");
        std::fs::create_dir_all(&pieces).unwrap();
        std::fs::write(pieces.join("audio_chunk_000.mp4"), b"x").unwrap();
        let audio = probe(dir.path(), false).unwrap();
        assert!(audio.readable);
        assert_eq!(audio.size_bytes, 0);
    }

    #[test]
    fn another_archives_key_opens_with_its_password_or_its_recovery_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("keystore.json");
        let (mut keystore, code, dek) = Keystore::create("старый пароль", true).unwrap();
        // That archive's lock screen had been throttled; the copy is not.
        keystore.failed_attempts = 9;
        keystore.locked_until = Some(i64::MAX);
        keystore.save_to(&path).unwrap();

        assert_eq!(*open_foreign_key(&path, "старый пароль").unwrap(), *dek);
        assert_eq!(*open_foreign_key(&path, &code.unwrap()).unwrap(), *dek);
        assert!(open_foreign_key(&path, "не тот пароль").is_err());
        assert!(open_foreign_key(&dir.path().join("missing.json"), "x").is_err());
        // Only read: the file is as it was.
        let reread = Keystore::load_from(&path).unwrap().unwrap();
        assert_eq!(reread.failed_attempts, 9);
    }

    #[test]
    fn a_dismissed_folder_is_remembered() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_dismissed(dir.path()));
        dismiss(dir.path());
        assert!(is_dismissed(dir.path()));
    }
}
