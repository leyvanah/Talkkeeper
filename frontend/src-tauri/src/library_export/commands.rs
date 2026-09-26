//! What the settings window can ask of the library export.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};

use super::plain::PlainExport;
use super::{clients, collect_meeting, meeting_ids, Manifest, FORMAT_VERSION};
use crate::security::session;
use crate::state::AppState;

/// What the window shows when the export is done.
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    done: usize,
    total: usize,
}

/// Writes the whole library as ordinary files under `target_dir`.
///
/// Everything comes out readable: audio decrypted, text as text. That is the
/// point of it and the window says so before it starts; this only refuses
/// when the archive is locked, because then there is nothing to read.
#[tauri::command]
pub async fn export_library_readable<R: Runtime>(
    app: AppHandle<R>,
    state: State<'_, AppState>,
    target_dir: String,
    unassigned_label: String,
) -> Result<ExportReport, String> {
    if session::archive_is_protected() && !session::archive_is_open() {
        return Err("The archive is locked — unlock it to export the library".to_string());
    }
    let target = PathBuf::from(target_dir.trim());
    if !target.is_dir() {
        return Err("The chosen folder does not exist".to_string());
    }
    // An export of a large library runs for minutes with nobody touching the
    // window; the idle lock waits for it, as it waits for a summary.
    let _busy = session::busy();

    let pool = state.db_manager.pool().clone();
    let clients = clients(&pool).await?;
    let ids = meeting_ids(&pool).await?;
    let total = ids.len();

    let mut export = PlainExport::create(&target, &unassigned_label)
        .map_err(|error| format!("Could not create the export folder: {error}"))?;
    log::info!("📦 Readable library export started: {total} recordings");

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
        let client = meeting
            .record
            .client_id
            .as_ref()
            .and_then(|client_id| clients.iter().find(|client| client.id == *client_id))
            .map(|client| client.name.clone());
        let tracks = meeting.audio.len();

        // Copying audio is file work; it does not belong on the async threads.
        let (returned, written) = tokio::task::spawn_blocking(move || {
            let written = export.write_meeting(&meeting, client.as_deref());
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
        "📦 Readable library export finished: {} recordings, {} audio files ({} MB), {} failed",
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
