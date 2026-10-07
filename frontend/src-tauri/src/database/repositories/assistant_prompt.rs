//! The owner's own wording for assistant requests.
//!
//! The default wording of a request — what a person's overview should cover,
//! say — is part of the interface and lives in the code, which is public. The
//! owner may want to ask something else, in words that say something about
//! the people they ask it about; that text lives only here, in the archive,
//! sealed with everything else they wrote.

use chrono::Utc;
use sqlx::SqlitePool;

use crate::database::fields;
use crate::state::AppState;

/// The request behind a person's overview.
pub const PERSON_OVERVIEW: &str = "person_overview";

/// Long enough for a careful paragraph, short enough to leave the model room
/// for the records it is asked about.
pub const MAX_PROMPT_CHARS: usize = 2_000;

pub struct AssistantPromptsRepository;

impl AssistantPromptsRepository {
    /// The owner's text for `id`, or `None` when the default is in use.
    pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<String>, sqlx::Error> {
        let stored: Option<String> =
            sqlx::query_scalar("SELECT prompt FROM assistant_prompts WHERE id = ?")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        fields::open_opt(fields::ASSISTANT_PROMPT, stored)
    }

    /// Stores the owner's text for `id`; blank goes back to the default.
    pub async fn set(pool: &SqlitePool, id: &str, prompt: Option<&str>) -> Result<(), String> {
        let prompt = prompt.map(str::trim).filter(|text| !text.is_empty());
        match prompt {
            Some(text) => {
                if text.chars().count() > MAX_PROMPT_CHARS {
                    return Err(format!(
                        "The request must be {MAX_PROMPT_CHARS} characters or fewer"
                    ));
                }
                let sealed = fields::seal(fields::ASSISTANT_PROMPT, text)
                    .map_err(|error| error.to_string())?;
                sqlx::query(
                    "INSERT INTO assistant_prompts (id, prompt, updated_at) VALUES (?, ?, ?) \
                     ON CONFLICT(id) DO UPDATE SET \
                         prompt = excluded.prompt, updated_at = excluded.updated_at",
                )
                .bind(id)
                .bind(sealed)
                .bind(Utc::now().to_rfc3339())
                .execute(pool)
                .await
                .map_err(|error| error.to_string())?;
            }
            None => {
                sqlx::query("DELETE FROM assistant_prompts WHERE id = ?")
                    .bind(id)
                    .execute(pool)
                    .await
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(())
    }
}

/// The owner's own overview request, or `None` for the default.
#[tauri::command]
pub async fn get_person_overview_prompt(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    AssistantPromptsRepository::get(state.db_manager.pool(), PERSON_OVERVIEW)
        .await
        .map_err(|error| error.to_string())
}

/// Saves the owner's own overview request; empty restores the default.
#[tauri::command]
pub async fn set_person_overview_prompt(
    state: tauri::State<'_, AppState>,
    prompt: Option<String>,
) -> Result<(), String> {
    AssistantPromptsRepository::set(state.db_manager.pool(), PERSON_OVERVIEW, prompt.as_deref())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(include_str!(
            "../../../migrations/20261007010000_add_assistant_prompts.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn the_default_is_used_until_the_owner_writes_their_own() {
        let pool = pool().await;
        assert_eq!(AssistantPromptsRepository::get(&pool, PERSON_OVERVIEW).await.unwrap(), None);

        AssistantPromptsRepository::set(&pool, PERSON_OVERVIEW, Some("  Свой запрос  "))
            .await
            .unwrap();
        assert_eq!(
            AssistantPromptsRepository::get(&pool, PERSON_OVERVIEW).await.unwrap().as_deref(),
            Some("Свой запрос")
        );

        // Blank goes back to the default rather than storing an empty request.
        AssistantPromptsRepository::set(&pool, PERSON_OVERVIEW, Some("   ")).await.unwrap();
        assert_eq!(AssistantPromptsRepository::get(&pool, PERSON_OVERVIEW).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_overlong_request_is_refused() {
        let pool = pool().await;
        let long = "а".repeat(MAX_PROMPT_CHARS + 1);
        assert!(AssistantPromptsRepository::set(&pool, PERSON_OVERVIEW, Some(&long))
            .await
            .is_err());
        assert_eq!(AssistantPromptsRepository::get(&pool, PERSON_OVERVIEW).await.unwrap(), None);
    }
}
