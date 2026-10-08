//! What a locked archive does with a write: refuses it.
//!
//! Its own binary because the key session is process-wide and set once — here
//! it is installed with a password and no key, which is the state the idle
//! lock leaves behind. Before this was a refusal, a summary finishing after
//! the lock wrote its text to the database in the clear.

use std::sync::Arc;

use app_lib::api::TranscriptSegment;
use app_lib::database::fields;
use app_lib::database::repositories::summary::SummaryProcessesRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::meeting_notes::{self, Note};
use app_lib::security::session::{self, KeySession};
use sqlx::SqlitePool;

fn lock_the_archive() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let key_session = Arc::new(KeySession::new());
        // A password exists; the key is not loaded.
        key_session.set_configured(true);
        session::install(key_session);
    });
}

async fn archive() -> SqlitePool {
    lock_the_archive();
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT NOT NULL, \
             created_at TEXT NOT NULL, updated_at TEXT NOT NULL, folder_path TEXT, \
             client_id TEXT, title_is_manual INTEGER NOT NULL DEFAULT 1); \
         CREATE TABLE transcripts (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, \
             transcript TEXT NOT NULL, timestamp TEXT NOT NULL, summary TEXT, \
             action_items TEXT, key_points TEXT, audio_start_time REAL, \
             audio_end_time REAL, duration REAL, speaker TEXT, words TEXT, edited_at TEXT); \
         CREATE TABLE summary_processes (meeting_id TEXT PRIMARY KEY, status TEXT, \
             created_at TEXT, updated_at TEXT, start_time TEXT, end_time TEXT, \
             error TEXT, result TEXT, chunk_count INTEGER, processing_time REAL, \
             metadata TEXT, result_backup TEXT, result_backup_timestamp TEXT);",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Taken from the migration itself, so this table cannot fall behind.
    sqlx::raw_sql(include_str!("../migrations/20251223000000_add_meeting_notes.sql"))
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// Every text value in the tables, for the crude check that matters.
async fn every_stored_value(pool: &SqlitePool) -> String {
    let mut all = String::new();
    for query in [
        "SELECT title FROM meetings",
        "SELECT transcript FROM transcripts",
        "SELECT COALESCE(result, '') FROM summary_processes",
        "SELECT COALESCE(notes_json, '') FROM meeting_notes",
    ] {
        let rows: Vec<(String,)> = sqlx::query_as(query).fetch_all(pool).await.unwrap();
        for (value,) in rows {
            all.push_str(&value);
            all.push('\n');
        }
    }
    all
}

#[tokio::test]
async fn sealing_refuses_while_locked() {
    lock_the_archive();
    let error = fields::seal(fields::MEETING_TITLE, "Встреча").unwrap_err();
    assert!(fields::is_archive_locked(&error));
    assert!(fields::seal_joinable(fields::TRANSCRIPT_SPEAKER, "Анна").is_err());
    assert!(fields::lookup(fields::PERSON_LOOKUP, "анна").is_err());
    // NULL has nothing to seal and stays allowed.
    assert_eq!(fields::seal_opt(fields::CLIENT_NOTES, None).unwrap(), None);
}

#[tokio::test]
async fn a_summary_finishing_after_the_lock_is_not_stored_in_the_clear() {
    let pool = archive().await;
    sqlx::query(
        "INSERT INTO summary_processes (meeting_id, status, created_at, updated_at) \
         VALUES ('m1', 'PENDING', '', '')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let result = serde_json::json!({ "markdown": "Обсуждали план релиза" });
    let outcome =
        SummaryProcessesRepository::update_process_completed(&pool, "m1", result, 1, 1.0).await;

    assert!(outcome.is_err(), "the write must be refused");
    assert!(!every_stored_value(&pool).await.contains("план релиза"));
}

#[tokio::test]
async fn transcript_lines_are_not_stored_in_the_clear() {
    let pool = archive().await;
    let segment = TranscriptSegment {
        id: "s1".to_string(),
        text: "Секретная фраза".to_string(),
        timestamp: "00:00:01".to_string(),
        audio_start_time: Some(1.0),
        audio_end_time: Some(2.0),
        duration: Some(1.0),
        speaker: Some("You".to_string()),
    };
    let outcome = TranscriptsRepository::save_transcript(
        &pool,
        "Название встречи",
        &[segment],
        &[],
        None,
        None,
    )
    .await;

    assert!(outcome.is_err(), "the write must be refused");
    let stored = every_stored_value(&pool).await;
    assert!(!stored.contains("Секретная"));
    assert!(!stored.contains("Название встречи"));
}

#[tokio::test]
async fn the_vocabulary_and_a_key_are_not_stored_in_the_clear() {
    use app_lib::database::repositories::setting::SettingsRepository;
    use app_lib::database::repositories::vocabulary::VocabularyRepository;

    let pool = archive().await;
    sqlx::raw_sql(
        "CREATE TABLE settings (id TEXT PRIMARY KEY, provider TEXT NOT NULL, \
             model TEXT NOT NULL, whisperModel TEXT NOT NULL, openaiApiKey TEXT); \
         CREATE TABLE transcript_settings (id TEXT PRIMARY KEY, provider TEXT NOT NULL, \
             model TEXT NOT NULL, whisperVocabulary TEXT); \
         CREATE TABLE meeting_whisper_vocabulary (meeting_id TEXT PRIMARY KEY, \
             vocabulary TEXT NOT NULL, updated_at TEXT NOT NULL);",
    )
    .execute(&pool)
    .await
    .unwrap();

    let global = VocabularyRepository::save_global(&pool, Some("Анна Петрова")).await;
    assert!(fields::is_archive_locked(&global.unwrap_err()));
    assert!(VocabularyRepository::save_meeting(&pool, "m1", "Пульсарова")
        .await
        .is_err());
    let key = SettingsRepository::save_api_key(&pool, "openai", "openai-key-for-tests").await;
    assert!(fields::is_archive_locked(&key.unwrap_err()));

    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT COALESCE(whisperVocabulary, '') FROM transcript_settings \
         UNION ALL SELECT vocabulary FROM meeting_whisper_vocabulary \
         UNION ALL SELECT COALESCE(openaiApiKey, '') FROM settings",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(stored.iter().all(String::is_empty), "written while locked: {stored:?}");
}

#[tokio::test]
async fn a_note_written_during_the_recording_is_not_stored_in_the_clear() {
    // Today the title is refused before the notes are reached; this keeps the
    // notes covered if the order of the writes ever changes. The note's own
    // sealing is checked by the next test.
    let pool = archive().await;
    let note = Note::new("Перезвонить в четверг", Some(5.0)).unwrap();
    let outcome = TranscriptsRepository::save_transcript(
        &pool,
        "Название встречи",
        &[],
        &[note],
        None,
        None,
    )
    .await;

    assert!(outcome.is_err(), "the write must be refused");
    assert!(!every_stored_value(&pool).await.contains("Перезвонить"));
}

#[tokio::test]
async fn a_note_added_to_a_saved_meeting_is_not_stored_in_the_clear() {
    let pool = archive().await;
    // Saved before the lock: the title is whatever was sealed then.
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at) \
         VALUES ('m1', 'sealed-title', '', '')",
    )
    .execute(&pool)
    .await
    .unwrap();

    let note = Note::new("Перезвонить в четверг", None).unwrap();
    let mut conn = pool.acquire().await.unwrap();
    let error = meeting_notes::store(&mut conn, "m1", &[note])
        .await
        .expect_err("the write must be refused");
    drop(conn);

    assert!(fields::is_archive_locked(&error));
    assert!(!every_stored_value(&pool).await.contains("Перезвонить"));
}
