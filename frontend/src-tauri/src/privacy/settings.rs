//! What the owner wants hidden, and whether hiding is on at all.
//!
//! The terms are sealed in the database like the rest of the text: a list of
//! the people in someone's life is exactly what this archive exists to keep.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::database::fields;

/// The settings as the window sees them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    /// Replace names before text goes to a model that is not on this machine.
    pub anonymize_cloud: bool,
    /// Words the owner added: places, employers, nicknames.
    pub hidden_terms: Vec<String>,
    /// Before hiding, let the built-in model look for names nobody listed.
    pub find_names_locally: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            anonymize_cloud: true,
            hidden_terms: Vec::new(),
            find_names_locally: true,
        }
    }
}

/// One term per line, which is what the window edits.
fn split_terms(stored: &str) -> Vec<String> {
    stored
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_string)
        .collect()
}

pub async fn load(pool: &SqlitePool) -> Result<PrivacySettings, String> {
    let row: Option<(i64, Option<String>, i64)> = sqlx::query_as(
        "SELECT anonymize_cloud, hidden_terms, find_names_locally FROM privacy_settings WHERE id = '1'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("Could not read the privacy settings: {error}"))?;

    let Some((anonymize_cloud, sealed_terms, find_names_locally)) = row else {
        return Ok(PrivacySettings::default());
    };

    let hidden_terms = match sealed_terms {
        Some(sealed) => {
            let opened = fields::open(fields::PRIVACY_HIDDEN_TERMS, &sealed)
                .map_err(|error| format!("Could not read the hidden terms: {error}"))?;
            split_terms(&opened)
        }
        None => Vec::new(),
    };

    Ok(PrivacySettings {
        anonymize_cloud: anonymize_cloud != 0,
        hidden_terms,
        find_names_locally: find_names_locally != 0,
    })
}

pub async fn save(pool: &SqlitePool, settings: &PrivacySettings) -> Result<(), String> {
    let terms = settings
        .hidden_terms
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let sealed = if terms.is_empty() {
        None
    } else {
        Some(
            fields::seal(fields::PRIVACY_HIDDEN_TERMS, &terms)
                .map_err(|error| format!("Could not seal the hidden terms: {error}"))?,
        )
    };

    sqlx::query(
        r#"
        INSERT INTO privacy_settings (id, anonymize_cloud, hidden_terms, find_names_locally)
        VALUES ('1', $1, $2, $3)
        ON CONFLICT(id) DO UPDATE SET
            anonymize_cloud = excluded.anonymize_cloud,
            hidden_terms = excluded.hidden_terms,
            find_names_locally = excluded.find_names_locally
        "#,
    )
    .bind(i64::from(settings.anonymize_cloud))
    .bind(sealed)
    .bind(i64::from(settings.find_names_locally))
    .execute(pool)
    .await
    .map_err(|error| format!("Could not save the privacy settings: {error}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stored_list_is_one_term_per_line_with_the_blanks_dropped() {
        let terms = split_terms("Простоквашино\n\n  Матроскин  \r\n");
        assert_eq!(terms, vec!["Простоквашино", "Матроскин"]);
    }

    #[test]
    fn hiding_is_on_before_anyone_chooses() {
        assert!(PrivacySettings::default().anonymize_cloud);
        assert!(PrivacySettings::default().find_names_locally);
    }

    #[tokio::test]
    async fn the_local_name_search_is_on_after_migration_and_keeps_the_owners_choice() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::manager::MIGRATOR.run(&pool).await.unwrap();

        assert!(load(&pool).await.unwrap().find_names_locally);

        let mut chosen = load(&pool).await.unwrap();
        chosen.find_names_locally = false;
        save(&pool, &chosen).await.unwrap();
        let reread = load(&pool).await.unwrap();
        assert!(!reread.find_names_locally);
        assert!(reread.anonymize_cloud);
    }
}
