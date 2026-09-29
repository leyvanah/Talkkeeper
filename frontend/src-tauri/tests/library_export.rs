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

#[tokio::test]
async fn a_package_carries_a_recording_into_another_library_whole() {
    use app_lib::audio::encrypted_audio::AudioSource;
    use app_lib::library_export::import::{import_meeting, match_clients, Outcome};
    use app_lib::library_export::sealed::{SealedExport, SealedPackage};
    use app_lib::library_export::{clients, Manifest, FORMAT_VERSION};
    use std::io::Read;

    open_the_archive();
    let source = archive().await;

    let recording = tempfile::tempdir().unwrap();
    let audio: Vec<u8> = (0..150_000u32).map(|i| (i % 241) as u8).collect();
    let file = std::fs::File::create(recording.path().join("mic.mp4")).unwrap();
    session::with_current_key(|key| {
        let mut writer = EncryptedWriter::create(file, key).unwrap();
        writer.write_all(&audio).unwrap();
        writer.finish().unwrap();
    })
    .unwrap();

    let (client_id, _) = app_lib::database::repositories::client::ClientsRepository::find_or_create(&source, "Вера")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO meetings (id, title, created_at, updated_at, folder_path, client_id) \
         VALUES ('m-transfer', ?, '2026-09-12T10:00:00Z', '2026-09-12T11:00:00Z', ?, ?)",
    )
    .bind(fields::seal(fields::MEETING_TITLE, "Вторая встреча").unwrap())
    .bind(recording.path().to_string_lossy().to_string())
    .bind(&client_id)
    .execute(&source)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transcripts (id, meeting_id, transcript, timestamp, speaker, source_track, audio_start_time, audio_end_time) \
         VALUES ('t-transfer', 'm-transfer', ?, '10:00:01', ?, 'system', 1.0, 3.0)",
    )
    .bind(fields::seal(fields::TRANSCRIPT_TEXT, "Про работу и отпуск").unwrap())
    .bind(fields::seal_joinable(fields::TRANSCRIPT_SPEAKER, "Вера").unwrap())
    .execute(&source)
    .await
    .unwrap();
    sqlx::query("INSERT INTO meeting_speaker_roles (meeting_id, speaker_label, role) VALUES ('m-transfer', ?, 'client')")
        .bind(fields::seal_joinable(fields::SPEAKER_LABEL, "Вера").unwrap())
        .execute(&source)
        .await
        .unwrap();

    let notes = vec![app_lib::meeting_notes::Note::new("вернуться к отпуску", Some(2.0)).unwrap()];
    let mut conn = source.acquire().await.unwrap();
    app_lib::meeting_notes::store(&mut conn, "m-transfer", &notes).await.unwrap();
    drop(conn);

    // Export from the first library.
    let collected = collect_meeting(&source, "m-transfer").await.unwrap();
    assert_eq!(collected.record.notes, notes);
    let target = tempfile::tempdir().unwrap();
    let mut export = SealedExport::create(target.path(), "перенос-2026").unwrap();
    let (entry, _) = export.write_meeting(&collected).unwrap();
    export
        .write_manifest(&Manifest {
            format: FORMAT_VERSION,
            app_version: "test".into(),
            exported_at: String::new(),
            clients: clients(&source).await.unwrap(),
            meetings: vec![entry.clone()],
        })
        .unwrap();

    // Import into a second, empty library.
    let destination = archive().await;
    let recordings = tempfile::tempdir().unwrap();
    let (package, manifest) = SealedPackage::open(export.root(), "перенос-2026").unwrap();
    let package = Arc::new(package);
    let matched = match_clients(&destination, &manifest.clients).await.unwrap();
    let outcome = import_meeting(&destination, package.clone(), &entry, &matched, recordings.path())
        .await
        .unwrap();
    assert_eq!(outcome, Outcome::Imported);
    // A second import of the same package leaves the library as it is.
    let again = import_meeting(&destination, package, &entry, &matched, recordings.path())
        .await
        .unwrap();
    assert_eq!(again, Outcome::AlreadyPresent);
    let folders = std::fs::read_dir(recordings.path()).unwrap().count();
    assert_eq!(folders, 1);

    // The recording reads the same, and its text is sealed in the new library.
    let imported = collect_meeting(&destination, "m-transfer").await.unwrap();
    assert_eq!(imported.record.title, "Вторая встреча");
    assert_eq!(imported.record.lines, collected.record.lines);
    assert_eq!(imported.record.speakers, collected.record.speakers);
    assert_eq!(imported.record.notes, notes);
    let sealed_notes: String = sqlx::query_scalar("SELECT notes_json FROM meeting_notes WHERE meeting_id = 'm-transfer'")
        .fetch_one(&destination)
        .await
        .unwrap();
    assert!(!sealed_notes.contains("отпуск"));
    assert_eq!(
        clients(&destination).await.unwrap()[0].name,
        "Вера"
    );
    let stored: String = sqlx::query_scalar("SELECT transcript FROM transcripts WHERE id = 't-transfer'")
        .fetch_one(&destination)
        .await
        .unwrap();
    assert!(!stored.contains("отпуск"));

    let (name, path) = &imported.audio[0];
    assert_eq!(name, "mic.mp4");
    assert!(file_looks_encrypted(path));
    let mut read = Vec::new();
    AudioSource::open(path).unwrap().read_to_end(&mut read).unwrap();
    assert_eq!(read, audio);
}
