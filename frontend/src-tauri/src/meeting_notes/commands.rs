//! What the window can ask of the notes: those of the recording now running,
//! and those of a saved meeting.
//!
//! The window edits the notes as lines of one text, so both kinds answer in
//! the same shape: an added or edited line comes back as the note it became.

use tauri::{Emitter, Runtime, State, WebviewWindow};

use super::{insert_after, load_in, store, Note};
use crate::audio::transcript_journal;
use crate::state::AppState;

/// The notes of the running recording can be written in the main window and in
/// the compact bar. Each change is announced with the label of the window that
/// made it, so the other one reads them again.
fn announce<R: Runtime, T>(
    window: &WebviewWindow<R>,
    result: Result<T, String>,
) -> Result<T, String> {
    if result.is_ok() {
        let _ = window.emit("recording-notes-changed", window.label());
    }
    result
}

/// Adds a line to the notes of the recording now running, right after the
/// line `after` (first when none).
///
/// `started_at` is where the recording was when the owner began typing the
/// line: a note is about what was just said, and typing it takes a while.
/// Without it — or with a moment the recording has not reached — the line is
/// placed where the recording is now.
#[tauri::command]
pub async fn recording_note_add<R: Runtime>(
    window: WebviewWindow<R>,
    text: String,
    started_at: Option<f64>,
    after: Option<String>,
) -> Result<Note, String> {
    let now = crate::audio::recording_commands::recorded_seconds();
    let note = Note::new(&text, moment(started_at, now))?;
    announce(
        &window,
        transcript_journal::add_note(note, after.as_deref()),
    )
}

fn moment(started_at: Option<f64>, now: Option<f64>) -> Option<f64> {
    match (started_at.filter(|at| at.is_finite() && *at >= 0.0), now) {
        (Some(at), Some(now)) => Some(at.min(now)),
        (_, now) => now,
    }
}

#[tauri::command]
pub async fn recording_note_edit<R: Runtime>(
    window: WebviewWindow<R>,
    id: String,
    text: String,
) -> Result<Note, String> {
    announce(&window, transcript_journal::edit_note(&id, &text))
}

#[tauri::command]
pub async fn recording_note_remove<R: Runtime>(
    window: WebviewWindow<R>,
    id: String,
) -> Result<(), String> {
    announce(&window, transcript_journal::remove_note(&id))
}

/// The notes of the recording now running, for a window that opens mid-way.
#[tauri::command]
pub async fn recording_notes() -> Vec<Note> {
    transcript_journal::current_notes()
}

fn failed(error: sqlx::Error) -> String {
    if crate::database::fields::is_archive_locked(&error) {
        "The archive is locked — unlock it first".to_string()
    } else {
        format!("Could not save the notes: {error}")
    }
}

#[tauri::command]
pub async fn api_get_meeting_notes(
    state: State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<Note>, String> {
    super::load(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| format!("Could not read the notes: {error}"))
}

/// Reads a meeting's notes, changes them and writes them back in one
/// transaction, so two quick edits cannot overwrite each other.
async fn change<T>(
    state: &State<'_, AppState>,
    meeting_id: &str,
    apply: impl FnOnce(&mut Vec<Note>) -> Result<T, String>,
) -> Result<T, String> {
    let pool = state.db_manager.pool();
    let exists: Option<i64> = sqlx::query_scalar("SELECT 1 FROM meetings WHERE id = ?")
        .bind(meeting_id)
        .fetch_optional(pool)
        .await
        .map_err(failed)?;
    if exists.is_none() {
        return Err("No such meeting".to_string());
    }
    let mut tx = pool.begin().await.map_err(failed)?;
    let mut notes = load_in(&mut tx, meeting_id)
        .await
        .map_err(|error| format!("Could not read the notes: {error}"))?;
    let answer = apply(&mut notes)?;
    store(&mut tx, meeting_id, &notes).await.map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    Ok(answer)
}

/// Adds a line to a saved meeting's notes, right after the line `after`
/// (first when none). It has no moment in the recording: it was written after.
#[tauri::command]
pub async fn api_add_meeting_note(
    state: State<'_, AppState>,
    meeting_id: String,
    text: String,
    after: Option<String>,
) -> Result<Note, String> {
    let note = Note::new(&text, None)?;
    change(&state, &meeting_id, |notes| {
        insert_after(notes, note.clone(), after.as_deref());
        Ok(note)
    })
    .await
}

#[tauri::command]
pub async fn api_edit_meeting_note(
    state: State<'_, AppState>,
    meeting_id: String,
    id: String,
    text: String,
) -> Result<Note, String> {
    change(&state, &meeting_id, |notes| {
        let note = notes.iter_mut().find(|note| note.id == id).ok_or("No such note")?;
        *note = note.edited(&text)?;
        Ok(note.clone())
    })
    .await
}

#[tauri::command]
pub async fn api_remove_meeting_note(
    state: State<'_, AppState>,
    meeting_id: String,
    id: String,
) -> Result<(), String> {
    change(&state, &meeting_id, |notes| {
        notes.retain(|note| note.id != id);
        Ok(())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::moment;

    #[test]
    fn a_note_belongs_to_when_typing_began_but_never_to_the_future() {
        assert_eq!(moment(Some(40.0), Some(55.0)), Some(40.0));
        assert_eq!(moment(Some(90.0), Some(55.0)), Some(55.0));
        assert_eq!(moment(Some(-1.0), Some(55.0)), Some(55.0));
        assert_eq!(moment(Some(f64::NAN), Some(55.0)), Some(55.0));
        assert_eq!(moment(None, Some(55.0)), Some(55.0));
        assert_eq!(moment(Some(40.0), None), None);
    }
}
