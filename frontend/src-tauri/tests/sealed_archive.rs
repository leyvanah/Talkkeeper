//! What B4 promises, checked against a real database with a real key.
//!
//! The unit tests in the library all run with no archive key, because the
//! process-wide session is set once and the first setter wins — a test that
//! installed a key would turn encryption on for every other test in the same
//! binary, and the order they run in is not fixed. An integration test is its
//! own binary with its own statics, so here the key can be installed for the
//! whole run and the repositories can be exercised as the application uses
//! them.
//!
//! The one assertion that matters most is the crude one: read the column back
//! as raw SQL and look for the words. If they are there, everything else in
//! this file is decoration.

use std::sync::Arc;

use app_lib::api::TranscriptSegment;
use app_lib::database::fields;
use app_lib::database::repositories::client::ClientsRepository;
use app_lib::database::repositories::meeting::MeetingsRepository;
use app_lib::database::repositories::person::PeopleRepository;
use app_lib::database::repositories::transcript::TranscriptsRepository;
use app_lib::security::envelope::generate_dek;
use app_lib::security::session::{self, KeySession};
use sqlx::SqlitePool;

/// Installs an archive key for this test binary. Called by every test; only the
/// first call does anything, which is exactly the behaviour wanted.
fn open_the_archive() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let key_session = Arc::new(KeySession::new());
        key_session.unlock(generate_dek());
        session::install(key_session);
    });
}

/// The tables the repositories under test touch, in their real shape.
async fn archive() -> SqlitePool {
    open_the_archive();
    let pool = SqlitePool::connect(":memory:").await.unwrap();
    sqlx::raw_sql(
        "CREATE TABLE meetings (id TEXT PRIMARY KEY, title TEXT NOT NULL, \
             created_at TEXT NOT NULL, updated_at TEXT NOT NULL, folder_path TEXT, \
             client_id TEXT, title_is_manual INTEGER NOT NULL DEFAULT 1); \
         CREATE TABLE transcripts (id TEXT PRIMARY KEY, meeting_id TEXT NOT NULL, \
             transcript TEXT NOT NULL, timestamp TEXT NOT NULL, summary TEXT, \
             action_items TEXT, key_points TEXT, audio_start_time REAL, \
             audio_end_time REAL, duration REAL, speaker TEXT); \
         CREATE TABLE summary_processes (meeting_id TEXT PRIMARY KEY, result TEXT); \
         CREATE TABLE transcript_chunks (meeting_id TEXT PRIMARY KEY, meeting_name TEXT); \
         CREATE TABLE people (id TEXT PRIMARY KEY, display_name TEXT NOT NULL, \
             normalized_name TEXT NOT NULL UNIQUE, notes TEXT, created_at TEXT NOT NULL, \
             updated_at TEXT NOT NULL); \
         CREATE TABLE person_speakers (person_id TEXT NOT NULL, meeting_id TEXT NOT NULL, \
             speaker_label TEXT NOT NULL, UNIQUE(meeting_id, speaker_label)); \
         CREATE TABLE clients (id TEXT PRIMARY KEY, \
             display_name TEXT NOT NULL CHECK (length(trim(display_name)) > 0), \
             normalized_name TEXT NOT NULL CHECK (length(normalized_name) > 0), \
             notes TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool
}

fn segment(id: &str, text: &str, speaker: &str, start: f64) -> TranscriptSegment {
    TranscriptSegment {
        id: id.to_string(),
        text: text.to_string(),
        timestamp: "2026-09-12T10:00:00Z".to_string(),
        audio_start_time: Some(start),
        audio_end_time: Some(start + 2.0),
        duration: Some(2.0),
        speaker: Some(speaker.to_string()),
    }
}

/// Everything written through the repositories, read back as raw SQL.
async fn every_stored_value(pool: &SqlitePool) -> String {
    let mut dump = String::new();
    for query in [
        "SELECT title FROM meetings",
        "SELECT transcript FROM transcripts",
        "SELECT COALESCE(speaker, '') FROM transcripts",
        "SELECT display_name FROM clients",
        "SELECT normalized_name FROM clients",
        "SELECT COALESCE(notes, '') FROM clients",
    ] {
        let values: Vec<String> = sqlx::query_scalar(query).fetch_all(pool).await.unwrap();
        dump.push_str(&values.join("\n"));
        dump.push('\n');
    }
    dump
}

#[tokio::test]
async fn the_words_are_not_in_the_database_file() {
    let pool = archive().await;

    let meeting_id = TranscriptsRepository::save_transcript(
        &pool,
        "Встреча в четверг",
        &[
            segment("s1", "отчёт за квартал будет в пятницу", "Анна", 0.0),
            segment("s2", "давайте сверим числа", "You", 3.0),
        ],
        None,
        None,
    )
    .await
    .unwrap();

    let client = ClientsRepository::create(&pool, "Анна Петрова").await.unwrap();
    ClientsRepository::update_notes(&pool, &client.id, Some("звонить до обеда"))
        .await
        .unwrap();

    let stored = every_stored_value(&pool).await;
    for word in [
        "Встреча",
        "четверг",
        "пятницу",
        "неделю",
        "числа",
        "Анна",
        "Петрова",
        "звонить",
        "утро",
    ] {
        assert!(
            !stored.contains(word),
            "the word {word:?} is stored in the clear:\n{stored}"
        );
    }

    // And the application still reads them.
    let meeting = MeetingsRepository::get_meeting(&pool, &meeting_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(meeting.title, "Встреча в четверг");
    assert_eq!(meeting.transcripts.len(), 2);
    assert_eq!(
        meeting.transcripts[0].text,
        "отчёт за квартал будет в пятницу"
    );
    assert_eq!(meeting.transcripts[0].speaker.as_deref(), Some("Анна"));

    let clients = ClientsRepository::list(&pool).await.unwrap();
    assert_eq!(clients.len(), 1);
    assert_eq!(clients[0].display_name, "Анна Петрова");
    assert_eq!(clients[0].notes.as_deref(), Some("звонить до обеда"));
}

#[tokio::test]
async fn search_still_finds_a_sealed_word() {
    // The scan decrypts as it goes; if it did not, this would find nothing and
    // the owner would conclude the archive had lost the recording.
    let pool = archive().await;
    TranscriptsRepository::save_transcript(
        &pool,
        "Встреча в четверг",
        &[segment("s1", "отчёт будет в пятницу", "Анна", 0.0)],
        None,
        None,
    )
    .await
    .unwrap();

    let by_word = PeopleRepository::global_search(&pool, "ПЯТНИЦУ", None)
        .await
        .unwrap();
    assert_eq!(by_word.len(), 1);
    assert_eq!(by_word[0].kind, "transcript");
    assert!(by_word[0].snippet.contains("пятницу"));

    let by_title = PeopleRepository::global_search(&pool, "четверг", None)
        .await
        .unwrap();
    assert_eq!(by_title.len(), 1);
    assert_eq!(by_title[0].title, "Встреча в четверг");

    let by_speaker = PeopleRepository::global_search(&pool, "анна", None)
        .await
        .unwrap();
    assert_eq!(by_speaker.len(), 1);
    assert_eq!(by_speaker[0].speaker.as_deref(), Some("Анна"));
}

#[tokio::test]
async fn a_person_profile_still_joins_to_the_lines_they_said() {
    // The join this exercises (`t.speaker = ps.speaker_label`) is why those two
    // columns are sealed deterministically. Under a random nonce the two sealed
    // forms of "Анна" would differ, the join would match nothing, and the
    // profile would come back empty without any error being raised.
    let pool = archive().await;
    let meeting_id = TranscriptsRepository::save_transcript(
        &pool,
        "Первая",
        &[
            segment("s1", "первая реплика", "Анна", 0.0),
            segment("s2", "вторая реплика", "Анна", 3.0),
        ],
        None,
        None,
    )
    .await
    .unwrap();

    // The identity link, written the way the repository writes it.
    sqlx::query(
        "INSERT INTO people (id, display_name, normalized_name, notes, created_at, updated_at) \
         VALUES (?, ?, ?, NULL, 'now', 'now')",
    )
    .bind("person-anna")
    .bind(fields::seal(fields::PERSON_NAME, "Анна"))
    .bind(fields::lookup(fields::PERSON_LOOKUP, "анна"))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO person_speakers VALUES (?, ?, ?)")
        .bind("person-anna")
        .bind(&meeting_id)
        .bind(fields::seal_joinable(fields::SPEAKER_LABEL, "Анна"))
        .execute(&pool)
        .await
        .unwrap();

    let profile = PeopleRepository::get_profile(&pool, "person-anna")
        .await
        .unwrap();
    assert_eq!(profile.display_name, "Анна");
    assert_eq!(profile.meeting_count, 1);
    assert_eq!(
        profile.message_count, 2,
        "the join between transcripts and person_speakers found nothing"
    );
    assert_eq!(profile.meetings[0].title, "Первая");

    let stored: Vec<String> = sqlx::query_scalar("SELECT speaker_label FROM person_speakers")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(!stored[0].contains("Анна"));
}

#[tokio::test]
async fn the_same_name_is_recognised_through_the_blind_index() {
    // Two people may share a name, so this is a warning rather than a refusal —
    // but the lookup that raises it has to keep working against a column it can
    // no longer read.
    let pool = archive().await;
    ClientsRepository::create(&pool, "Анна").await.unwrap();
    ClientsRepository::create(&pool, "  анна  ").await.unwrap();

    let indexes: Vec<String> = sqlx::query_scalar("SELECT normalized_name FROM clients")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(indexes.len(), 2);
    assert_eq!(
        indexes[0], indexes[1],
        "the same name must give the same blind index"
    );
    assert!(!indexes[0].contains("анна"));
}
