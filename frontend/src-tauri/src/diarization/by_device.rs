//! Who spoke, for a recording made with separate tracks: by device.
//!
//! The microphone is the owner and the speakers are the other side. That is
//! not a guess to be refined by a model — it is how the recording was made —
//! and it holds for every line, however long, however much the two talked
//! over each other. The model is for recordings that do not have it: an
//! imported file, a dictaphone, several people around one microphone.
//!
//! Each line remembers the track it was recognised from. Lines saved before
//! that was remembered are placed by which track was louder over their span,
//! once, and then remember it too — so an old recording is fixed without
//! being recognised again.

use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

use super::MeetingDiarizationResult;
use crate::audio::echo_offline::{Envelope, WINDOW_MS};
use crate::database::fields;

pub const MIC: &str = "mic";
pub const SYSTEM: &str = "system";

/// The label a remote voice gets when nobody named it.
const REMOTE_LABEL: &str = "Speaker 1";
/// The label the owner's microphone gets when nobody named it.
const LOCAL_LABEL: &str = "You";

/// Whether a label is one the app made up rather than a name someone chose.
fn is_generated(label: &str) -> bool {
    let trimmed = label.trim();
    trimmed.is_empty()
        || trimmed.contains(" + ")
        || trimmed.eq_ignore_ascii_case("you")
        || trimmed.eq_ignore_ascii_case("guest")
        || trimmed.eq_ignore_ascii_case("гость")
        || trimmed
            .to_lowercase()
            .strip_prefix("speaker ")
            .is_some_and(|rest| rest.trim().chars().all(|c| c.is_ascii_digit()))
}

/// The track a line came from, judged by the label it was saved with.
///
/// Fresh from recognition every line is labelled by its source — "You" for the
/// microphone, "Guest" or "Speaker N" for the speakers — so at the moment of
/// saving this is exact. A line two voices were merged into ("You + Speaker 1")
/// says nothing, and neither does a name someone typed.
pub fn source_of_label(label: Option<&str>) -> Option<&'static str> {
    let label = label?.trim();
    if label.contains(" + ") {
        return None;
    }
    if label.eq_ignore_ascii_case(LOCAL_LABEL) {
        return Some(MIC);
    }
    if is_generated(label) {
        return Some(SYSTEM);
    }
    None
}

/// Which track was louder over a stretch of the recording.
///
/// `None` when neither carried anything there. With headphones the other
/// track is near silence, and the answer is not close; a speakerphone
/// recording is closer, and still right far more often than not.
pub fn louder_track(
    start_s: f64,
    end_s: f64,
    mic: &Envelope,
    system: &Envelope,
) -> Option<&'static str> {
    let level = |envelope: &Envelope| -> f32 {
        let from = ((start_s * 1000.0) / envelope.window_ms).floor().max(0.0) as usize;
        let to = (((end_s * 1000.0) / envelope.window_ms).ceil() as usize).min(envelope.rms.len());
        if from >= to {
            return 0.0;
        }
        let slice = &envelope.rms[from..to];
        slice.iter().sum::<f32>() / slice.len() as f32
    };
    let (heard, played) = (level(mic), level(system));
    if heard < 0.002 && played < 0.002 {
        return None;
    }
    Some(if heard >= played { MIC } else { SYSTEM })
}

/// The name each side goes by in this meeting, when someone gave it one.
///
/// Renaming "Speaker 1" to a person's name is exactly the work that must
/// survive a relabelling. When one side has been given exactly one name, every
/// line of that side takes it; with none, or with several, the generated label
/// is used and nothing anyone chose is overwritten.
fn chosen_names(rows: &[(Option<String>, Option<&'static str>)]) -> (Option<String>, Option<String>) {
    let mut local: Vec<String> = Vec::new();
    let mut remote: Vec<String> = Vec::new();
    for (label, source) in rows {
        let (Some(label), Some(source)) = (label, source) else {
            continue;
        };
        if is_generated(label) {
            continue;
        }
        let bucket = if *source == MIC { &mut local } else { &mut remote };
        if !bucket.iter().any(|kept| kept == label) {
            bucket.push(label.clone());
        }
    }
    let single = |names: Vec<String>| if names.len() == 1 { names.into_iter().next() } else { None };
    (single(local), single(remote))
}

/// The label a line gets on its side.
fn label_for(
    current: Option<&str>,
    source: &str,
    local_name: Option<&str>,
    remote_name: Option<&str>,
) -> String {
    // A name typed on this very line stays exactly as it is.
    if let Some(current) = current {
        if !is_generated(current) {
            return current.to_string();
        }
    }
    if source == MIC {
        local_name.unwrap_or(LOCAL_LABEL).to_string()
    } else {
        remote_name.unwrap_or(REMOTE_LABEL).to_string()
    }
}

/// The two tracks of a recording folder, if it has both.
pub fn device_tracks(folder: &Path) -> Option<(PathBuf, PathBuf)> {
    if let (Some(mic), Some(system)) = (
        crate::audio::find_working_track(folder, "mic"),
        crate::audio::find_working_track(folder, "system"),
    ) {
        return Some((mic, system));
    }
    let (mic, system) = (folder.join("mic.mp4"), folder.join("system.mp4"));
    (mic.is_file() && system.is_file()).then_some((mic, system))
}

fn envelope_of(path: &Path) -> Result<Envelope, String> {
    let decoded = crate::audio::decoder::decode_audio_file(path)
        .map_err(|error| format!("Could not decode {}: {error}", path.display()))?;
    let samples = decoded.to_whisper_format();
    Ok(Envelope::measure(&samples, 16_000, WINDOW_MS))
}

/// Label a meeting's lines by the device they were heard on.
pub async fn assign(
    pool: &SqlitePool,
    meeting_id: &str,
    folder: &Path,
) -> Result<MeetingDiarizationResult, String> {
    type Row = (String, Option<String>, Option<String>, Option<f64>, Option<f64>);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, speaker, source_track, audio_start_time, audio_end_time
         FROM transcripts WHERE meeting_id = ? ORDER BY audio_start_time",
    )
    .bind(meeting_id)
    .fetch_all(pool)
    .await
    .map_err(|error| format!("Failed to read the transcript: {error}"))?;

    let mut lines: Vec<(String, Option<String>, Option<&'static str>, Option<f64>, Option<f64>)> =
        Vec::with_capacity(rows.len());
    for (id, sealed_speaker, stored_source, start, end) in rows {
        let speaker = sealed_speaker
            .map(|value| fields::open(fields::TRANSCRIPT_SPEAKER, &value))
            .transpose()
            .map_err(|error| format!("Failed to read a speaker label: {error}"))?;
        let source = match stored_source.as_deref() {
            Some(MIC) => Some(MIC),
            Some(SYSTEM) => Some(SYSTEM),
            _ => None,
        };
        lines.push((id, speaker, source, start, end));
    }

    // A line saved before the track was remembered is placed by which track
    // was louder over it — not by its label. Its label may be the model's,
    // and the model is exactly what put "Speaker 1" on the owner's words. The
    // tracks are decoded only if such a line exists.
    let tracks = device_tracks(folder);
    if lines.iter().any(|line| line.2.is_none()) {
        if let Some((mic_path, system_path)) = tracks {
            let envelopes = tokio::task::spawn_blocking(move || -> Result<(Envelope, Envelope), String> {
                Ok((envelope_of(&mic_path)?, envelope_of(&system_path)?))
            })
            .await
            .map_err(|error| format!("The measuring task failed: {error}"))??;
            let (mic, system) = envelopes;
            let mut placed = 0usize;
            for line in lines.iter_mut().filter(|line| line.2.is_none()) {
                if let (Some(start), Some(end)) = (line.3, line.4) {
                    line.2 = louder_track(start, end, &mic, &system);
                    placed += usize::from(line.2.is_some());
                }
            }
            log::info!("🎚️ Placed {placed} lines on a track by which one was louder");
        }
    }

    // With no tracks to measure, the label is all there is.
    for line in lines.iter_mut().filter(|line| line.2.is_none()) {
        line.2 = source_of_label(line.1.as_deref());
    }

    let chosen: Vec<(Option<String>, Option<&'static str>)> =
        lines.iter().map(|line| (line.1.clone(), line.2)).collect();
    let (local_name, remote_name) = chosen_names(&chosen);

    let mut transaction = pool
        .begin()
        .await
        .map_err(|error| format!("Failed to start a transaction: {error}"))?;
    let mut assignments: Vec<(String, String)> = Vec::new();
    let mut unplaced = 0usize;
    for (id, speaker, source, _, _) in &lines {
        let Some(source) = source else {
            unplaced += 1;
            continue;
        };
        let label = label_for(speaker.as_deref(), source, local_name.as_deref(), remote_name.as_deref());
        // A label a person chose stays; where the line was heard is still recorded.
        sqlx::query(
            "UPDATE transcripts SET speaker = CASE WHEN speaker_set_at IS NULL THEN ? ELSE speaker END, \
             source_track = ? WHERE id = ?",
        )
            .bind(
                fields::seal_joinable(fields::TRANSCRIPT_SPEAKER, &label)
                    .map_err(|error| format!("Failed to seal a speaker label: {error}"))?,
            )
            .bind(*source)
            .bind(id)
            .execute(&mut *transaction)
            .await
            .map_err(|error| format!("Failed to label a line: {error}"))?;
        assignments.push((id.clone(), label));
    }
    transaction
        .commit()
        .await
        .map_err(|error| format!("Failed to save the labels: {error}"))?;

    let mut distinct: Vec<&String> = assignments.iter().map(|(_, label)| label).collect();
    distinct.sort();
    distinct.dedup();
    log::info!(
        "🎙️ Labelled {} lines by device ({} voices){}",
        assignments.len(),
        distinct.len(),
        if unplaced > 0 {
            format!(", {unplaced} left as they were: no track to place them on")
        } else {
            String::new()
        }
    );

    Ok(MeetingDiarizationResult {
        num_speakers: distinct.len(),
        labeled: assignments.len(),
        assignments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_line_tells_its_track_by_its_label() {
        assert_eq!(source_of_label(Some("You")), Some(MIC));
        assert_eq!(source_of_label(Some("Guest")), Some(SYSTEM));
        assert_eq!(source_of_label(Some("Speaker 2")), Some(SYSTEM));
        // Merged or named, a label says nothing about where it was heard.
        assert_eq!(source_of_label(Some("You + Speaker 1")), None);
        assert_eq!(source_of_label(Some("Анна")), None);
        assert_eq!(source_of_label(None), None);
    }

    #[test]
    fn a_line_goes_to_the_louder_track() {
        // Four seconds; the microphone speaks in the first two, the speakers
        // in the last two — headphones, so each is near silence in the other.
        let mut mic = vec![0.0005f32; 160];
        let mut system = vec![0.0005f32; 160];
        mic[..80].iter_mut().for_each(|level| *level = 0.08);
        system[80..].iter_mut().for_each(|level| *level = 0.08);
        let (mic, system) = (
            Envelope { window_ms: WINDOW_MS, rms: mic },
            Envelope { window_ms: WINDOW_MS, rms: system },
        );

        assert_eq!(louder_track(0.2, 1.8, &mic, &system), Some(MIC));
        assert_eq!(louder_track(2.2, 3.8, &mic, &system), Some(SYSTEM));
    }

    #[test]
    fn silence_on_both_tracks_places_nothing() {
        let quiet = Envelope { window_ms: WINDOW_MS, rms: vec![0.0; 80] };
        assert_eq!(louder_track(0.0, 1.0, &quiet, &quiet), None);
    }

    #[test]
    fn a_name_someone_gave_a_side_is_kept_for_the_whole_side() {
        let rows = vec![
            (Some("Анна".to_string()), Some(SYSTEM)),
            (Some("You + Speaker 1".to_string()), Some(SYSTEM)),
            (Some("You".to_string()), Some(MIC)),
        ];
        let (local, remote) = chosen_names(&rows);

        assert_eq!(local, None);
        assert_eq!(remote.as_deref(), Some("Анна"));
        assert_eq!(
            label_for(Some("You + Speaker 1"), SYSTEM, local.as_deref(), remote.as_deref()),
            "Анна"
        );
        assert_eq!(label_for(Some("You + Speaker 1"), MIC, None, None), "You");
    }

    #[test]
    fn a_side_with_two_names_is_not_forced_into_one() {
        let rows = vec![
            (Some("Анна".to_string()), Some(SYSTEM)),
            (Some("Пётр".to_string()), Some(SYSTEM)),
        ];
        let (_, remote) = chosen_names(&rows);

        assert_eq!(remote, None);
        // Each named line keeps its own name.
        assert_eq!(label_for(Some("Пётр"), SYSTEM, None, None), "Пётр");
    }
}

#[cfg(test)]
mod against_the_schema {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    /// The real migrations, so the column this relies on is the one that ships.
    async fn migrated() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::manager::MIGRATOR.run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) \
             VALUES ('m1', 'встреча', '2026-09-21T10:00:00Z', '2026-09-21T10:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    async fn line(pool: &SqlitePool, id: &str, speaker: &str, track: Option<&str>, at: f64) {
        sqlx::query(
            "INSERT INTO transcripts \
             (id, meeting_id, transcript, timestamp, speaker, source_track, audio_start_time, audio_end_time) \
             VALUES (?, 'm1', 'текст', '2026-09-21T10:00:00Z', ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(speaker)
        .bind(track)
        .bind(at)
        .bind(at + 1.0)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn labels(pool: &SqlitePool) -> Vec<(String, String, Option<String>)> {
        sqlx::query_as("SELECT id, speaker, source_track FROM transcripts ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_meeting_is_labelled_by_the_device_each_line_was_heard_on() {
        let pool = migrated().await;
        line(&pool, "t1", "You", None, 0.0).await;
        line(&pool, "t2", "Guest", None, 1.0).await;
        // What the model left behind: a merged label, on a line that is known
        // to have come from the speakers.
        line(&pool, "t3", "You + Speaker 1", Some("system"), 2.0).await;
        // A name the owner gave the other side.
        line(&pool, "t4", "Анна", Some("system"), 3.0).await;
        let folder = tempfile::tempdir().unwrap();

        let result = assign(&pool, "m1", folder.path()).await.unwrap();

        assert_eq!(result.labeled, 4);
        assert_eq!(result.num_speakers, 2);
        assert_eq!(
            labels(&pool).await,
            vec![
                ("t1".into(), "You".into(), Some("mic".into())),
                ("t2".into(), "Анна".into(), Some("system".into())),
                ("t3".into(), "Анна".into(), Some("system".into())),
                ("t4".into(), "Анна".into(), Some("system".into())),
            ]
        );
    }

    #[tokio::test]
    async fn a_line_with_no_track_to_place_it_on_is_left_as_it_was() {
        let pool = migrated().await;
        line(&pool, "t1", "You + Speaker 1", None, 0.0).await;
        // No tracks in the folder: nothing to measure the old line against.
        let folder = tempfile::tempdir().unwrap();

        let result = assign(&pool, "m1", folder.path()).await.unwrap();

        assert_eq!(result.labeled, 0);
        assert_eq!(
            labels(&pool).await,
            vec![("t1".into(), "You + Speaker 1".into(), None)]
        );
    }
}
