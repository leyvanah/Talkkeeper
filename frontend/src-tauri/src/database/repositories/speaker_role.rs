//! Which side of the conversation each speaker of a meeting is on.
//!
//! Two sides: the host, who holds the conversation and records it, and the
//! client, for whom it is held. A table view of a recording is laid out by this
//! — one column per side — and so is anything later that needs to treat the
//! two differently.
//!
//! ## Nothing to configure for the recordings that exist
//!
//! Every recording so far is a call: the host speaks into the local
//! microphone, whose label is "You", and the client arrives through the
//! speakers under whatever label diarization or a rename gave them. So the
//! role follows from the label, and no row is needed for it.
//!
//! ## Room for the recordings that will not fit that
//!
//! A conversation recorded in one room puts both voices on one microphone, and
//! diarization, not the capture source, tells them apart; a rename can also
//! leave a label that says nothing about its side. For those, a role can be
//! set per speaker per meeting, and the setting wins over the label. Only
//! those exceptions are stored.
//!
//! A line spoken by both at once carries a joined label ("You + Speaker 1") and
//! belongs to both sides.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use sqlx::{Error as SqlxError, Sqlite, SqlitePool, Transaction};

use crate::database::fields;
use crate::state::AppState;

/// One side of the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Host,
    Client,
}

impl Role {
    fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Client => "client",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "host" => Some(Self::Host),
            "client" => Some(Self::Client),
            _ => None,
        }
    }
}

/// Where a line belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Host,
    Client,
    /// Both spoke at once.
    Both,
}

impl Side {
    fn of(roles: &BTreeSet<Role>) -> Self {
        match (roles.contains(&Role::Host), roles.contains(&Role::Client)) {
            (true, true) => Self::Both,
            (true, false) => Self::Host,
            _ => Self::Client,
        }
    }
}

/// One speaker label of a meeting, and where its lines go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerSide {
    pub speaker: String,
    pub side: Side,
    /// Whether a person chose this rather than the label implying it.
    pub assigned: bool,
}

/// Whether a label is the local microphone's.
///
/// The same test the transcript view makes: the bare marker, or a display
/// name the view decorated with it.
pub fn is_local_speaker(label: &str) -> bool {
    let lower = label.trim().to_lowercase();
    // `^you\b`
    let marker_first = lower.strip_prefix("you").is_some_and(|rest| {
        !rest.starts_with(|c: char| c.is_alphanumeric() || c == '_')
    });
    // `\(\s*you\s*\)$`
    let marker_last = lower
        .strip_suffix(')')
        .and_then(|head| head.rsplit_once('('))
        .is_some_and(|(_, inner)| inner.trim() == "you");
    marker_first || marker_last
}

/// The role a single, unjoined label has when nobody said otherwise.
pub fn implied_role(label: &str) -> Role {
    if is_local_speaker(label) {
        Role::Host
    } else {
        Role::Client
    }
}

/// The parts of a label: one for a single voice, several for overlapping ones.
fn voices(label: &str) -> impl Iterator<Item = &str> {
    label.split(" + ").map(str::trim).filter(|part| !part.is_empty())
}

/// Where the lines of `label` go, given the roles assigned in this meeting.
///
/// `assigned` answers for a whole label first — a joined label can be
/// assigned too — and then for each voice in it.
pub fn side_of(label: &str, assigned: impl Fn(&str) -> Option<Role>) -> Side {
    if let Some(role) = assigned(label.trim()) {
        return Side::of(&BTreeSet::from([role]));
    }
    let roles: BTreeSet<Role> = voices(label)
        .map(|voice| assigned(voice).unwrap_or_else(|| implied_role(voice)))
        .collect();
    Side::of(&roles)
}

pub struct SpeakerRolesRepository;

impl SpeakerRolesRepository {
    /// Every speaker of a meeting and its side.
    pub async fn sides(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<SpeakerSide>, SqlxError> {
        let stored_labels: Vec<Option<String>> = sqlx::query_scalar(
            "SELECT DISTINCT speaker FROM transcripts WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        let mut labels = BTreeSet::new();
        for stored in stored_labels.into_iter().flatten() {
            let label = fields::open(fields::TRANSCRIPT_SPEAKER, &stored)?;
            if !label.trim().is_empty() {
                labels.insert(label.trim().to_string());
            }
        }

        let assigned = Self::assigned(pool, meeting_id).await?;
        let lookup = |label: &str| {
            assigned
                .iter()
                .find(|(speaker, _)| speaker == label)
                .map(|(_, role)| *role)
        };

        Ok(labels
            .into_iter()
            .map(|speaker| SpeakerSide {
                side: side_of(&speaker, &lookup),
                assigned: lookup(&speaker).is_some(),
                speaker,
            })
            .collect())
    }

    /// The roles people chose in this meeting.
    async fn assigned(pool: &SqlitePool, meeting_id: &str) -> Result<Vec<(String, Role)>, SqlxError> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT speaker_label, role FROM meeting_speaker_roles WHERE meeting_id = ?",
        )
        .bind(meeting_id)
        .fetch_all(pool)
        .await?;
        let mut assigned = Vec::with_capacity(rows.len());
        for (stored, role) in rows {
            // The CHECK constraint makes an unknown role impossible to write;
            // one that got in anyway is ignored rather than guessed at.
            if let Some(role) = Role::parse(&role) {
                assigned.push((fields::open(fields::SPEAKER_LABEL, &stored)?, role));
            }
        }
        Ok(assigned)
    }

    /// Put a speaker on a side, or with `None` let the label decide again.
    pub async fn assign(
        pool: &SqlitePool,
        meeting_id: &str,
        speaker: &str,
        role: Option<Role>,
    ) -> Result<(), SqlxError> {
        let speaker = speaker.trim();
        let label = fields::seal_joinable(fields::SPEAKER_LABEL, speaker);
        match role {
            Some(role) => {
                sqlx::query(
                    "INSERT INTO meeting_speaker_roles (meeting_id, speaker_label, role) \
                     VALUES (?, ?, ?) \
                     ON CONFLICT(meeting_id, speaker_label) DO UPDATE SET role = excluded.role",
                )
                .bind(meeting_id)
                .bind(label)
                .bind(role.as_str())
                .execute(pool)
                .await?;
            }
            None => {
                sqlx::query(
                    "DELETE FROM meeting_speaker_roles WHERE meeting_id = ? AND speaker_label = ?",
                )
                .bind(meeting_id)
                .bind(label)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Carry a speaker's assigned role across a rename, inside the rename's
    /// own transaction.
    ///
    /// The lines of `from` are now lines of `to`, so a role chosen for `from`
    /// goes with them — unless `to` already had its own, which describes the
    /// lines that were there first. Renaming someone to the local marker is a
    /// statement of who they are, and the role that statement implies wins
    /// over any earlier choice.
    pub(crate) async fn follow_rename(
        tx: &mut Transaction<'_, Sqlite>,
        meeting_id: &str,
        from: &str,
        to: &str,
    ) -> Result<(), SqlxError> {
        let from_label = fields::seal_joinable(fields::SPEAKER_LABEL, from);
        let to_label = fields::seal_joinable(fields::SPEAKER_LABEL, to);
        if from_label == to_label {
            return Ok(());
        }

        let moved: Option<String> = sqlx::query_scalar(
            "SELECT role FROM meeting_speaker_roles WHERE meeting_id = ? AND speaker_label = ?",
        )
        .bind(meeting_id)
        .bind(&from_label)
        .fetch_optional(&mut **tx)
        .await?;
        sqlx::query("DELETE FROM meeting_speaker_roles WHERE meeting_id = ? AND speaker_label = ?")
            .bind(meeting_id)
            .bind(&from_label)
            .execute(&mut **tx)
            .await?;

        let Some(role) = moved else {
            return Ok(());
        };
        if is_local_speaker(to) {
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO meeting_speaker_roles (meeting_id, speaker_label, role) \
             VALUES (?, ?, ?) \
             ON CONFLICT(meeting_id, speaker_label) DO NOTHING",
        )
        .bind(meeting_id)
        .bind(&to_label)
        .bind(role)
        .execute(&mut **tx)
        .await?;
        Ok(())
    }
}

#[tauri::command]
pub async fn api_get_speaker_sides(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
) -> Result<Vec<SpeakerSide>, String> {
    SpeakerRolesRepository::sides(state.db_manager.pool(), &meeting_id)
        .await
        .map_err(|error| format!("Failed to read speaker roles: {}", error))
}

#[tauri::command]
pub async fn api_assign_speaker_role(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    speaker: String,
    role: Option<Role>,
) -> Result<(), String> {
    if speaker.trim().is_empty() {
        return Err("Speaker label is empty".to_string());
    }
    SpeakerRolesRepository::assign(state.db_manager.pool(), &meeting_id, &speaker, role)
        .await
        .map_err(|error| format!("Failed to assign speaker role: {}", error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE meetings (id TEXT PRIMARY KEY); \
             CREATE TABLE transcripts (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, \
                 speaker TEXT); \
             INSERT INTO meetings VALUES ('m1'), ('m2'); \
             INSERT INTO transcripts VALUES \
                 ('t1', 'm1', 'You'), ('t2', 'm1', 'Speaker 1'), \
                 ('t3', 'm1', 'You + Speaker 1'), ('t4', 'm1', NULL), \
                 ('t5', 'm2', 'Speaker 2');",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../../../migrations/20260916000000_add_speaker_roles.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    fn side(sides: &[SpeakerSide], speaker: &str) -> (Side, bool) {
        let found = sides
            .iter()
            .find(|entry| entry.speaker == speaker)
            .unwrap_or_else(|| panic!("{speaker} is not among {sides:?}"));
        (found.side, found.assigned)
    }

    #[test]
    fn the_microphone_holds_the_conversation() {
        for label in ["You", "you", " You ", "Anna (You)", "Anna (you)"] {
            assert_eq!(implied_role(label), Role::Host, "{label}");
        }
        for label in ["Speaker 1", "Guest", "Anna", "Young", "Yours truly"] {
            assert_eq!(implied_role(label), Role::Client, "{label}");
        }
    }

    #[test]
    fn a_line_spoken_by_both_belongs_to_both() {
        let nobody = |_: &str| None;
        assert_eq!(side_of("You + Speaker 1", nobody), Side::Both);
        assert_eq!(side_of("Speaker 1 + Speaker 2", nobody), Side::Client);
        assert_eq!(side_of("You", nobody), Side::Host);
    }

    /// One microphone in one room: diarization's Speaker 1 is the host.
    #[test]
    fn an_assigned_role_wins_over_the_label() {
        let speaker_one_hosts = |label: &str| (label == "Speaker 1").then_some(Role::Host);
        assert_eq!(side_of("Speaker 1", speaker_one_hosts), Side::Host);
        assert_eq!(side_of("Speaker 1 + Speaker 2", speaker_one_hosts), Side::Both);
        assert_eq!(side_of("Speaker 2", speaker_one_hosts), Side::Client);
    }

    #[tokio::test]
    async fn a_call_needs_no_setup() {
        let pool = test_pool().await;
        let sides = SpeakerRolesRepository::sides(&pool, "m1").await.unwrap();

        assert_eq!(sides.len(), 3, "{sides:?}");
        assert_eq!(side(&sides, "You"), (Side::Host, false));
        assert_eq!(side(&sides, "Speaker 1"), (Side::Client, false));
        assert_eq!(side(&sides, "You + Speaker 1"), (Side::Both, false));
    }

    #[tokio::test]
    async fn an_assignment_is_kept_per_meeting_and_can_be_taken_back() {
        let pool = test_pool().await;
        SpeakerRolesRepository::assign(&pool, "m1", "Speaker 1", Some(Role::Host))
            .await
            .unwrap();

        let sides = SpeakerRolesRepository::sides(&pool, "m1").await.unwrap();
        assert_eq!(side(&sides, "Speaker 1"), (Side::Host, true));
        assert_eq!(side(&sides, "You + Speaker 1"), (Side::Host, false));
        // Another meeting's Speaker 1 is another person.
        let other = SpeakerRolesRepository::sides(&pool, "m2").await.unwrap();
        assert_eq!(side(&other, "Speaker 2"), (Side::Client, false));

        SpeakerRolesRepository::assign(&pool, "m1", "Speaker 1", None)
            .await
            .unwrap();
        let sides = SpeakerRolesRepository::sides(&pool, "m1").await.unwrap();
        assert_eq!(side(&sides, "Speaker 1"), (Side::Client, false));
    }

    #[tokio::test]
    async fn a_role_follows_its_speaker_through_a_rename() {
        let pool = test_pool().await;
        SpeakerRolesRepository::assign(&pool, "m1", "Speaker 1", Some(Role::Host))
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        sqlx::query("UPDATE transcripts SET speaker = 'Anna' WHERE speaker = 'Speaker 1'")
            .execute(&mut *tx)
            .await
            .unwrap();
        SpeakerRolesRepository::follow_rename(&mut tx, "m1", "Speaker 1", "Anna")
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let sides = SpeakerRolesRepository::sides(&pool, "m1").await.unwrap();
        assert_eq!(side(&sides, "Anna"), (Side::Host, true));
        assert!(sides.iter().all(|entry| entry.speaker != "Speaker 1"));
        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_speaker_roles")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 1);
    }

    /// Merging into a speaker that already had a role keeps that role.
    #[tokio::test]
    async fn a_merge_keeps_the_role_of_the_speaker_merged_into() {
        let pool = test_pool().await;
        SpeakerRolesRepository::assign(&pool, "m1", "Speaker 1", Some(Role::Host))
            .await
            .unwrap();
        SpeakerRolesRepository::assign(&pool, "m1", "Speaker 2", Some(Role::Client))
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        SpeakerRolesRepository::follow_rename(&mut tx, "m1", "Speaker 1", "Speaker 2")
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT speaker_label, role FROM meeting_speaker_roles")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows, vec![("Speaker 2".to_string(), "client".to_string())]);
    }

    /// Calling someone "You" says who they are; an earlier choice does not
    /// outlive that.
    #[tokio::test]
    async fn renaming_to_the_local_marker_drops_the_assignment() {
        let pool = test_pool().await;
        SpeakerRolesRepository::assign(&pool, "m1", "Speaker 1", Some(Role::Client))
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        SpeakerRolesRepository::follow_rename(&mut tx, "m1", "Speaker 1", "You")
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meeting_speaker_roles")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0);
    }
}
