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
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            anonymize_cloud: true,
            hidden_terms: Vec::new(),
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
    let row: Option<(i64, Option<String>)> =
        sqlx::query_as("SELECT anonymize_cloud, hidden_terms FROM privacy_settings WHERE id = '1'")
            .fetch_optional(pool)
            .await
            .map_err(|error| format!("Could not read the privacy settings: {error}"))?;

    let Some((anonymize_cloud, sealed_terms)) = row else {
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
        INSERT INTO privacy_settings (id, anonymize_cloud, hidden_terms)
        VALUES ('1', $1, $2)
        ON CONFLICT(id) DO UPDATE SET
            anonymize_cloud = excluded.anonymize_cloud,
            hidden_terms = excluded.hidden_terms
        "#,
    )
    .bind(i64::from(settings.anonymize_cloud))
    .bind(sealed)
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
    }
}
