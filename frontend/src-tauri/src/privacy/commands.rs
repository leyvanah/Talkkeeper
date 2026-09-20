//! What the window can ask and change about hiding.

use tauri::State;

use super::settings::{self, PrivacySettings};
use super::vocabulary;
use crate::state::AppState;

/// The settings as they stand, plus the names the archive already knows.
///
/// The window shows those names so that the owner can see what would be
/// hidden without having to guess, and add what is missing.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyOverview {
    #[serde(flatten)]
    pub settings: PrivacySettings,
    /// Names gathered from the library and from the recordings.
    pub known_names: Vec<String>,
}

#[tauri::command]
pub async fn api_get_privacy_settings(
    state: State<'_, AppState>,
) -> Result<PrivacyOverview, String> {
    let pool = state.db_manager.pool();
    let settings = settings::load(pool).await?;
    let known_names = vocabulary::for_meeting(pool, None).await.names;
    Ok(PrivacyOverview {
        settings,
        known_names,
    })
}

#[tauri::command]
pub async fn api_save_privacy_settings(
    state: State<'_, AppState>,
    anonymize_cloud: bool,
    hidden_terms: Vec<String>,
) -> Result<(), String> {
    settings::save(
        state.db_manager.pool(),
        &PrivacySettings {
            anonymize_cloud,
            hidden_terms,
        },
    )
    .await?;
    log::info!("Privacy settings saved (hiding before the cloud: {anonymize_cloud})");
    Ok(())
}
