//! The readable export: ordinary folders and files, nothing encrypted.
//!
//! ```text
//! <target>/Talkkeeper 2026-09-26 15-30/
//!     library.json
//!     <client>/<2026-09-12 10-00 title>/
//!         audio.mp4, mic.mp4, system.mp4   as recorded, decrypted
//!         transcript.md
//!         summary.md                        when there is one
//!         meeting.json                      everything, for an import
//! ```

use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};

use super::{clock, safe_name, CollectedMeeting, Manifest, MeetingEntry, MeetingRecord, MANIFEST_NAME, MEETING_NAME};
use crate::audio::encrypted_audio::AudioSource;

/// Where the export goes, and the names already taken inside it.
pub struct PlainExport {
    root: PathBuf,
    taken: HashSet<String>,
    unassigned: String,
}

impl PlainExport {
    /// A fresh folder under `target`, named for the moment of the export.
    pub fn create(target: &Path, unassigned: &str) -> io::Result<Self> {
        let stamp = Local::now().format("%Y-%m-%d %H-%M").to_string();
        let mut root = target.join(format!("Talkkeeper {stamp}"));
        let mut attempt = 2;
        while root.exists() {
            root = target.join(format!("Talkkeeper {stamp} ({attempt})"));
            attempt += 1;
        }
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            taken: HashSet::new(),
            unassigned: safe_name(unassigned, "Unassigned"),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Writes one recording and returns its entry for the manifest and how
    /// many bytes of audio went out.
    pub fn write_meeting(
        &mut self,
        meeting: &CollectedMeeting,
        filed_under: Option<&str>,
    ) -> io::Result<(MeetingEntry, u64)> {
        let record = &meeting.record;
        let parent = filed_under
            .map(|name| safe_name(name, &self.unassigned))
            .unwrap_or_else(|| self.unassigned.clone());
        let when = local_time(&record.created_at)
            .map(|time| time.format("%Y-%m-%d %H-%M").to_string())
            .unwrap_or_default();
        let base = safe_name(&format!("{when} {}", record.title), &record.id);

        let mut relative = format!("{parent}/{base}");
        let mut attempt = 2;
        while !self.taken.insert(relative.to_lowercase()) {
            relative = format!("{parent}/{base} ({attempt})");
            attempt += 1;
        }
        let folder = self.root.join(&relative);
        fs::create_dir_all(&folder)?;

        let mut bytes = 0;
        for (name, source) in &meeting.audio {
            bytes += copy_audio(source, &folder.join(name))?;
        }
        write_atomically(&folder.join("transcript.md"), transcript_markdown(record, filed_under).as_bytes())?;
        if let Some(summary) = record.summary_markdown() {
            write_atomically(&folder.join("summary.md"), summary.as_bytes())?;
        }
        if let Some(notes) = notes_markdown(record) {
            write_atomically(&folder.join("notes.md"), notes.as_bytes())?;
        }
        let json = serde_json::to_vec_pretty(record).map_err(io::Error::other)?;
        write_atomically(&folder.join(MEETING_NAME), &json)?;

        Ok((MeetingEntry { id: record.id.clone(), folder: relative }, bytes))
    }

    pub fn write_manifest(&self, manifest: &Manifest) -> io::Result<()> {
        let json = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
        write_atomically(&self.root.join(MANIFEST_NAME), &json)
    }
}

fn local_time(stored: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(stored)
        .ok()
        .map(|time| time.with_timezone(&Local))
}

/// The recording's audio as it was captured, decrypted on the way.
fn copy_audio(source: &Path, target: &Path) -> io::Result<u64> {
    let mut reader = AudioSource::open(source).map_err(io::Error::other)?;
    let partial = with_suffix(target, ".part");
    let mut out = io::BufWriter::new(fs::File::create(&partial)?);
    let copied = io::copy(&mut reader, &mut out)?;
    out.flush()?;
    drop(out);
    fs::rename(&partial, target)?;
    Ok(copied)
}

/// A file that is either there whole or not there: an export cut short leaves
/// only `.part` names behind, never a truncated file under the real one.
fn write_atomically(target: &Path, bytes: &[u8]) -> io::Result<()> {
    let partial = with_suffix(target, ".part");
    fs::write(&partial, bytes)?;
    fs::rename(&partial, target)
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// The owner's notes as a person reads them, each with its moment in the
/// recording when it has one.
pub fn notes_markdown(record: &MeetingRecord) -> Option<String> {
    if record.notes.is_empty() {
        return None;
    }
    let mut out = format!("# {}\n\n", record.title.trim());
    for note in &record.notes {
        match note.at {
            Some(at) => out.push_str(&format!("**[{}]** {}\n\n", clock(at), note.text)),
            None => out.push_str(&format!("{}\n\n", note.text)),
        }
    }
    Some(out)
}

/// The transcript as a person reads it: who spoke, when, what.
pub fn transcript_markdown(record: &MeetingRecord, filed_under: Option<&str>) -> String {
    let mut out = format!("# {}\n\n", record.title.trim());
    let when = local_time(&record.created_at)
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| record.created_at.clone());
    match filed_under {
        Some(client) => out.push_str(&format!("{when} · {client}\n\n")),
        None => out.push_str(&format!("{when}\n\n")),
    }
    for line in &record.lines {
        let text = line.text.trim();
        if text.is_empty() {
            continue;
        }
        let at = line.start.map(|start| format!("[{}] ", clock(start))).unwrap_or_default();
        match line.speaker.as_deref().map(str::trim).filter(|speaker| !speaker.is_empty()) {
            Some(speaker) => out.push_str(&format!("**{at}{speaker}:** {text}\n\n")),
            None => out.push_str(&format!("{at}{text}\n\n")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_export::LineRecord;

    fn line(start: Option<f64>, speaker: Option<&str>, text: &str) -> LineRecord {
        LineRecord {
            id: "l".into(),
            text: text.into(),
            timestamp: String::new(),
            start,
            end: None,
            duration: None,
            speaker: speaker.map(str::to_string),
            source_track: None,
            words: None,
            edited_at: None,
            speaker_set_at: None,
        }
    }

    fn note(id: &str, at: Option<f64>, text: &str) -> crate::meeting_notes::Note {
        crate::meeting_notes::Note {
            id: id.into(),
            at,
            text: text.into(),
            written_at: "2026-09-12T11:00:00+00:00".into(),
        }
    }

    fn record() -> MeetingRecord {
        MeetingRecord {
            id: "m1".into(),
            title: "План: неделя".into(),
            created_at: "2026-09-12T10:00:00+00:00".into(),
            updated_at: "2026-09-12T11:00:00+00:00".into(),
            client_id: None,
            lines: vec![
                line(Some(3.0), Some("You"), "Добрый день."),
                line(Some(75.0), None, "  "),
                line(Some(83.0), Some("Speaker 1"), "Здравствуйте."),
                line(None, None, "Без времени."),
            ],
            summary: Some(r#"{"markdown":"Итоги"}"#.into()),
            speakers: Vec::new(),
            tracks: vec!["audio.mp4".into()],
            notes: vec![note("n1", Some(83.0), "по ходу"), note("n2", None, "после")],
        }
    }

    #[test]
    fn the_transcript_reads_as_who_said_what_and_when() {
        let text = transcript_markdown(&record(), Some("Клиент"));
        assert!(text.starts_with("# План: неделя\n\n"));
        assert!(text.contains(" · Клиент\n\n"));
        assert!(text.contains("**[00:03] You:** Добрый день.\n\n"));
        assert!(text.contains("**[01:23] Speaker 1:** Здравствуйте.\n\n"));
        assert!(text.contains("\nБез времени.\n"));
        // An empty line is not written as an empty entry.
        assert_eq!(text.matches("[01:15]").count(), 0);
    }

    #[test]
    fn a_recording_is_written_with_its_audio_decrypted_and_its_text() {
        let target = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        // A plaintext track: the export copies what AudioSource reads.
        std::fs::write(source.path().join("audio.mp4"), b"audio bytes").unwrap();

        let mut export = PlainExport::create(target.path(), "Без клиента").unwrap();
        let meeting = CollectedMeeting {
            record: record(),
            audio: vec![("audio.mp4".into(), source.path().join("audio.mp4"))],
        };
        let (entry, bytes) = export.write_meeting(&meeting, None).unwrap();
        // The same title twice gets a second folder, not an overwrite.
        let (second, _) = export.write_meeting(&meeting, None).unwrap();

        assert_eq!(bytes, 11);
        assert!(entry.folder.starts_with("Без клиента/"));
        assert!(entry.folder.ends_with("План неделя"));
        assert_eq!(second.folder, format!("{} (2)", entry.folder));
        let folder = export.root().join(&entry.folder);
        assert_eq!(std::fs::read(folder.join("audio.mp4")).unwrap(), b"audio bytes");
        assert_eq!(std::fs::read_to_string(folder.join("summary.md")).unwrap(), "Итоги");
        let notes = std::fs::read_to_string(folder.join("notes.md")).unwrap();
        assert!(notes.contains("**[01:23]** по ходу\n\n"));
        assert!(notes.contains("\nпосле\n"));
        let reread: MeetingRecord =
            serde_json::from_slice(&std::fs::read(folder.join(MEETING_NAME)).unwrap()).unwrap();
        assert_eq!(reread, record());
        // Nothing is left under a temporary name.
        let leftovers = std::fs::read_dir(&folder)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".part"))
            .count();
        assert_eq!(leftovers, 0);
    }
}
