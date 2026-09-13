use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{FromRow, Row};

use crate::database::fields;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingModel {
    pub id: String,
    pub title: String,
    pub created_at: DateTimeUtc,
    pub updated_at: DateTimeUtc,
    pub folder_path: Option<String>,
    /// Client this meeting is filed under; `None` while it is unassigned.
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct DateTimeUtc(pub DateTime<Utc>);

impl From<NaiveDateTime> for DateTimeUtc {
    fn from(naive: NaiveDateTime) -> Self {
        DateTimeUtc(DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc))
    }
}

// Renamed from TranscriptSegment to Transcript to match the table name
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    pub id: String,
    pub meeting_id: String,
    pub transcript: String,
    pub timestamp: String,
    pub summary: Option<String>,
    pub action_items: Option<String>,
    pub key_points: Option<String>,
    // Recording-relative timestamps for audio-transcript synchronization
    pub audio_start_time: Option<f64>,
    pub audio_end_time: Option<f64>,
    pub duration: Option<f64>,
    /// Speaker label: capture source ("You"/"Guest") or diarization ("Speaker N")
    pub speaker: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryProcess {
    pub meeting_id: String,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub error: Option<String>,
    pub result: Option<String>, // JSON
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
    pub chunk_count: i64,
    pub processing_time: f64,
    pub metadata: Option<String>, // JSON
    pub result_backup: Option<String>, // Backup of result before regeneration
    pub result_backup_timestamp: Option<chrono::DateTime<chrono::Utc>>, // When backup was created
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptChunk {
    pub meeting_id: String,
    pub meeting_name: Option<String>,
    pub transcript_text: String,
    pub model: String,
    pub model_name: String,
    pub chunk_size: Option<i64>,
    pub overlap: Option<i64>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct Setting {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[sqlx(rename = "whisperModel")]
    #[serde(rename = "whisperModel")]
    pub whisper_model: String,
    #[sqlx(rename = "groqApiKey")]
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: Option<String>,
    #[sqlx(rename = "openaiApiKey")]
    #[serde(rename = "openaiApiKey")]
    pub openai_api_key: Option<String>,
    #[sqlx(rename = "anthropicApiKey")]
    #[serde(rename = "anthropicApiKey")]
    pub anthropic_api_key: Option<String>,
    #[sqlx(rename = "ollamaApiKey")]
    #[serde(rename = "ollamaApiKey")]
    pub ollama_api_key: Option<String>,
    #[sqlx(rename = "openRouterApiKey")]
    #[serde(rename = "openRouterApiKey")]
    pub open_router_api_key: Option<String>,
    #[sqlx(rename = "ollamaEndpoint")]
    #[serde(rename = "ollamaEndpoint")]
    pub ollama_endpoint: Option<String>,
    /// Custom OpenAI-compatible endpoint configuration stored as JSON
    #[sqlx(rename = "customOpenAIConfig")]
    #[serde(rename = "customOpenAIConfig")]
    pub custom_openai_config: Option<String>,
}

impl Setting {
    /// Parse the custom OpenAI config from JSON string
    pub fn get_custom_openai_config(&self) -> Option<crate::summary::CustomOpenAIConfig> {
        self.custom_openai_config.as_ref().and_then(|json| {
            serde_json::from_str(json).ok()
        })
    }
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct TranscriptSetting {
    pub id: String,
    pub provider: String,
    pub model: String,
    #[sqlx(rename = "whisperApiKey")]
    #[serde(rename = "whisperApiKey")]
    pub whisper_api_key: Option<String>,
    #[sqlx(rename = "deepgramApiKey")]
    #[serde(rename = "deepgramApiKey")]
    pub deepgram_api_key: Option<String>,
    #[sqlx(rename = "elevenLabsApiKey")]
    #[serde(rename = "elevenLabsApiKey")]
    pub eleven_labs_api_key: Option<String>,
    #[sqlx(rename = "groqApiKey")]
    #[serde(rename = "groqApiKey")]
    pub groq_api_key: Option<String>,
    #[sqlx(rename = "openaiApiKey")]
    #[serde(rename = "openaiApiKey")]
    pub openai_api_key: Option<String>,
}

// ---------------------------------------------------------------------------
// Reading sealed columns back
//
// These four structures are how the rest of the application receives a row, so
// implementing `FromRow` by hand — instead of deriving it — makes the archive
// key part of loading a row rather than something each query has to remember.
// A query written next year that selects a meeting gets the title in the clear
// without knowing that B4 exists; one that forgets would not compile, because
// there is no other way to build the struct.
//
// The reverse direction cannot be centralised the same way: sqlx binds values,
// not structs, so writes call `fields::seal` at each `bind`.
// ---------------------------------------------------------------------------

impl<'r> FromRow<'r, SqliteRow> for MeetingModel {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            title: fields::open(fields::MEETING_TITLE, &row.try_get::<String, _>("title")?)?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            folder_path: row.try_get("folder_path")?,
            client_id: row.try_get("client_id")?,
        })
    }
}

impl<'r> FromRow<'r, SqliteRow> for Transcript {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            meeting_id: row.try_get("meeting_id")?,
            transcript: fields::open(
                fields::TRANSCRIPT_TEXT,
                &row.try_get::<String, _>("transcript")?,
            )?,
            timestamp: row.try_get("timestamp")?,
            summary: fields::open_opt(fields::TRANSCRIPT_SUMMARY, row.try_get("summary")?)?,
            action_items: fields::open_opt(
                fields::TRANSCRIPT_ACTION_ITEMS,
                row.try_get("action_items")?,
            )?,
            key_points: fields::open_opt(
                fields::TRANSCRIPT_KEY_POINTS,
                row.try_get("key_points")?,
            )?,
            audio_start_time: row.try_get("audio_start_time")?,
            audio_end_time: row.try_get("audio_end_time")?,
            duration: row.try_get("duration")?,
            speaker: fields::open_opt(fields::TRANSCRIPT_SPEAKER, row.try_get("speaker")?)?,
        })
    }
}

impl<'r> FromRow<'r, SqliteRow> for SummaryProcess {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            meeting_id: row.try_get("meeting_id")?,
            status: row.try_get("status")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            error: row.try_get("error")?,
            result: fields::open_opt(fields::SUMMARY_RESULT, row.try_get("result")?)?,
            start_time: row.try_get("start_time")?,
            end_time: row.try_get("end_time")?,
            chunk_count: row.try_get("chunk_count")?,
            processing_time: row.try_get("processing_time")?,
            metadata: row.try_get("metadata")?,
            result_backup: fields::open_opt(
                fields::SUMMARY_RESULT_BACKUP,
                row.try_get("result_backup")?,
            )?,
            result_backup_timestamp: row.try_get("result_backup_timestamp")?,
        })
    }
}

impl<'r> FromRow<'r, SqliteRow> for TranscriptChunk {
    fn from_row(row: &'r SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            meeting_id: row.try_get("meeting_id")?,
            meeting_name: fields::open_opt(
                fields::CHUNK_MEETING_NAME,
                row.try_get("meeting_name")?,
            )?,
            transcript_text: fields::open(
                fields::CHUNK_TEXT,
                &row.try_get::<String, _>("transcript_text")?,
            )?,
            model: row.try_get("model")?,
            model_name: row.try_get("model_name")?,
            chunk_size: row.try_get("chunk_size")?,
            overlap: row.try_get("overlap")?,
            created_at: row.try_get("created_at")?,
        })
    }
}
