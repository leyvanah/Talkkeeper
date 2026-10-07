use crate::database::fields;
use crate::database::models::{Setting, TranscriptSetting};
use crate::security::field::Field;
use crate::summary::CustomOpenAIConfig;
use sqlx::SqlitePool;

#[derive(serde::Deserialize, Debug)]
pub struct SaveModelConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    #[serde(rename = "ollamaEndpoint")]
    pub ollama_endpoint: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
pub struct SaveTranscriptConfigRequest {
    pub provider: String,
    pub model: String,
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
}

pub struct SettingsRepository;

// Transcript providers: localWhisper, deepgram, elevenLabs, groq, openai
// Summary providers: openai, claude, ollama, groq, added openrouter
// NOTE: Handle data exclusion in the higher layer as this is database abstraction layer(using SELECT *)

/// The column a summary provider's key is stored in, or `None` for a provider
/// that takes no key. Custom OpenAI keeps its key inside its own JSON document
/// and is handled by each caller before this.
fn summary_key_field(provider: &str) -> Result<Option<Field>, sqlx::Error> {
    match provider {
        "openai" => Ok(Some(fields::SUMMARY_KEY_OPENAI)),
        "claude" => Ok(Some(fields::SUMMARY_KEY_ANTHROPIC)),
        "ollama" => Ok(Some(fields::SUMMARY_KEY_OLLAMA)),
        "groq" => Ok(Some(fields::SUMMARY_KEY_GROQ)),
        "openrouter" => Ok(Some(fields::SUMMARY_KEY_OPENROUTER)),
        "builtin-ai" => Ok(None), // No API key needed
        _ => Err(sqlx::Error::Protocol(
            format!("Invalid provider: {}", provider).into(),
        )),
    }
}

/// The column a transcription provider's key is stored in, or `None` for an
/// engine that takes no key.
fn transcript_key_field(provider: &str) -> Result<Option<Field>, sqlx::Error> {
    match provider {
        "localWhisper" => Ok(Some(fields::TRANSCRIPT_KEY_WHISPER)),
        // Parakeet and GigaAM run locally and take no key
        "parakeet" | "gigaam" => Ok(None),
        // The external STT service keeps its token inside externalSttConfig
        "externalStt" => Ok(None),
        "deepgram" => Ok(Some(fields::TRANSCRIPT_KEY_DEEPGRAM)),
        "elevenLabs" => Ok(Some(fields::TRANSCRIPT_KEY_ELEVENLABS)),
        "groq" => Ok(Some(fields::TRANSCRIPT_KEY_GROQ)),
        "openai" => Ok(Some(fields::TRANSCRIPT_KEY_OPENAI)),
        _ => Err(sqlx::Error::Protocol(
            format!("Invalid provider: {}", provider).into(),
        )),
    }
}

/// Reads one sealed column of the single settings row and opens it. A `NULL`
/// column is no value, not an error.
async fn read_sealed(
    pool: &SqlitePool,
    field: Field,
) -> std::result::Result<Option<String>, sqlx::Error> {
    let query = format!(
        "SELECT {} FROM {} WHERE id = '1' LIMIT 1",
        field.column, field.table
    );
    let stored: Option<String> = sqlx::query_scalar::<_, Option<String>>(&query)
        .fetch_optional(pool)
        .await?
        .flatten();
    fields::open_opt(field, stored)
}

impl SettingsRepository {
    pub async fn get_model_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<Setting>, sqlx::Error> {
        let setting = sqlx::query_as::<_, Setting>("SELECT * FROM settings LIMIT 1")
            .fetch_optional(pool)
            .await?;
        Ok(setting)
    }

    pub async fn save_model_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
        whisper_model: &str,
        ollama_endpoint: Option<&str>,
    ) -> std::result::Result<(), sqlx::Error> {
        // Using id '1' for backward compatibility
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, ollamaEndpoint)
            VALUES ('1', $1, $2, $3, $4)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model,
                whisperModel = excluded.whisperModel,
                ollamaEndpoint = excluded.ollamaEndpoint
            "#,
        )
        .bind(provider)
        .bind(model)
        .bind(whisper_model)
        .bind(ollama_endpoint)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_api_key(
        pool: &SqlitePool,
        provider: &str,
        api_key: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Custom OpenAI uses JSON config (customOpenAIConfig) instead of a separate API key column
        if provider == "custom-openai" {
            return Err(sqlx::Error::Protocol(
                "custom-openai provider should use save_custom_openai_config() instead of save_api_key()".into(),
            ));
        }

        let Some(field) = summary_key_field(provider)? else {
            return Ok(());
        };

        let query = format!(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, "{}")
            VALUES ('1', 'openai', 'gpt-4o-2024-11-20', 'large-v3', $1)
            ON CONFLICT(id) DO UPDATE SET
                "{}" = $1
            "#,
            field.column, field.column
        );
        sqlx::query(&query)
            .bind(fields::seal(field, api_key)?)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn get_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        // Custom OpenAI uses JSON config - extract API key from there
        if provider == "custom-openai" {
            let config = Self::get_custom_openai_config(pool).await?;
            return Ok(config.and_then(|c| c.api_key));
        }

        match summary_key_field(provider)? {
            Some(field) => read_sealed(pool, field).await,
            None => Ok(None),
        }
    }

    pub async fn get_transcript_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<TranscriptSetting>, sqlx::Error> {
        let setting =
            sqlx::query_as::<_, TranscriptSetting>("SELECT * FROM transcript_settings LIMIT 1")
                .fetch_optional(pool)
                .await?;
        Ok(setting)

    }

    pub async fn save_transcript_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO transcript_settings (id, provider, model)
            VALUES ('1', $1, $2)
            ON CONFLICT(id) DO UPDATE SET
                provider = excluded.provider,
                model = excluded.model
            "#,
        )
        .bind(provider)
        .bind(model)
        .execute(pool)
        .await?;

        Ok(())
    }

    /// Raw JSON of the external HTTP STT settings, if the owner configured one.
    pub async fn get_external_stt_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        read_sealed(pool, fields::EXTERNAL_STT_CONFIG).await
    }

    pub async fn save_external_stt_config(
        pool: &SqlitePool,
        config_json: Option<&str>,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO transcript_settings (id, provider, model, externalSttConfig)
            VALUES ('1', 'parakeet', $1, $2)
            ON CONFLICT(id) DO UPDATE SET
                externalSttConfig = excluded.externalSttConfig
            "#,
        )
        .bind(crate::config::DEFAULT_PARAKEET_MODEL)
        .bind(fields::seal_opt(fields::EXTERNAL_STT_CONFIG, config_json)?)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn get_post_call_transcript_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<(String, String)>, sqlx::Error> {
        sqlx::query_as::<_, (String, String)>(
            r#"
            SELECT postCallProvider, postCallModel
            FROM transcript_settings
            WHERE id = '1'
              AND postCallProvider IS NOT NULL
              AND postCallModel IS NOT NULL
            LIMIT 1
            "#,
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn save_post_call_transcript_config(
        pool: &SqlitePool,
        provider: &str,
        model: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO transcript_settings (id, provider, model, postCallProvider, postCallModel)
            VALUES ('1', 'parakeet', $1, $2, $3)
            ON CONFLICT(id) DO UPDATE SET
                postCallProvider = excluded.postCallProvider,
                postCallModel = excluded.postCallModel
            "#,
        )
        .bind(crate::config::DEFAULT_PARAKEET_MODEL)
        .bind(provider)
        .bind(model)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn save_transcript_api_key(
        pool: &SqlitePool,
        provider: &str,
        api_key: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        let Some(field) = transcript_key_field(provider)? else {
            return Ok(());
        };

        let query = format!(
            r#"
            INSERT INTO transcript_settings (id, provider, model, "{}")
            VALUES ('1', 'parakeet', '{}', $1)
            ON CONFLICT(id) DO UPDATE SET
                "{}" = $1
            "#,
            field.column, crate::config::DEFAULT_PARAKEET_MODEL, field.column
        );
        sqlx::query(&query)
            .bind(fields::seal(field, api_key)?)
            .execute(pool)
            .await?;

        Ok(())
    }

    pub async fn get_transcript_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<Option<String>, sqlx::Error> {
        match transcript_key_field(provider)? {
            Some(field) => read_sealed(pool, field).await,
            None => Ok(None),
        }
    }

    pub async fn delete_api_key(
        pool: &SqlitePool,
        provider: &str,
    ) -> std::result::Result<(), sqlx::Error> {
        // Custom OpenAI uses JSON config - clear the entire config
        if provider == "custom-openai" {
            sqlx::query("UPDATE settings SET customOpenAIConfig = NULL WHERE id = '1'")
                .execute(pool)
                .await?;
            return Ok(());
        }

        let Some(field) = summary_key_field(provider)? else {
            return Ok(());
        };

        let query = format!(
            "UPDATE settings SET {} = NULL WHERE id = '1'",
            field.column
        );
        sqlx::query(&query).execute(pool).await?;

        Ok(())
    }

    // ===== CUSTOM OPENAI CONFIG METHODS =====

    /// Gets the custom OpenAI configuration from JSON
    ///
    /// # Returns
    /// * `Ok(Some(CustomOpenAIConfig))` - Config exists and is valid JSON
    /// * `Ok(None)` - No config stored
    /// * `Err(sqlx::Error)` - Database error
    pub async fn get_custom_openai_config(
        pool: &SqlitePool,
    ) -> std::result::Result<Option<CustomOpenAIConfig>, sqlx::Error> {
        match read_sealed(pool, fields::CUSTOM_OPENAI_CONFIG).await? {
            Some(json) => {
                // Parse JSON into CustomOpenAIConfig
                let config: CustomOpenAIConfig = serde_json::from_str(&json)
                    .map_err(|e| sqlx::Error::Protocol(
                        format!("Invalid JSON in customOpenAIConfig: {}", e).into()
                    ))?;

                Ok(Some(config))
            }
            None => Ok(None),
        }
    }

    /// Saves the custom OpenAI configuration as JSON
    ///
    /// # Arguments
    /// * `pool` - Database connection pool
    /// * `config` - CustomOpenAIConfig to save (includes endpoint, apiKey, model, maxTokens, temperature, topP)
    ///
    /// # Returns
    /// * `Ok(())` - Config saved successfully
    /// * `Err(sqlx::Error)` - Database or JSON serialization error
    pub async fn save_custom_openai_config(
        pool: &SqlitePool,
        config: &CustomOpenAIConfig,
    ) -> std::result::Result<(), sqlx::Error> {
        // Serialize config to JSON
        let config_json = serde_json::to_string(config)
            .map_err(|e| sqlx::Error::Protocol(
                format!("Failed to serialize config to JSON: {}", e).into()
            ))?;

        // Upsert into settings table
        sqlx::query(
            r#"
            INSERT INTO settings (id, provider, model, whisperModel, customOpenAIConfig)
            VALUES ('1', 'custom-openai', $1, 'large-v3', $2)
            ON CONFLICT(id) DO UPDATE SET
                customOpenAIConfig = excluded.customOpenAIConfig
            "#,
        )
        .bind(&config.model)
        .bind(fields::seal(fields::CUSTOM_OPENAI_CONFIG, &config_json)?)
        .execute(pool)
        .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every provider the settings screen can store for live transcription.
    const LIVE_TRANSCRIPT_PROVIDERS: [&str; 4] =
        ["localWhisper", "parakeet", "gigaam", "externalStt"];

    async fn migrated_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        crate::database::manager::MIGRATOR
            .run(&pool)
            .await
            .expect("migrations");
        pool
    }

    /// Reading the transcript config asks for the provider's API key, so a
    /// provider missing from that lookup makes the whole config unreadable and
    /// the app silently falls back to another engine.
    #[tokio::test]
    async fn every_live_provider_can_be_saved_and_read_back() {
        let pool = migrated_pool().await;

        for provider in LIVE_TRANSCRIPT_PROVIDERS {
            SettingsRepository::save_transcript_config(&pool, provider, "some-model")
                .await
                .unwrap_or_else(|error| panic!("saving '{}' failed: {}", provider, error));

            let config = SettingsRepository::get_transcript_config(&pool)
                .await
                .unwrap_or_else(|error| panic!("reading back '{}' failed: {}", provider, error))
                .unwrap_or_else(|| panic!("no config stored for '{}'", provider));
            assert_eq!(config.provider, provider);

            SettingsRepository::get_transcript_api_key(&pool, provider)
                .await
                .unwrap_or_else(|error| {
                    panic!("no API key lookup for '{}': {}", provider, error)
                });
        }
    }

    /// The locally running engines never carry a key, so storing one is a no-op
    /// rather than an error.
    #[tokio::test]
    async fn local_engines_ignore_api_keys() {
        let pool = migrated_pool().await;

        for provider in ["parakeet", "gigaam", "externalStt"] {
            SettingsRepository::save_transcript_api_key(&pool, provider, "ignored")
                .await
                .unwrap_or_else(|error| panic!("saving a key for '{}' failed: {}", provider, error));
            assert_eq!(
                SettingsRepository::get_transcript_api_key(&pool, provider)
                    .await
                    .expect("key lookup"),
                None
            );
        }
    }
}
