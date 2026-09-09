//! Clients: the durable grouping a meeting belongs to.
//!
//! A meeting has at most one client, which is what makes the library a tree
//! rather than a tag cloud. The link is nullable on both ends of its life:
//! recordings start unassigned, and deleting a client detaches its recordings
//! instead of taking them along. Foreign keys are not enforced on every
//! connection this database has ever been opened with (see
//! `meeting.rs::delete_meeting_with_transaction`), so the detach is written out
//! explicitly rather than left to `ON DELETE SET NULL`.

use chrono::Utc;
use serde::Serialize;
use sqlx::{Error as SqlxError, SqlitePool};
use uuid::Uuid;

use crate::state::AppState;

/// A client row plus the two numbers the tree needs to render and sort itself.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientSummary {
    pub id: String,
    pub display_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub meeting_count: i64,
    /// Newest meeting start, so the list can lead with whoever was seen last.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_meeting_at: Option<String>,
}

pub struct ClientsRepository;

impl ClientsRepository {
    pub async fn list(pool: &SqlitePool) -> Result<Vec<ClientSummary>, SqlxError> {
        let rows = sqlx::query_as::<_, (String, String, Option<String>, i64, Option<String>)>(
            r#"
            SELECT
                c.id,
                c.display_name,
                c.notes,
                COUNT(m.id) AS meeting_count,
                MAX(m.created_at) AS last_meeting_at
            FROM clients c
            LEFT JOIN meetings m ON m.client_id = c.id
            GROUP BY c.id, c.display_name, c.notes
            ORDER BY last_meeting_at IS NULL, last_meeting_at DESC, c.normalized_name
            "#,
        )
        .fetch_all(pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(id, display_name, notes, meeting_count, last_meeting_at)| ClientSummary {
                    id,
                    display_name,
                    notes,
                    meeting_count,
                    last_meeting_at,
                },
            )
            .collect())
    }

    pub async fn create(pool: &SqlitePool, display_name: &str) -> Result<ClientSummary, SqlxError> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err(SqlxError::Protocol(
                "Client name cannot be empty".to_string(),
            ));
        }

        let id = format!("client-{}", Uuid::new_v4());
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO clients (id, display_name, normalized_name, notes, created_at, updated_at)
             VALUES (?, ?, ?, NULL, ?, ?)",
        )
        .bind(&id)
        .bind(display_name)
        .bind(normalize_display_name(display_name))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await?;

        Ok(ClientSummary {
            id,
            display_name: display_name.to_string(),
            notes: None,
            meeting_count: 0,
            last_meeting_at: None,
        })
    }

    pub async fn rename(
        pool: &SqlitePool,
        client_id: &str,
        display_name: &str,
    ) -> Result<(), SqlxError> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err(SqlxError::Protocol(
                "Client name cannot be empty".to_string(),
            ));
        }

        let result = sqlx::query(
            "UPDATE clients SET display_name = ?, normalized_name = ?, updated_at = ? WHERE id = ?",
        )
        .bind(display_name)
        .bind(normalize_display_name(display_name))
        .bind(Utc::now().to_rfc3339())
        .bind(client_id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Ok(())
    }

    pub async fn update_notes(
        pool: &SqlitePool,
        client_id: &str,
        notes: Option<&str>,
    ) -> Result<(), SqlxError> {
        let notes = notes.map(str::trim).filter(|value| !value.is_empty());
        let result = sqlx::query("UPDATE clients SET notes = ?, updated_at = ? WHERE id = ?")
            .bind(notes)
            .bind(Utc::now().to_rfc3339())
            .bind(client_id)
            .execute(pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Ok(())
    }

    /// Removes the client and returns how many meetings were detached. The
    /// meetings themselves survive: they hold the only copy of the recording.
    pub async fn delete(pool: &SqlitePool, client_id: &str) -> Result<u64, SqlxError> {
        let mut transaction = pool.begin().await?;

        let detached = sqlx::query("UPDATE meetings SET client_id = NULL WHERE client_id = ?")
            .bind(client_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();

        let removed = sqlx::query("DELETE FROM clients WHERE id = ?")
            .bind(client_id)
            .execute(&mut *transaction)
            .await?
            .rows_affected();

        if removed == 0 {
            transaction.rollback().await?;
            return Err(SqlxError::RowNotFound);
        }

        transaction.commit().await?;
        Ok(detached)
    }

    /// Moves a meeting under a client, or out from under all of them.
    ///
    /// An unknown client id is rejected rather than stored: a dangling id looks
    /// exactly like "unassigned" in every query, so the meeting would silently
    /// vanish from both the client's folder and the unassigned group.
    pub async fn assign_meeting(
        pool: &SqlitePool,
        meeting_id: &str,
        client_id: Option<&str>,
    ) -> Result<(), SqlxError> {
        if let Some(client_id) = client_id {
            let exists: Option<(i64,)> = sqlx::query_as("SELECT 1 FROM clients WHERE id = ?")
                .bind(client_id)
                .fetch_optional(pool)
                .await?;
            if exists.is_none() {
                return Err(SqlxError::RowNotFound);
            }
        }

        let result = sqlx::query("UPDATE meetings SET client_id = ? WHERE id = ?")
            .bind(client_id)
            .bind(meeting_id)
            .execute(pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Ok(())
    }
}

/// Lower-cased, with runs of whitespace collapsed, so "Anna  B" and "anna b"
/// are recognised as the same spelling when warning about duplicates.
pub(crate) fn normalize_display_name(name: &str) -> String {
    name.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

#[tauri::command]
pub async fn api_list_clients(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<ClientSummary>, String> {
    ClientsRepository::list(state.db_manager.pool())
        .await
        .map_err(|error| format!("Failed to list clients: {}", error))
}

#[tauri::command]
pub async fn api_create_client(
    state: tauri::State<'_, AppState>,
    display_name: String,
) -> Result<ClientSummary, String> {
    ClientsRepository::create(state.db_manager.pool(), &display_name)
        .await
        .map_err(|error| format!("Failed to create client: {}", error))
}

#[tauri::command]
pub async fn api_rename_client(
    state: tauri::State<'_, AppState>,
    client_id: String,
    display_name: String,
) -> Result<(), String> {
    ClientsRepository::rename(state.db_manager.pool(), &client_id, &display_name)
        .await
        .map_err(|error| match error {
            SqlxError::RowNotFound => "Client not found".to_string(),
            _ => format!("Failed to rename client: {}", error),
        })
}

#[tauri::command]
pub async fn api_update_client_notes(
    state: tauri::State<'_, AppState>,
    client_id: String,
    notes: Option<String>,
) -> Result<(), String> {
    ClientsRepository::update_notes(state.db_manager.pool(), &client_id, notes.as_deref())
        .await
        .map_err(|error| match error {
            SqlxError::RowNotFound => "Client not found".to_string(),
            _ => format!("Failed to update client notes: {}", error),
        })
}

#[tauri::command]
pub async fn api_delete_client(
    state: tauri::State<'_, AppState>,
    client_id: String,
) -> Result<u64, String> {
    ClientsRepository::delete(state.db_manager.pool(), &client_id)
        .await
        .map_err(|error| match error {
            SqlxError::RowNotFound => "Client not found".to_string(),
            _ => format!("Failed to delete client: {}", error),
        })
}

#[tauri::command]
pub async fn api_set_meeting_client(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    client_id: Option<String>,
) -> Result<(), String> {
    ClientsRepository::assign_meeting(
        state.db_manager.pool(),
        &meeting_id,
        client_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty()),
    )
    .await
    .map_err(|error| match error {
        SqlxError::RowNotFound => "Meeting or client not found".to_string(),
        _ => format!("Failed to change the meeting client: {}", error),
    })
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
        sqlx::query(
            "CREATE TABLE meetings (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                client_id TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE clients (
                id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
                normalized_name TEXT NOT NULL CHECK (length(normalized_name) > 0),
                notes TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn insert_meeting(pool: &SqlitePool, id: &str, created_at: &str) {
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(id)
        .bind(created_at)
        .bind(created_at)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn client_of(pool: &SqlitePool, meeting_id: &str) -> Option<String> {
        sqlx::query_scalar("SELECT client_id FROM meetings WHERE id = ?")
            .bind(meeting_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_new_client_starts_with_no_meetings() {
        let pool = test_pool().await;
        let created = ClientsRepository::create(&pool, "  Anna  ").await.unwrap();

        assert_eq!(created.display_name, "Anna");
        let listed = ClientsRepository::list(&pool).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].meeting_count, 0);
        assert!(listed[0].last_meeting_at.is_none());
    }

    #[tokio::test]
    async fn a_nameless_client_is_refused() {
        let pool = test_pool().await;
        assert!(ClientsRepository::create(&pool, "   ").await.is_err());

        let client = ClientsRepository::create(&pool, "Anna").await.unwrap();
        assert!(ClientsRepository::rename(&pool, &client.id, "\t")
            .await
            .is_err());
        let listed = ClientsRepository::list(&pool).await.unwrap();
        assert_eq!(listed[0].display_name, "Anna");
    }

    #[tokio::test]
    async fn two_people_may_share_a_name() {
        let pool = test_pool().await;
        let first = ClientsRepository::create(&pool, "Anna").await.unwrap();
        let second = ClientsRepository::create(&pool, "anna").await.unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(ClientsRepository::list(&pool).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn the_list_leads_with_whoever_was_seen_last() {
        let pool = test_pool().await;
        let older = ClientsRepository::create(&pool, "Older").await.unwrap();
        let newer = ClientsRepository::create(&pool, "Newer").await.unwrap();
        let never = ClientsRepository::create(&pool, "Never").await.unwrap();
        insert_meeting(&pool, "meeting-1", "2026-01-01T10:00:00Z").await;
        insert_meeting(&pool, "meeting-2", "2026-06-01T10:00:00Z").await;
        ClientsRepository::assign_meeting(&pool, "meeting-1", Some(&older.id))
            .await
            .unwrap();
        ClientsRepository::assign_meeting(&pool, "meeting-2", Some(&newer.id))
            .await
            .unwrap();

        let listed = ClientsRepository::list(&pool).await.unwrap();
        let order: Vec<&str> = listed.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(order, vec![newer.id.as_str(), older.id.as_str(), never.id.as_str()]);
        assert_eq!(listed[0].meeting_count, 1);
        assert_eq!(
            listed[0].last_meeting_at.as_deref(),
            Some("2026-06-01T10:00:00Z")
        );
    }

    #[tokio::test]
    async fn a_meeting_can_be_moved_and_unassigned() {
        let pool = test_pool().await;
        let client = ClientsRepository::create(&pool, "Anna").await.unwrap();
        insert_meeting(&pool, "meeting-1", "2026-01-01T10:00:00Z").await;

        ClientsRepository::assign_meeting(&pool, "meeting-1", Some(&client.id))
            .await
            .unwrap();
        assert_eq!(client_of(&pool, "meeting-1").await, Some(client.id.clone()));

        ClientsRepository::assign_meeting(&pool, "meeting-1", None)
            .await
            .unwrap();
        assert_eq!(client_of(&pool, "meeting-1").await, None);
    }

    #[tokio::test]
    async fn a_meeting_is_never_filed_under_a_client_that_does_not_exist() {
        let pool = test_pool().await;
        insert_meeting(&pool, "meeting-1", "2026-01-01T10:00:00Z").await;

        assert!(
            ClientsRepository::assign_meeting(&pool, "meeting-1", Some("client-ghost"))
                .await
                .is_err()
        );
        assert_eq!(client_of(&pool, "meeting-1").await, None);
    }

    #[tokio::test]
    async fn deleting_a_client_keeps_the_recordings_and_lets_them_go() {
        let pool = test_pool().await;
        let client = ClientsRepository::create(&pool, "Anna").await.unwrap();
        insert_meeting(&pool, "meeting-1", "2026-01-01T10:00:00Z").await;
        insert_meeting(&pool, "meeting-2", "2026-02-01T10:00:00Z").await;
        ClientsRepository::assign_meeting(&pool, "meeting-1", Some(&client.id))
            .await
            .unwrap();
        ClientsRepository::assign_meeting(&pool, "meeting-2", Some(&client.id))
            .await
            .unwrap();

        let detached = ClientsRepository::delete(&pool, &client.id).await.unwrap();

        assert_eq!(detached, 2);
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM meetings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(remaining, 2);
        assert_eq!(client_of(&pool, "meeting-1").await, None);
        assert!(ClientsRepository::list(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn notes_are_kept_and_blank_notes_are_erased() {
        let pool = test_pool().await;
        let client = ClientsRepository::create(&pool, "Anna").await.unwrap();

        ClientsRepository::update_notes(&pool, &client.id, Some("  prefers mornings  "))
            .await
            .unwrap();
        assert_eq!(
            ClientsRepository::list(&pool).await.unwrap()[0].notes.as_deref(),
            Some("prefers mornings")
        );

        ClientsRepository::update_notes(&pool, &client.id, Some("   "))
            .await
            .unwrap();
        assert!(ClientsRepository::list(&pool).await.unwrap()[0]
            .notes
            .is_none());
    }

    #[tokio::test]
    async fn an_unknown_client_is_reported_rather_than_ignored() {
        let pool = test_pool().await;
        assert!(matches!(
            ClientsRepository::rename(&pool, "client-ghost", "Anna").await,
            Err(SqlxError::RowNotFound)
        ));
        assert!(matches!(
            ClientsRepository::update_notes(&pool, "client-ghost", None).await,
            Err(SqlxError::RowNotFound)
        ));
        assert!(matches!(
            ClientsRepository::delete(&pool, "client-ghost").await,
            Err(SqlxError::RowNotFound)
        ));
    }

    #[test]
    fn spacing_and_case_do_not_make_a_different_name() {
        assert_eq!(normalize_display_name("  Anna   B  "), "anna b");
        assert_eq!(normalize_display_name("ANNA B"), "anna b");
    }
}
