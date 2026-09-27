//! What the window can ask of the notes: those of the recording now running,
//! and those of a saved meeting.

use tauri::State;

use super::{load, store, Note};
use crate::audio::transcript_journal;
use crate::state::AppState;

/// Adds a note to the recording now running.
///
/// `started_at` is where the recording was when the owner began typing: a
/// note is about what was just said, and typing it takes a while. Without
/// it — or with a moment the recording has not reached — the note is placed
/// where the recording is now.
#[tauri::command]
pub async fn recording_note_add(text: String, started_at: Option<f64>) -> Result<Note, String> {
    let now = crate::audio::recording_commands::recorded_seconds();
    transcript_journal::add_note(Note::new(&text, moment(started_at, now))?)
}

fn moment(started_at: Option<f64>, now: Option<f64>) -> Option<f64> {
    match (started_at.filter(|at| at.is_finite() && *at >= 0.0), now) {
        (Some(at), Some(now)) => Some(at.min(now)),
        (_, now) => now,
    }
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

#[tauri::command]
pub async fn recording_note_edit(id: String, text: String) -> Result<Note, String> {
    transcript_journal::edit_note(&id, &text)
}

#[tauri::command]
pub async fn recording_note_remove(id: String) -> Result<(), String> {
    transcript_journal::remove_note(&id)
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
    load(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| format!("Could not read the notes: {error}"))
}

/// Reads a meeting's notes, changes them and writes them back in one
/// transaction, so two quick edits cannot overwrite each other.
async fn change(
    state: &State<'_, AppState>,
    meeting_id: &str,
    apply: impl FnOnce(&mut Vec<Note>) -> Result<(), String>,
) -> Result<Vec<Note>, String> {
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
    let mut notes = super::load_in(&mut tx, meeting_id)
        .await
        .map_err(|error| format!("Could not read the notes: {error}"))?;
    apply(&mut notes)?;
    store(&mut tx, meeting_id, &notes).await.map_err(failed)?;
    tx.commit().await.map_err(failed)?;
    Ok(notes)
}

/// Adds a note to a saved meeting. It has no moment in the recording: it was
/// written after.
#[tauri::command]
pub async fn api_add_meeting_note(
    state: State<'_, AppState>,
    meeting_id: String,
    text: String,
) -> Result<Vec<Note>, String> {
    let note = Note::new(&text, None)?;
    change(&state, &meeting_id, |notes| {
        notes.push(note);
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn api_edit_meeting_note(
    state: State<'_, AppState>,
    meeting_id: String,
    id: String,
    text: String,
) -> Result<Vec<Note>, String> {
    change(&state, &meeting_id, |notes| {
        let note = notes.iter_mut().find(|note| note.id == id).ok_or("No such note")?;
        *note = note.edited(&text)?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn api_remove_meeting_note(
    state: State<'_, AppState>,
    meeting_id: String,
    id: String,
) -> Result<Vec<Note>, String> {
    change(&state, &meeting_id, |notes| {
        notes.retain(|note| note.id != id);
        Ok(())
    })
    .await
}
