//! Which recordings still wait for their processing after Stop.
//!
//! The window runs that processing one recording at a time, and its queue is
//! held in memory. This mark is what survives a restart: the window sets it
//! when a recording becomes a meeting, clears it when the processing settles,
//! and at the next start takes up whatever is still marked.

use crate::state::AppState;
use serde::Serialize;
use sqlx::{Error as SqlxError, SqlitePool};

/// A meeting whose processing has not settled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPostCall {
    pub meeting_id: String,
    pub folder_path: Option<String>,
}

pub struct PostCallRepository;

impl PostCallRepository {
    pub async fn set_pending(
        pool: &SqlitePool,
        meeting_id: &str,
        pending: bool,
    ) -> Result<(), SqlxError> {
        let result = sqlx::query("UPDATE meetings SET post_call_pending = ? WHERE id = ?")
            .bind(pending)
            .bind(meeting_id)
            .execute(pool)
            .await?;
        if result.rows_affected() == 0 {
            return Err(SqlxError::RowNotFound);
        }
        Ok(())
    }

    /// Oldest first, the order they were recorded in.
    pub async fn pending(pool: &SqlitePool) -> Result<Vec<PendingPostCall>, SqlxError> {
        let rows: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id, folder_path FROM meetings WHERE post_call_pending = 1 ORDER BY created_at",
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(meeting_id, folder_path)| PendingPostCall {
                meeting_id,
                folder_path,
            })
            .collect())
    }
}

#[tauri::command]
pub async fn api_set_post_call_pending(
    state: tauri::State<'_, AppState>,
    meeting_id: String,
    pending: bool,
) -> Result<(), String> {
    PostCallRepository::set_pending(state.db_manager.pool(), &meeting_id, pending)
        .await
        .map_err(|error| match error {
            SqlxError::RowNotFound => "Meeting not found".to_string(),
            _ => format!("Failed to mark the meeting's processing: {}", error),
        })
}

#[tauri::command]
pub async fn api_list_post_call_pending(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<PendingPostCall>, String> {
    PostCallRepository::pending(state.db_manager.pool())
        .await
        .map_err(|error| format!("Failed to list the meetings waiting for processing: {}", error))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn archive() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::database::manager::MIGRATOR.run(&pool).await.unwrap();
        for (id, created, folder) in [
            ("later", "2026-10-14T11:00:00Z", Some("rec-2")),
            ("earlier", "2026-10-14T10:00:00Z", Some("rec-1")),
            ("imported", "2026-10-14T09:00:00Z", None),
        ] {
            sqlx::query(
                "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) \
                 VALUES (?, 'm', ?, ?, ?)",
            )
            .bind(id)
            .bind(created)
            .bind(created)
            .bind(folder)
            .execute(&pool)
            .await
            .unwrap();
        }
        pool
    }

    #[tokio::test]
    async fn a_meeting_waits_only_while_marked() {
        let pool = archive().await;
        assert!(PostCallRepository::pending(&pool).await.unwrap().is_empty());

        PostCallRepository::set_pending(&pool, "later", true).await.unwrap();
        PostCallRepository::set_pending(&pool, "earlier", true).await.unwrap();
        let waiting = PostCallRepository::pending(&pool).await.unwrap();
        assert_eq!(
            waiting,
            vec![
                PendingPostCall {
                    meeting_id: "earlier".into(),
                    folder_path: Some("rec-1".into()),
                },
                PendingPostCall {
                    meeting_id: "later".into(),
                    folder_path: Some("rec-2".into()),
                },
            ],
            "oldest recording first"
        );

        PostCallRepository::set_pending(&pool, "earlier", false).await.unwrap();
        let waiting = PostCallRepository::pending(&pool).await.unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].meeting_id, "later");
    }

    #[tokio::test]
    async fn marking_a_meeting_that_is_gone_says_so() {
        let pool = archive().await;
        let error = PostCallRepository::set_pending(&pool, "deleted", true)
            .await
            .unwrap_err();
        assert!(matches!(error, SqlxError::RowNotFound));
    }
}
