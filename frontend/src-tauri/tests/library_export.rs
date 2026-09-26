//! The readable library export, checked against a sealed archive with a real
//! key: what is encrypted in the archive has to come out as plain text and as
//! the audio that was recorded.
//!
//! Its own test binary for the same reason as `sealed_archive.rs`: the key is
//! installed process-wide, once.

use std::io::Write;
use std::sync::Arc;

use app_lib::database::fields;
use app_lib::library_export::plain::PlainExport;
use app_lib::library_export::{collect_meeting, MeetingRecord, MEETING_NAME};
use app_lib::security::envelope::generate_dek;
use app_lib::security::session::{self, KeySession};
use app_lib::security::stream::{file_looks_encrypted, EncryptedWriter};
use sqlx::SqlitePool;

fn open_the_archive() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let key_session = Arc::new(KeySession::new());
        key_session.unlock(generate_dek());
        session::install(key_session);
    });
}

async fn archive() -> SqlitePool {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    pool
}

#[tokio::test]
async fn a_sealed_recording_comes_out_readable() {
    open_the_archive();
    let pool = archive().await;

    // The recording's audio, encrypted with the archive key as a recording is.
    let recording = tempfile::tempdir().unwrap();
    let audio: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    let file = std::fs::File::create(recording.path().join("audio.mp4")).unwrap();
    session::with_current_key(|key| {
        let mut writer = EncryptedWriter::create(file, key).unwrap();
        writer.write_all(&audio).unwrap();
        writer.finish().unwrap();
    })
    .unwrap();
    assert!(file_looks_encrypted(&recording.path().join("audio.mp4")));

    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path) \
         VALUES ('m1', ?, '2026-09-12T10:00:00Z', '2026-09-12T11:00:00Z', ?)",
    )
    .bind(fields::seal(fields::MEETING_TITLE, "Разговор о переезде").unwrap())
    .bind(recording.path().to_string_lossy().to_string())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker, audio_start_time, audio_end_time) \
         VALUES ('t1', 'm1', ?, '10:00:01', ?, 1.0, 3.0)",
    )
    .bind(fields::seal(fields::TRANSCRIPT_TEXT, "Мы переезжаем в марте").unwrap())
    .bind(fields::seal_joinable(fields::TRANSCRIPT_SPEAKER, "Борис").unwrap())
    .execute(&pool)
    .await
    .unwrap();

    // The archive itself holds none of it in the clear.
    let stored: String = sqlx::query_scalar("SELECT transcript FROM transcripts WHERE id = 't1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!stored.contains("переезжаем"));

    let collected = collect_meeting(&pool, "m1").await.unwrap();
    let target = tempfile::tempdir().unwrap();
    let mut export = PlainExport::create(target.path(), "Без клиента").unwrap();
    let (entry, bytes) = export.write_meeting(&collected, None).unwrap();
    let folder = export.root().join(&entry.folder);

    assert_eq!(bytes, audio.len() as u64);
    assert_eq!(std::fs::read(folder.join("audio.mp4")).unwrap(), audio);
    let transcript = std::fs::read_to_string(folder.join("transcript.md")).unwrap();
    assert!(transcript.contains("# Разговор о переезде"));
    assert!(transcript.contains("**[00:01] Борис:** Мы переезжаем в марте"));
    let record: MeetingRecord =
        serde_json::from_slice(&std::fs::read(folder.join(MEETING_NAME)).unwrap()).unwrap();
    assert_eq!(record.lines[0].text, "Мы переезжаем в марте");
    assert!(entry.folder.ends_with("Разговор о переезде"));
}
