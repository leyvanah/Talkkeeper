//! Which meetings have work running on them in the background.
//!
//! Retranscription, diarization and summaries run after their command has
//! answered, holding the meeting's files open and writing its rows. Deleting
//! the meeting under them went wrong on disk: Windows refuses to remove files
//! that are open, so the folder was half deleted, and a job writing audio
//! created the folder again — a piece of a deleted meeting that the recovery
//! window then offered back.
//!
//! So each such job marks its meeting while it runs, and a meeting with a mark
//! is not deleted (see `api_delete_meeting`). `security::session::busy` says
//! only that *some* work runs; this says on which meeting.

use std::sync::Mutex;

/// One entry per running job; a meeting with two jobs is listed twice.
static RUNNING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What a refused delete answers. The window recognises it by this text.
pub const MEETING_BUSY: &str = "The meeting is being processed; wait for it to finish or cancel it";

fn running() -> std::sync::MutexGuard<'static, Vec<String>> {
    RUNNING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Marks `meeting_id` as having work running until the guard is dropped.
#[must_use = "the meeting is marked only while the guard lives"]
pub fn start(meeting_id: &str) -> MeetingWork {
    running().push(meeting_id.to_string());
    MeetingWork {
        meeting_id: meeting_id.to_string(),
    }
}

/// Whether any job is running on `meeting_id`.
pub fn is_running(meeting_id: &str) -> bool {
    running().iter().any(|id| id == meeting_id)
}

/// Refuses with [`MEETING_BUSY`] while work runs on `meeting_id`.
pub fn refuse_while_running(meeting_id: &str) -> Result<(), String> {
    if is_running(meeting_id) {
        return Err(MEETING_BUSY.to_string());
    }
    Ok(())
}

/// The mark of one running job; dropping it removes that one entry.
pub struct MeetingWork {
    meeting_id: String,
}

impl Drop for MeetingWork {
    fn drop(&mut self) {
        let mut running = running();
        if let Some(index) = running.iter().position(|id| *id == self.meeting_id) {
            running.swap_remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meeting_is_busy_while_any_of_its_jobs_runs() {
        let id = "meeting-work-test-a";
        assert!(!is_running(id));
        assert!(refuse_while_running(id).is_ok());

        let first = start(id);
        let second = start(id);
        assert!(is_running(id));
        assert!(!is_running("meeting-work-test-other"));
        assert_eq!(refuse_while_running(id).unwrap_err(), MEETING_BUSY);

        drop(first);
        assert!(is_running(id), "the second job still runs");
        drop(second);
        assert!(!is_running(id));
    }
}
