//! Names a person gave speakers, carried across a retranscription.
//!
//! A rename ("Speaker 2" → a person's name) is written onto the lines
//! themselves, and a retranscription replaces those lines. Before they go,
//! the named ones are read with where they sat in the recording; each new line
//! that falls mostly inside one name's stretch takes that name back. The
//! labelling that follows — by the model or by the device — keeps a name it
//! finds, so the name outlives the pass instead of turning into "Speaker N".

use sqlx::{Row, Sqlite, Transaction};

use super::by_device::{is_generated, MIC, SYSTEM};
use crate::database::fields;

/// How much of a new line one name has to cover before the line takes it.
const MIN_SHARE: f64 = 0.5;

/// A stretch of the recording an old line carried a chosen name over.
#[derive(Debug, Clone, PartialEq)]
pub struct NamedSpan {
    pub name: String,
    pub start: f64,
    pub end: f64,
    /// The track the old line was heard on, when that was known.
    pub track: Option<&'static str>,
}

/// The named stretches among the lines a retranscription is about to replace.
///
/// Only lines nobody edited: an edited line stays where it is, name and all.
pub async fn named_spans(
    tx: &mut Transaction<'_, Sqlite>,
    meeting_id: &str,
) -> Result<Vec<NamedSpan>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT speaker, source_track, audio_start_time, audio_end_time FROM transcripts \
         WHERE meeting_id = ? AND edited_at IS NULL AND speaker IS NOT NULL",
    )
    .bind(meeting_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut spans = Vec::new();
    for row in rows {
        let Some(name) = fields::open_opt(fields::TRANSCRIPT_SPEAKER, row.try_get("speaker")?)? else {
            continue;
        };
        let name = name.trim();
        if is_generated(name) {
            continue;
        }
        let (Some(start), Some(end)) = (
            row.try_get::<Option<f64>, _>("audio_start_time")?,
            row.try_get::<Option<f64>, _>("audio_end_time")?,
        ) else {
            continue;
        };
        let track = match row.try_get::<Option<String>, _>("source_track")?.as_deref() {
            Some(MIC) => Some(MIC),
            Some(SYSTEM) => Some(SYSTEM),
            _ => None,
        };
        spans.push(NamedSpan { name: name.to_string(), start, end, track });
    }
    Ok(spans)
}

/// The name a new line takes back, if one name covers most of it.
///
/// Only stretches from the same track count when both tracks are known; a
/// one-track recording matches anything. A line of no length takes the name
/// of the stretch its moment falls in.
pub fn name_for(start: f64, end: f64, track: Option<&str>, spans: &[NamedSpan]) -> Option<String> {
    let compatible = |span: &&NamedSpan| match (track, span.track) {
        (Some(line), Some(old)) => line == old,
        _ => true,
    };

    if end <= start {
        return spans
            .iter()
            .filter(compatible)
            .find(|span| span.start <= start && start <= span.end)
            .map(|span| span.name.clone());
    }

    let mut covered: Vec<(&str, f64)> = Vec::new();
    for span in spans.iter().filter(compatible) {
        let overlap = end.min(span.end) - start.max(span.start);
        if overlap <= 0.0 {
            continue;
        }
        match covered.iter_mut().find(|(name, _)| *name == span.name) {
            Some((_, total)) => *total += overlap,
            None => covered.push((&span.name, overlap)),
        }
    }
    let (name, overlap) = covered
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    (overlap >= (end - start) * MIN_SHARE).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(name: &str, start: f64, end: f64, track: Option<&'static str>) -> NamedSpan {
        NamedSpan { name: name.to_string(), start, end, track }
    }

    #[test]
    fn a_line_inside_a_named_stretch_takes_the_name() {
        let spans = [span("Anna", 10.0, 20.0, None)];
        assert_eq!(name_for(12.0, 18.0, None, &spans).as_deref(), Some("Anna"));
    }

    #[test]
    fn a_line_mostly_outside_stays_unnamed() {
        let spans = [span("Anna", 10.0, 20.0, None)];
        assert_eq!(name_for(18.0, 26.0, None, &spans), None);
    }

    #[test]
    fn the_name_covering_more_of_the_line_wins() {
        let spans = [span("Anna", 0.0, 4.0, None), span("Boris", 4.0, 10.0, None)];
        assert_eq!(name_for(2.0, 10.0, None, &spans).as_deref(), Some("Boris"));
    }

    #[test]
    fn pieces_of_one_name_add_up() {
        // Two old lines of one person, split where the new line is not.
        let spans = [
            span("Anna", 0.0, 3.0, None),
            span("Anna", 3.2, 6.0, None),
            span("Boris", 6.0, 9.0, None),
        ];
        assert_eq!(name_for(0.0, 9.0, None, &spans).as_deref(), Some("Anna"));
    }

    #[test]
    fn a_name_from_the_other_track_does_not_count() {
        let spans = [span("Anna", 0.0, 10.0, Some(SYSTEM))];
        assert_eq!(name_for(1.0, 5.0, Some(MIC), &spans), None);
        assert_eq!(name_for(1.0, 5.0, Some(SYSTEM), &spans).as_deref(), Some("Anna"));
        // An old one-track line and a new two-track line still meet.
        let unknown = [span("Anna", 0.0, 10.0, None)];
        assert_eq!(name_for(1.0, 5.0, Some(MIC), &unknown).as_deref(), Some("Anna"));
    }

    #[tokio::test]
    async fn only_names_on_lines_about_to_be_replaced_are_read() {
        use sqlx::sqlite::SqlitePoolOptions;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::database::manager::MIGRATOR.run(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO meetings (id, title, created_at, updated_at) \
             VALUES ('m1', 'meeting', '2026-09-26T10:00:00Z', '2026-09-26T10:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, speaker, track, start, edited) in [
            ("a", Some("Anna"), Some("system"), 1.0, None),
            ("b", Some("Speaker 2"), None, 3.0, None),
            ("c", Some("You"), Some("mic"), 5.0, None),
            ("d", Some("Boris"), None, 7.0, Some("2026-09-26T11:00:00Z")),
            ("e", None, None, 9.0, None),
        ] {
            sqlx::query(
                "INSERT INTO transcripts \
                 (id, meeting_id, transcript, timestamp, speaker, source_track, audio_start_time, audio_end_time, edited_at) \
                 VALUES (?, 'm1', 'text', '2026-09-26T10:00:00Z', ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(speaker)
            .bind(track)
            .bind(start)
            .bind(start + 1.5)
            .bind(edited)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mut tx = pool.begin().await.unwrap();
        let spans = named_spans(&mut tx, "m1").await.unwrap();
        // Generated labels are not names, and an edited line keeps its own.
        assert_eq!(spans, vec![span("Anna", 1.0, 2.5, Some(SYSTEM))]);
    }

    #[test]
    fn a_line_of_no_length_takes_the_stretch_it_falls_in() {
        let spans = [span("Anna", 10.0, 20.0, None)];
        assert_eq!(name_for(15.0, 15.0, None, &spans).as_deref(), Some("Anna"));
        assert_eq!(name_for(25.0, 25.0, None, &spans), None);
    }
}
