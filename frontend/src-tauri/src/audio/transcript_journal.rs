//! The transcript of a recording in progress, kept on disk so a crash does not
//! take it along.
//!
//! This used to live in the window's IndexedDB: every line, in the clear, in a
//! LevelDB store that only forgets deleted data at its next compaction. It
//! survived a crash — and everything else too, including the password.
//!
//! Here it is one file per recording, `transcript.journal` in the meeting
//! folder. Each line of the file is one JSON record sealed on its own, with the
//! same key and the same rule as a database column: sealed while the archive
//! has a password, plain only when it has none. Sealing line by line is what
//! makes it crash-safe — a stream cipher with frames would lose the frame that
//! was still filling when the process died, which for a transcript is all of
//! it. A torn last line is simply skipped on reading.
//!
//! The journal is deleted once the meeting is saved to the database. A journal
//! still on disk therefore means a recording that was never saved, which is
//! exactly what the recovery dialog asks for.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{info, warn};
use serde::{Deserialize, Serialize};

use super::orphan_recordings::{dismiss, is_dismissed, looks_like_recording, probe, AudioOnly};
use super::recording_saver::TranscriptSegment;
use crate::database::fields;
use crate::security::field::Field;

/// File name inside the meeting folder.
pub const JOURNAL_FILE: &str = "transcript.journal";

/// Sealing context for journal lines. Its own, so a line cannot be moved into
/// a database column and read there, or the other way round.
const JOURNAL_LINE: Field = Field::new("transcript_journal", "line");

/// One line of the journal.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Record {
    /// Written first: what the recording was called and when it began.
    Meeting { title: String, started_at: String },
    /// A line of transcript. A later record with the same `sequence_id`
    /// replaces an earlier one, as the in-memory list does.
    Segment(TranscriptSegment),
}

/// The journal of the recording now running, if any.
static CURRENT: Mutex<Option<Journal>> = Mutex::new(None);

struct Journal {
    folder: PathBuf,
    file: File,
}

impl Journal {
    fn append(&mut self, record: &Record) -> anyhow::Result<()> {
        let json = serde_json::to_string(record)?;
        let line = fields::seal(JOURNAL_LINE, &json)?;
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        Ok(())
    }
}

fn current() -> std::sync::MutexGuard<'static, Option<Journal>> {
    CURRENT.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Starts the journal for a recording that has just got its folder.
pub fn begin(folder: &Path, title: &str, started_at: &str) {
    let path = folder.join(JOURNAL_FILE);
    let opened = OpenOptions::new().create(true).append(true).open(&path);
    let mut journal = match opened {
        Ok(file) => Journal {
            folder: folder.to_path_buf(),
            file,
        },
        Err(error) => {
            warn!("Could not open the transcript journal: {error}");
            return;
        }
    };
    let header = Record::Meeting {
        title: title.to_string(),
        started_at: started_at.to_string(),
    };
    if let Err(error) = journal.append(&header) {
        warn!("Could not start the transcript journal: {error}");
        return;
    }
    *current() = Some(journal);
}

/// Adds a line of transcript to the running journal. A failure is logged and
/// otherwise ignored: the recording itself must not stop over it.
pub fn append(segment: &TranscriptSegment) {
    if let Some(journal) = current().as_mut() {
        if let Err(error) = journal.append(&Record::Segment(segment.clone())) {
            warn!("Could not add a line to the transcript journal: {error}");
        }
    }
}

/// Closes the running journal. The file stays until the meeting is saved.
pub fn end() {
    *current() = None;
}

/// The folder whose journal is being written right now, so recovery does not
/// offer a recording that is still running.
fn active_folder() -> Option<PathBuf> {
    current().as_ref().map(|journal| journal.folder.clone())
}

/// Whether `folder` is the one being recorded into right now.
pub(super) fn is_active(folder: &Path) -> bool {
    active_folder().is_some_and(|active| plain(&active) == plain(folder))
}

/// Deletes the journal in `folder` — the meeting it belongs to is saved, or
/// the owner chose to discard it.
pub fn remove(folder: &Path) {
    let path = folder.join(JOURNAL_FILE);
    match std::fs::remove_file(&path) {
        Ok(()) => info!("Transcript journal removed"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => warn!("Could not remove the transcript journal: {error}"),
    }
}

/// What a journal holds, read back.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsaved {
    pub folder_path: String,
    pub title: String,
    pub started_at: String,
    /// Modification time of the journal, milliseconds since the epoch.
    pub last_updated: i64,
    pub segments: Vec<TranscriptSegment>,
    /// Set when there is no transcript to restore, only audio: restoring then
    /// means recognising the recording again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_only: Option<AudioOnly>,
}

/// Title and start time the journal of `folder` was begun with, if it has one
/// that can be read.
pub(super) fn header(folder: &Path) -> Option<(String, String)> {
    read(folder)
        .ok()
        .flatten()
        .map(|journal| (journal.title, journal.started_at))
        .filter(|(title, started_at)| !title.is_empty() || !started_at.is_empty())
}

/// Reads a journal. Lines that do not open — torn by a crash, or sealed with a
/// key this archive no longer has — are skipped rather than failing the whole
/// file; a locked archive fails, because then nothing would open.
fn read(folder: &Path) -> anyhow::Result<Option<Unsaved>> {
    let path = folder.join(JOURNAL_FILE);
    let file = match File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let last_updated = file
        .metadata()
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);

    if crate::security::session::archive_is_protected()
        && !crate::security::session::archive_is_open()
    {
        anyhow::bail!("the archive is locked");
    }

    let mut title = String::new();
    let mut started_at = String::new();
    let mut segments: Vec<TranscriptSegment> = Vec::new();
    let mut skipped = 0usize;

    for line in BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record = fields::open(JOURNAL_LINE, line.trim())
            .ok()
            .and_then(|json| serde_json::from_str::<Record>(&json).ok());
        match record {
            Some(Record::Meeting {
                title: t,
                started_at: s,
            }) => {
                title = t;
                started_at = s;
            }
            Some(Record::Segment(segment)) => {
                match segments
                    .iter_mut()
                    .find(|existing| existing.sequence_id == segment.sequence_id)
                {
                    Some(existing) => *existing = segment,
                    None => segments.push(segment),
                }
            }
            None => skipped += 1,
        }
    }
    if skipped > 0 {
        warn!("Transcript journal: {skipped} unreadable lines skipped");
    }
    segments.sort_by_key(|segment| segment.sequence_id);

    Ok(Some(Unsaved {
        folder_path: folder.to_string_lossy().into_owned(),
        title,
        started_at,
        last_updated,
        segments,
        audio_only: None,
    }))
}

/// A canonical Windows path without its `\\?\` prefix, so it reads — and
/// compares with `meetings.folder_path` — the way the recorder wrote it.
pub(super) fn plain(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with(r"UNC\") => PathBuf::from(rest),
        _ => path.to_path_buf(),
    }
}

/// Every meeting folder under `roots` with a journal or with audio, other than
/// the one being recorded into.
fn find(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let folder = plain(&entry.path());
            if (folder.join(JOURNAL_FILE).is_file() || looks_like_recording(&folder))
                && !is_active(&folder)
                && !found.contains(&folder)
            {
                found.push(folder);
            }
        }
    }
    found
}

/// The folders the library already has a meeting for, in the form [`find`]
/// yields them: canonical, without the verbatim prefix. A meeting's folder is
/// stored the way the recorder wrote it, so the text alone may differ in case
/// or in the prefix and still name the same folder.
pub(super) async fn saved_folders(
    pool: &sqlx::SqlitePool,
) -> Result<std::collections::HashSet<PathBuf>, String> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT folder_path FROM meetings WHERE folder_path IS NOT NULL")
            .fetch_all(pool)
            .await
            .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|(folder,)| {
            let folder = PathBuf::from(folder);
            plain(&folder.canonicalize().unwrap_or(folder))
        })
        .collect())
}

/// Newest modification time among the files of `folder`, milliseconds since
/// the epoch: when the recording last wrote anything.
fn last_written(folder: &Path) -> i64 {
    std::fs::read_dir(folder)
        .ok()
        .and_then(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok()?.modified().ok())
                .max()
        })
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// Recordings that never reached the database: those whose transcript the
/// journal kept, and those with only audio on disk (see
/// [`super::orphan_recordings`]).
///
/// A journal whose folder already belongs to a saved meeting is a leftover
/// from a delete that failed; it is removed here instead of being offered. A
/// folder the owner chose not to restore is not offered again.
#[tauri::command]
pub async fn list_unsaved_recordings<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    state: tauri::State<'_, crate::state::AppState>,
) -> Result<Vec<Unsaved>, String> {
    let roots = super::recording_preferences::recording_roots(&app).await;
    let pool = state.db_manager.pool();
    let saved = saved_folders(pool).await?;
    let archive_open = !crate::security::session::archive_is_protected()
        || crate::security::session::archive_is_open();
    // A recording under way has a folder with audio and no meeting yet; it is
    // not something to restore.
    let recording = super::recording_commands::is_recording_active();
    let mut unsaved = Vec::new();
    for folder in find(&roots) {
        if saved.contains(&folder) {
            remove(&folder);
            continue;
        }
        if is_dismissed(&folder) {
            continue;
        }
        let journal = match read(&folder) {
            Ok(journal) => journal,
            // The archive is locked: no journal opens, and neither would audio.
            Err(error) if !archive_open => return Err(error.to_string()),
            Err(error) => {
                warn!("Could not read a transcript journal: {error}");
                None
            }
        };
        match journal {
            Some(journal) if !journal.segments.is_empty() => unsaved.push(journal),
            journal if !recording => {
                let Some(audio) = probe(&folder, archive_open) else {
                    continue;
                };
                let (title, started_at) = journal
                    .map(|journal| (journal.title, journal.started_at))
                    .unwrap_or_default();
                unsaved.push(Unsaved {
                    folder_path: folder.to_string_lossy().into_owned(),
                    title,
                    started_at,
                    last_updated: last_written(&folder),
                    segments: Vec::new(),
                    audio_only: Some(audio),
                });
            }
            _ => {}
        }
    }
    unsaved.sort_by_key(|journal| std::cmp::Reverse(journal.last_updated));
    Ok(unsaved)
}

/// Only folders inside the recording roots, so the window cannot point these
/// commands at an arbitrary path.
pub(super) async fn checked_folder<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    folder_path: &str,
) -> Result<PathBuf, String> {
    let folder = PathBuf::from(folder_path)
        .canonicalize()
        .map_err(|error| format!("No such recording folder: {error}"))?;
    let roots = super::recording_preferences::recording_roots(app).await;
    if super::recording_preferences::is_meeting_folder(&folder, &roots) {
        Ok(folder)
    } else {
        Err("That folder is not a recording folder".to_string())
    }
}

/// The owner chose not to restore a recording: its journal is discarded and
/// the folder is not offered again. The audio stays where it is.
#[tauri::command]
pub async fn discard_unsaved_transcript<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    folder_path: String,
) -> Result<(), String> {
    let folder = checked_folder(&app, &folder_path).await?;
    remove(&folder);
    dismiss(&folder);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(sequence_id: u64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            id: format!("seg_{sequence_id}"),
            text: text.to_string(),
            audio_start_time: sequence_id as f64,
            audio_end_time: sequence_id as f64 + 1.0,
            duration: 1.0,
            display_time: "[00:00]".to_string(),
            confidence: 1.0,
            sequence_id,
            speaker: Some("You".to_string()),
        }
    }

    // One test, because the running journal is process-wide.
    #[test]
    fn a_journal_reads_back_with_updates_applied_and_torn_lines_skipped() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("meeting");
        std::fs::create_dir(&folder).unwrap();
        begin(&folder, "Встреча", "2026-09-18T10:00:00Z");
        assert_eq!(active_folder().as_deref(), Some(folder.as_path()));
        // A recording still running is not offered for recovery.
        assert!(find(&[root.path().to_path_buf()]).is_empty());
        append(&segment(2, "второе"));
        append(&segment(1, "первое"));
        append(&segment(2, "второе, исправленное"));
        end();
        assert!(active_folder().is_none());

        // A crash mid-write leaves half a line.
        let mut file = OpenOptions::new()
            .append(true)
            .open(folder.join(JOURNAL_FILE))
            .unwrap();
        file.write_all(b"{\"kind\":\"segm").unwrap();
        drop(file);

        let journal = read(&folder).unwrap().unwrap();
        assert_eq!(journal.title, "Встреча");
        let texts: Vec<_> = journal.segments.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["первое", "второе, исправленное"]);

        assert_eq!(find(&[root.path().to_path_buf()]), vec![folder.clone()]);
        remove(&folder);
        assert!(read(&folder).unwrap().is_none());
        assert!(find(&[root.path().to_path_buf()]).is_empty());
    }

    #[test]
    fn a_folder_with_audio_and_no_journal_is_found() {
        let root = tempfile::tempdir().unwrap();
        let with_audio = root.path().join("rec-a");
        let empty = root.path().join("rec-b");
        std::fs::create_dir(&with_audio).unwrap();
        std::fs::create_dir(&empty).unwrap();
        std::fs::write(with_audio.join("system.mp4"), b"audio").unwrap();
        std::fs::write(empty.join("metadata.json"), b"{}").unwrap();
        assert_eq!(find(&[root.path().to_path_buf()]), vec![with_audio]);
    }

    #[test]
    fn a_verbatim_prefix_is_dropped_but_a_unc_path_is_kept() {
        assert_eq!(plain(Path::new(r"\\?\C:\rec\m1")), PathBuf::from(r"C:\rec\m1"));
        assert_eq!(
            plain(Path::new(r"\\?\UNC\srv\share")),
            PathBuf::from(r"\\?\UNC\srv\share")
        );
        assert_eq!(plain(Path::new(r"C:\rec\m1")), PathBuf::from(r"C:\rec\m1"));
    }
}
