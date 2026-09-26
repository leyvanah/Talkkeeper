//! What the settings window can ask of the library export and import.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use super::import::{import_meeting, match_clients, Outcome};
use super::plain::PlainExport;
use super::sealed::{SealedExport, SealedPackage};
use super::{clients, collect_meeting, meeting_ids, CollectedMeeting, Manifest, MeetingEntry, FORMAT_VERSION};
use crate::security::session;
use crate::state::AppState;

/// What the window shows when an export is done.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    /// The folder the export was written to.
    pub folder: String,
    pub meetings: usize,
    pub audio_files: usize,
    pub audio_bytes: u64,
    /// Recordings that could not be written; the log says why.
    pub failed: usize,
}

/// What the window shows when an import is done.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub imported: usize,
    /// Already in this library, left as they were.
    pub already_present: usize,
    pub failed: usize,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    done: usize,
    total: usize,
}

/// Either kind of export, as the loop below drives it.
trait ExportTarget: Send + 'static {
    fn write_meeting(
        &mut self,
        meeting: &CollectedMeeting,
        filed_under: Option<&str>,
    ) -> io::Result<(MeetingEntry, u64)>;
    fn write_manifest(&self, manifest: &Manifest) -> io::Result<()>;
    fn root(&self) -> &Path;
}

impl ExportTarget for PlainExport {
    fn write_meeting(&mut self, meeting: &CollectedMeeting, filed_under: Option<&str>) -> io::Result<(MeetingEntry, u64)> {
        PlainExport::write_meeting(self, meeting, filed_under)
    }
    fn write_manifest(&self, manifest: &Manifest) -> io::Result<()> {
        PlainExport::write_manifest(self, manifest)
    }
    fn root(&self) -> &Path {
        PlainExport::root(self)
    }
}

impl ExportTarget for SealedExport {
    /// The package keeps the client in the sealed record, not in a folder name.
    fn write_meeting(&mut self, meeting: &CollectedMeeting, _: Option<&str>) -> io::Result<(MeetingEntry, u64)> {
        SealedExport::write_meeting(self, meeting)
    }
    fn write_manifest(&self, manifest: &Manifest) -> io::Result<()> {
        SealedExport::write_manifest(self, manifest)
    }
    fn root(&self) -> &Path {
        SealedExport::root(self)
    }
}

/// Nothing can be read out of, or written into, a locked archive.
fn refuse_when_locked() -> Result<(), String> {
    if session::archive_is_protected() && !session::archive_is_open() {
        return Err("The archive is locked — unlock it first".to_string());
    }
    Ok(())
}

fn existing_folder(dir: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(dir.trim());
    if !path.is_dir() {
        return Err("The chosen folder does not exist".to_string());
    }
    Ok(path)
}

async fn run_export<R: Runtime, T: ExportTarget>(
    app: &AppHandle<R>,
    state: &State<'_, AppState>,
    mut export: T,
    kind: &str,
) -> Result<ExportReport, String> {
    // An export of a large library runs for minutes with nobody touching the
    // window; the idle lock waits for it, as it waits for a summary.
    let _busy = session::busy();
    let pool = state.db_manager.pool().clone();
    let clients = clients(&pool).await?;
    let ids = meeting_ids(&pool).await?;
    let total = ids.len();
    log::info!("📦 Library export ({kind}) started: {total} recordings");

    let mut entries = Vec::with_capacity(total);
    let (mut audio_files, mut audio_bytes, mut failed) = (0usize, 0u64, 0usize);
    for (index, id) in ids.iter().enumerate() {
        let _ = app.emit("library-export-progress", Progress { done: index, total });
        let meeting = match collect_meeting(&pool, id).await {
            Ok(meeting) => meeting,
            Err(error) => {
                log::warn!("📦 Recording {id} not exported: {error}");
                failed += 1;
                continue;
            }
        };
        let filed_under = meeting
            .record
            .client_id
            .as_ref()
            .and_then(|client_id| clients.iter().find(|client| client.id == *client_id))
            .map(|client| client.name.clone());
        let tracks = meeting.audio.len();

        // Copying audio is file work; it does not belong on the async threads.
        let (returned, written) = tokio::task::spawn_blocking(move || {
            let written = export.write_meeting(&meeting, filed_under.as_deref());
            (export, written)
        })
        .await
        .map_err(|error| format!("The export task failed: {error}"))?;
        export = returned;

        match written {
            Ok((entry, bytes)) => {
                entries.push(entry);
                audio_files += tracks;
                audio_bytes += bytes;
            }
            Err(error) => {
                log::warn!("📦 Recording {id} not exported: {error}");
                failed += 1;
            }
        }
    }

    let manifest = Manifest {
        format: FORMAT_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        exported_at: chrono::Utc::now().to_rfc3339(),
        clients,
        meetings: entries,
    };
    export
        .write_manifest(&manifest)
        .map_err(|error| format!("Could not write the library list: {error}"))?;
    let _ = app.emit("library-export-progress", Progress { done: total, total });

    log::info!(
        "📦 Library export ({kind}) finished: {} recordings, {} audio files ({} MB), {} failed",
        manifest.meetings.len(),
        audio_files,
        audio_bytes / 1_048_576,
        failed
    );
    Ok(ExportReport {
        folder: export.root().to_string_lossy().to_string(),
        meetings: manifest.meetings.len(),
        audio_files,
        audio_bytes,
        failed,
    })
}

/// Writes the whole library as ordinary files under `target_dir`.
///
/// Everything comes out readable: audio decrypted, text as text. That is the
/// point of it and the window says so before it starts.
#[tauri::command]
pub async fn export_library_readable<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    target_dir: String,
    unassigned_label: String,
) -> Result<ExportReport, String> {
    refuse_when_locked()?;
    let target = existing_folder(&target_dir)?;
    let export = PlainExport::create(&target, &unassigned_label)
        .map_err(|error| format!("Could not create the export folder: {error}"))?;
    run_export(&app, &state, export, "readable").await
}

/// Writes the whole library as a transfer package sealed with `password`.
#[tauri::command]
pub async fn export_library_package<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    target_dir: String,
    password: String,
) -> Result<ExportReport, String> {
    refuse_when_locked()?;
    let target = existing_folder(&target_dir)?;
    // Argon2 is deliberately slow; not on the async threads either.
    let export = tokio::task::spawn_blocking(move || SealedExport::create(&target, &password))
        .await
        .map_err(|error| format!("The export task failed: {error}"))??;
    run_export(&app, &state, export, "package").await
}

/// Adds the recordings of a transfer package to this library.
#[tauri::command]
pub async fn import_library_package<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    package_dir: String,
    password: String,
) -> Result<ImportReport, String> {
    refuse_when_locked()?;
    let root = existing_folder(&package_dir)?;
    let _busy = session::busy();

    let (package, manifest) = tokio::task::spawn_blocking(move || SealedPackage::open(&root, &password))
        .await
        .map_err(|error| format!("The import task failed: {error}"))??;
    let package = Arc::new(package);

    let pool = state.db_manager.pool().clone();
    let clients = match_clients(&pool, &manifest.clients).await?;
    let recordings_root = crate::audio::recording_preferences::get_default_recordings_folder();
    let total = manifest.meetings.len();
    log::info!("📥 Library import started: {total} recordings in the package");

    let mut report = ImportReport { imported: 0, already_present: 0, failed: 0 };
    for (index, entry) in manifest.meetings.iter().enumerate() {
        let _ = app.emit("library-import-progress", Progress { done: index, total });
        match import_meeting(&pool, package.clone(), entry, &clients, &recordings_root).await {
            Ok(Outcome::Imported) => report.imported += 1,
            Ok(Outcome::AlreadyPresent) => report.already_present += 1,
            Err(error) => {
                log::warn!("📥 Recording {} not imported: {error}", entry.id);
                report.failed += 1;
            }
        }
    }
    let _ = app.emit("library-import-progress", Progress { done: total, total });
    // The sidebar lists what the database holds; tell it to read again.
    let _ = app.emit("library-changed", ());

    log::info!(
        "📥 Library import finished: {} added, {} already here, {} failed",
        report.imported,
        report.already_present,
        report.failed
    );
    Ok(report)
}
