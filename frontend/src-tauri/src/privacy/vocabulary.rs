//! Who this archive knows about, gathered for one outgoing request.
//!
//! The names come from the archive itself — the people in the library, the
//! speakers of this recording — plus whatever the owner added by hand. None of
//! it leaves: it is what decides *what must not leave*.

use sqlx::SqlitePool;

use super::anonymize::{Session, Vocabulary};
use super::settings;
use crate::database::fields;

/// Labels the app gives a voice when nobody has named it. They are not names,
/// and replacing them would turn a readable transcript into nonsense.
fn is_placeholder(label: &str) -> bool {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return true;
    }
    // "You + Speaker 1" is two placeholders; if every part is one, so is it.
    trimmed.split('+').all(|part| {
        let part = part.trim().to_lowercase();
        part == "you"
            || part == "вы"
            || part == "гость"
            || part == "guest"
            || part
                .strip_prefix("speaker")
                .or_else(|| part.strip_prefix("спикер"))
                .is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
    })
}

/// Names of the people the library holds.
async fn library_names(pool: &SqlitePool) -> Vec<String> {
    let sealed: Vec<String> = sqlx::query_scalar("SELECT display_name FROM clients")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    sealed
        .iter()
        .filter_map(|value| fields::open(fields::CLIENT_DISPLAY_NAME, value).ok())
        .collect()
}

/// Names of the people a recording can be filed under.
async fn person_names(pool: &SqlitePool) -> Vec<String> {
    let sealed: Vec<String> = sqlx::query_scalar("SELECT display_name FROM people")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    sealed
        .iter()
        .filter_map(|value| fields::open(fields::PERSON_NAME, value).ok())
        .collect()
}

/// Whatever the speakers of this recording were named.
async fn speaker_names(pool: &SqlitePool, meeting_id: &str) -> Vec<String> {
    let sealed: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT speaker FROM transcripts WHERE meeting_id = ?")
            .bind(meeting_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    sealed
        .iter()
        .filter_map(|value| fields::open(fields::TRANSCRIPT_SPEAKER, value).ok())
        .filter(|name| !is_placeholder(name))
        .collect()
}

/// Everything to hide for this recording, with duplicates and placeholders
/// removed.
pub async fn for_meeting(pool: &SqlitePool, meeting_id: Option<&str>) -> Vocabulary {
    let stored = settings::load(pool).await.unwrap_or_default();

    let mut names: Vec<String> = Vec::new();
    for name in library_names(pool)
        .await
        .into_iter()
        .chain(person_names(pool).await)
        .chain(match meeting_id {
            Some(meeting_id) => speaker_names(pool, meeting_id).await,
            None => Vec::new(),
        })
    {
        let trimmed = name.trim().to_string();
        if trimmed.is_empty() || is_placeholder(&trimmed) {
            continue;
        }
        if !names.iter().any(|kept| kept.eq_ignore_ascii_case(&trimmed)) {
            names.push(trimmed);
        }
    }

    Vocabulary {
        names,
        terms: stored.hidden_terms,
    }
}

/// The session to hide one recording's text with, or `None` when the owner
/// asked for no hiding at all.
///
/// A session is returned even when there is nothing to hide: the patterns —
/// addresses, numbers, links — need no vocabulary.
pub async fn session_for(pool: &SqlitePool, meeting_id: Option<&str>) -> Option<Session> {
    let stored = settings::load(pool).await.unwrap_or_default();
    if !stored.anonymize_cloud {
        return None;
    }
    Some(Session::new(for_meeting(pool, meeting_id).await))
}

/// The same, wrapped for sharing across the calls of one request.
pub async fn shield_for(pool: &SqlitePool, meeting_id: Option<&str>) -> Option<std::sync::Mutex<Session>> {
    session_for(pool, meeting_id).await.map(std::sync::Mutex::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_labels_the_app_invents_are_not_treated_as_names() {
        for label in ["You", "вы", "Speaker 1", "Спикер 12", "You + Speaker 1", "  "] {
            assert!(is_placeholder(label), "{label}");
        }
    }

    #[test]
    fn a_named_speaker_is_a_name() {
        for label in ["Анна", "Speaker Anna", "Пётр + Анна", "You + Анна"] {
            assert!(!is_placeholder(label), "{label}");
        }
    }
}
