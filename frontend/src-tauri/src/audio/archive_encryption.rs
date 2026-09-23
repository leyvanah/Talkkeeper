//! Converting recordings that are already on disk.
//!
//! Encryption is not only about what is recorded next. A password set today has
//! to cover the sessions recorded last month, or the lock screen is telling the
//! owner something that is not true. And a password *removed* has to give those
//! sessions back as playable files, or removing it destroys them — the one
//! outcome this whole phase exists to prevent.
//!
//! So there is exactly one operation here, run in one direction or the other
//! over every recording under the archive's folders.
//!
//! ## Why it is safe to run on the owner's real archive
//!
//! A recording is the only copy of something that cannot be recorded again, so
//! no file is ever written in place and none is replaced until its replacement
//! has been read back and checked:
//!
//! 1. the converted bytes go to `<name>.tkconv`;
//! 2. that file is read back through the reader that will read it in earnest,
//!    and its SHA-256 has to equal the source's;
//! 3. only then the original is moved aside to `<name>.tkold`, the replacement
//!    takes its place, and the original is deleted.
//!
//! Every interruption therefore leaves either the original or the verified
//! replacement in place, and [`recover_interrupted`] cleans up what a crash
//! left: a stray `.tkconv` is discarded, and a `.tkold` whose file is missing
//! is put back.
//!
//! ## What is converted
//!
//! The three delivery tracks, the two lossless working tracks, the detector
//! record that sits beside them, and anything imported into a meeting's
//! folder. The walk goes one level into each
//! recording folder plus its `.work/` — deliberately not a recursive sweep of
//! everything under the archive root, which is also where the models live.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::constants::AUDIO_EXTENSIONS;
use super::encrypted_audio::{AudioSink, AudioSource};
use super::own_speech_record;
use super::streaming_encoder::PARTIAL_EXT;
use crate::security::stream::file_looks_encrypted;

/// Suffix of a conversion in progress. Never read as audio.
const IN_PROGRESS_SUFFIX: &str = "tkconv";

/// Suffix of an original waiting to be deleted once its replacement is in place.
const SUPERSEDED_SUFFIX: &str = "tkold";

/// Which way to convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Plaintext recordings become encrypted ones.
    Encrypt,
    /// Encrypted recordings become plaintext ones, so they stay playable after
    /// the password is removed.
    Decrypt,
}

impl Direction {
    fn wants_conversion(self, encrypted: bool) -> bool {
        match self {
            Self::Encrypt => !encrypted,
            Self::Decrypt => encrypted,
        }
    }
}

/// What one pass did.
#[derive(Debug, Default, Clone)]
pub struct ConversionReport {
    /// Files converted by this pass.
    pub converted: usize,
    /// Files that were already the way they should be.
    pub untouched: usize,
    /// Files that could not be converted, with the reason. The originals are
    /// still in place.
    pub failed: Vec<(PathBuf, String)>,
}

impl ConversionReport {
    pub fn is_complete(&self) -> bool {
        self.failed.is_empty()
    }

    /// One line for the log, which is where the owner's support question starts.
    pub fn summary(&self, direction: Direction) -> String {
        format!(
            "{:?}: {} converted, {} already so, {} failed",
            direction,
            self.converted,
            self.untouched,
            self.failed.len()
        )
    }
}

/// Convert every recording under `roots`.
///
/// `progress` is called before each file with (done so far, total found), so a
/// caller can report an archive that takes a while without this module knowing
/// what a window is.
pub fn convert_all(
    roots: &[PathBuf],
    key: &[u8],
    direction: Direction,
    mut progress: impl FnMut(usize, usize),
) -> ConversionReport {
    let files = recordings_under(roots);
    let total = files.len();
    let mut report = ConversionReport::default();

    for (done, path) in files.into_iter().enumerate() {
        progress(done, total);
        if !direction.wants_conversion(file_looks_encrypted(&path)) {
            report.untouched += 1;
            continue;
        }
        match convert_one(&path, key, direction) {
            Ok(()) => {
                report.converted += 1;
                log::info!("Converted {}", path.display());
            }
            Err(error) => {
                log::error!("Could not convert {}: {error}", path.display());
                report.failed.push((path, error.to_string()));
            }
        }
    }
    progress(total, total);
    log::info!("Archive conversion — {}", report.summary(direction));
    report
}

/// How the recordings on disk are split between encrypted and not.
#[derive(Debug, Default, Clone, Copy)]
pub struct RecordingCounts {
    pub encrypted: usize,
    pub plaintext: usize,
}

/// Count the recordings, so the settings screen can say what is actually true
/// of the archive rather than what the password implies.
pub fn count_recordings(roots: &[PathBuf]) -> RecordingCounts {
    let mut counts = RecordingCounts::default();
    for path in recordings_under(roots) {
        if file_looks_encrypted(&path) {
            counts.encrypted += 1;
        } else {
            counts.plaintext += 1;
        }
    }
    counts
}

/// Put right whatever a crash left behind. Safe to call at any time, including
/// when no conversion was ever started.
pub fn recover_interrupted(roots: &[PathBuf]) {
    for folder in recording_folders(roots) {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.ends_with(&format!(".{IN_PROGRESS_SUFFIX}")) {
                // Unverified by definition: it was being written when the
                // process died.
                log::warn!("Discarding an unfinished conversion: {}", path.display());
                let _ = std::fs::remove_file(&path);
            } else if name.ends_with(&format!(".{SUPERSEDED_SUFFIX}")) {
                let original = with_suffix_removed(&path, SUPERSEDED_SUFFIX);
                if original.exists() {
                    // The replacement is in place; this is the original that
                    // was about to be deleted.
                    let _ = std::fs::remove_file(&path);
                } else {
                    log::warn!(
                        "Putting back a recording a conversion moved aside: {}",
                        original.display()
                    );
                    let _ = std::fs::rename(&path, &original);
                }
            }
        }
    }
}

/// Convert one file, leaving the original in place unless the replacement
/// verifies.
fn convert_one(path: &Path, key: &[u8], direction: Direction) -> anyhow::Result<()> {
    let replacement = with_suffix(path, IN_PROGRESS_SUFFIX);
    let _ = std::fs::remove_file(&replacement);

    let source_digest = write_converted(path, &replacement, key, direction)?;
    let written_digest = digest_of(&replacement, key)?;
    if source_digest != written_digest {
        let _ = std::fs::remove_file(&replacement);
        anyhow::bail!("the converted file does not hold the same audio");
    }
    put_in_place(path, &replacement)
}

/// Re-encrypt one file sealed with `from` so that it is sealed with `to`
/// instead, under the same rule as [`convert_one`]: the original stays until
/// the replacement has been read back with `to` and holds the same audio.
fn rekey_one(path: &Path, from: &[u8], to: &[u8]) -> anyhow::Result<()> {
    let replacement = with_suffix(path, IN_PROGRESS_SUFFIX);
    let _ = std::fs::remove_file(&replacement);

    let source_digest = {
        let mut source = AudioSource::open_with_key(path, from)?;
        let mut sink = AudioSink::create_with_key(&replacement, to)?;
        let digest = copy_hashing(&mut source, &mut sink)?;
        sink.finish()?;
        digest
    };
    let written_digest = digest_of(&replacement, to)?;
    if source_digest != written_digest {
        let _ = std::fs::remove_file(&replacement);
        anyhow::bail!("the re-encrypted file does not hold the same audio");
    }
    put_in_place(path, &replacement)
}

/// Brings one recording folder that was sealed with another archive's key
/// under this archive's: re-encrypted with `to`, or decrypted when this
/// archive has no password (`to` is `None`). Files that are not encrypted are
/// left as they are.
pub fn rekey_folder(folder: &Path, from: &[u8], to: Option<&[u8]>) -> ConversionReport {
    let mut files = Vec::new();
    collect_audio_in(folder, &mut files);
    collect_audio_in(&folder.join(".work"), &mut files);

    let mut report = ConversionReport::default();
    for path in files {
        if !file_looks_encrypted(&path) {
            report.untouched += 1;
            continue;
        }
        let result = match to {
            Some(to) => rekey_one(&path, from, to),
            None => convert_one(&path, from, Direction::Decrypt),
        };
        match result {
            Ok(()) => report.converted += 1,
            Err(error) => report.failed.push((path, error.to_string())),
        }
    }
    report
}

/// Swap a verified replacement in for the original.
fn put_in_place(path: &Path, replacement: &Path) -> anyhow::Result<()> {
    // From here on the original is expendable, but not before.
    let superseded = with_suffix(path, SUPERSEDED_SUFFIX);
    let _ = std::fs::remove_file(&superseded);
    std::fs::rename(path, &superseded)?;
    if let Err(error) = std::fs::rename(replacement, path) {
        // Put the original back rather than leaving a gap where a session was.
        let _ = std::fs::rename(&superseded, path);
        return Err(error.into());
    }
    let _ = std::fs::remove_file(&superseded);
    Ok(())
}

/// Copy `from` to `to`, encrypting or decrypting on the way, and return the
/// SHA-256 of the audio that went through.
fn write_converted(
    from: &Path,
    to: &Path,
    key: &[u8],
    direction: Direction,
) -> anyhow::Result<[u8; 32]> {
    use std::io::Write;

    let mut source = AudioSource::open_with_key(from, key)?;
    // Not written through one boxed writer: closing an encrypted stream means
    // sealing its last frame, which `flush` deliberately does not do, and a
    // `dyn Write` cannot be asked for anything else. Half a frame of a session
    // would go missing and the digest below would be the only thing to notice.
    match direction {
        Direction::Encrypt => {
            let mut sink = AudioSink::create_with_key(to, key)?;
            let digest = copy_hashing(&mut source, &mut sink)?;
            sink.finish()?;
            Ok(digest)
        }
        Direction::Decrypt => {
            let mut sink =
                std::io::BufWriter::with_capacity(1 << 16, std::fs::File::create(to)?);
            let digest = copy_hashing(&mut source, &mut sink)?;
            sink.flush()?;
            Ok(digest)
        }
    }
}

/// Copy everything, hashing it on the way through.
fn copy_hashing(
    source: &mut impl std::io::Read,
    sink: &mut impl std::io::Write,
) -> std::io::Result<[u8; 32]> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        sink.write_all(&buffer[..read])?;
    }
    Ok(hasher.finalize().into())
}

/// SHA-256 of the audio in a file, read the way the application will read it.
fn digest_of(path: &Path, key: &[u8]) -> anyhow::Result<[u8; 32]> {
    use std::io::Read;

    let mut source = AudioSource::open_with_key(path, key)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().into())
}

/// Every recording file under the archive's folders.
fn recordings_under(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for folder in recording_folders(roots) {
        collect_audio_in(&folder, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

/// The folders a recording's files live in: each meeting folder, and its
/// `.work/`. One level, not a sweep: the archive root also holds the models.
fn recording_folders(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let work = path.join(".work");
            folders.push(path);
            if work.is_dir() {
                folders.push(work);
            }
        }
    }
    folders.sort();
    folders.dedup();
    folders
}

fn collect_audio_in(folder: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        // A track still being recorded, or the remains of a conversion, is not
        // a recording to convert.
        if [PARTIAL_EXT, IN_PROGRESS_SUFFIX, SUPERSEDED_SUFFIX]
            .iter()
            .any(|suffix| name.ends_with(&format!(".{suffix}")))
        {
            continue;
        }
        let is_audio = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| {
                AUDIO_EXTENSIONS
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(extension))
            })
            .unwrap_or(false);
        // Not audio, but it says who was speaking when, and it has to follow
        // the password the same way: left behind, it would be a plaintext
        // sketch of a session whose audio is sealed — or an unreadable file
        // once the password is removed.
        let is_detector_record = name == own_speech_record::RECORD_NAME;
        if is_audio || is_detector_record {
            found.push(path);
        }
    }
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{suffix}"));
    PathBuf::from(name)
}

fn with_suffix_removed(path: &Path, suffix: &str) -> PathBuf {
    let text = path.as_os_str().to_string_lossy().into_owned();
    let marker = format!(".{suffix}");
    PathBuf::from(text.strip_suffix(&marker).unwrap_or(&text).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::envelope::generate_dek;
    use crate::security::stream::{EncryptedWriter, FRAME_LEN};
    use std::io::Write;

    fn session_audio(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 253) as u8).collect()
    }

    /// An archive as it is on disk: a meeting folder, its three tracks and the
    /// working tracks underneath.
    fn archive(dir: &Path, audio: &[u8]) -> (PathBuf, Vec<PathBuf>) {
        let meeting = dir.join("rec-0123456789abcdef0123456789abcdef");
        std::fs::create_dir_all(meeting.join(".work")).unwrap();
        let files = vec![
            meeting.join("audio.mp4"),
            meeting.join("mic.mp4"),
            meeting.join("system.mp4"),
            meeting.join(".work").join("mic.flac"),
        ];
        for path in &files {
            std::fs::write(path, audio).unwrap();
        }
        // Things that are not recordings and must be left alone.
        std::fs::write(meeting.join("metadata.json"), b"{}").unwrap();
        std::fs::write(meeting.join("audio.mp4.part"), b"half a track").unwrap();
        (meeting, files)
    }

    fn read_all(path: &Path, key: &[u8]) -> Vec<u8> {
        let mut back = Vec::new();
        std::io::Read::read_to_end(&mut AudioSource::open_with_key(path, key).unwrap(), &mut back)
            .unwrap();
        back
    }

    #[test]
    fn a_recording_from_another_archive_is_brought_under_this_key() {
        let dir = tempfile::tempdir().unwrap();
        let theirs = generate_dek();
        let ours = generate_dek();
        let audio = session_audio(2 * FRAME_LEN + 11);
        let (meeting, files) = archive(dir.path(), &audio);
        convert_all(&[dir.path().to_path_buf()], theirs.as_ref(), Direction::Encrypt, |_, _| {});

        let report = rekey_folder(&meeting, theirs.as_ref(), Some(ours.as_ref()));
        assert!(report.is_complete(), "{:?}", report.failed);
        assert_eq!(report.converted, 4);
        for path in &files {
            assert!(file_looks_encrypted(path));
            assert_eq!(read_all(path, ours.as_ref()), audio, "{}", path.display());
            assert!(AudioSource::open_with_key(path, theirs.as_ref()).is_err());
        }
        assert!(!meeting.join("audio.mp4.tkconv").exists());
        assert!(!meeting.join("audio.mp4.tkold").exists());
    }

    #[test]
    fn without_a_password_here_a_foreign_recording_is_decrypted() {
        let dir = tempfile::tempdir().unwrap();
        let theirs = generate_dek();
        let audio = session_audio(FRAME_LEN + 3);
        let (meeting, files) = archive(dir.path(), &audio);
        convert_all(&[dir.path().to_path_buf()], theirs.as_ref(), Direction::Encrypt, |_, _| {});

        let report = rekey_folder(&meeting, theirs.as_ref(), None);
        assert!(report.is_complete(), "{:?}", report.failed);
        for path in &files {
            assert!(!file_looks_encrypted(path));
            assert_eq!(std::fs::read(path).unwrap(), audio);
        }
    }

    #[test]
    fn the_wrong_key_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let theirs = generate_dek();
        let audio = session_audio(FRAME_LEN + 3);
        let (meeting, files) = archive(dir.path(), &audio);
        convert_all(&[dir.path().to_path_buf()], theirs.as_ref(), Direction::Encrypt, |_, _| {});
        let before: Vec<_> = files.iter().map(|path| std::fs::read(path).unwrap()).collect();

        let report = rekey_folder(&meeting, generate_dek().as_ref(), Some(generate_dek().as_ref()));
        assert_eq!(report.failed.len(), 4);
        let after: Vec<_> = files.iter().map(|path| std::fs::read(path).unwrap()).collect();
        assert_eq!(before, after);
        assert!(!meeting.join("audio.mp4.tkconv").exists());
    }

    #[test]
    fn setting_a_password_encrypts_what_is_already_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_dek();
        let audio = session_audio(2 * FRAME_LEN + 11);
        let (meeting, files) = archive(dir.path(), &audio);

        let report = convert_all(
            &[dir.path().to_path_buf()],
            key.as_ref(),
            Direction::Encrypt,
            |_, _| {},
        );
        assert!(report.is_complete(), "{:?}", report.failed);
        assert_eq!(report.converted, 4, "{}", report.summary(Direction::Encrypt));

        for path in &files {
            assert!(file_looks_encrypted(path), "{} was left in the clear", path.display());
            let mut back = Vec::new();
            std::io::Read::read_to_end(
                &mut AudioSource::open_with_key(path, key.as_ref()).unwrap(),
                &mut back,
            )
            .unwrap();
            assert_eq!(back, audio, "{} does not hold its audio", path.display());
        }

        // What was not a recording is untouched, including the half-written track.
        assert_eq!(std::fs::read(meeting.join("metadata.json")).unwrap(), b"{}");
        assert_eq!(
            std::fs::read(meeting.join("audio.mp4.part")).unwrap(),
            b"half a track"
        );
        // Nothing left over.
        assert!(!meeting.join("audio.mp4.tkconv").exists());
        assert!(!meeting.join("audio.mp4.tkold").exists());
    }

    /// The detector record is not audio, but it says who was speaking when,
    /// and it has to follow the password the same way the audio does.
    #[test]
    fn the_detector_record_is_converted_with_the_recordings() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_dek();
        let (meeting, _) = archive(dir.path(), &session_audio(FRAME_LEN + 3));

        let mut written = own_speech_record::GateRecord::new(50.0);
        for _ in 0..40 {
            written.observe(Some(false), Some(true));
        }
        let written = written.encode();
        let record = meeting
            .join(".work")
            .join(own_speech_record::RECORD_NAME);
        std::fs::write(&record, &written).unwrap();
        // One still being written, which is not a record yet.
        let partial = meeting.join(".work").join("own-speech.tkgate.part");
        std::fs::write(&partial, b"half a record").unwrap();

        let report = convert_all(
            &[dir.path().to_path_buf()],
            key.as_ref(),
            Direction::Encrypt,
            |_, _| {},
        );
        assert!(report.is_complete(), "{:?}", report.failed);
        assert!(
            file_looks_encrypted(&record),
            "the detector record was left in the clear beside sealed audio"
        );
        assert_eq!(std::fs::read(&partial).unwrap(), b"half a record");

        // And a password removed gives it back as it was, or the recordings it
        // describes would come back readable with the record unreadable.
        let report = convert_all(
            &[dir.path().to_path_buf()],
            key.as_ref(),
            Direction::Decrypt,
            |_, _| {},
        );
        assert!(report.is_complete(), "{:?}", report.failed);
        assert_eq!(std::fs::read(&record).unwrap(), written);
    }

    #[test]
    fn removing_the_password_gives_the_recordings_back_playable() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_dek();
        let audio = session_audio(3 * FRAME_LEN);
        let (_, files) = archive(dir.path(), &audio);

        convert_all(
            &[dir.path().to_path_buf()],
            key.as_ref(),
            Direction::Encrypt,
            |_, _| {},
        );
        let report = convert_all(
            &[dir.path().to_path_buf()],
            key.as_ref(),
            Direction::Decrypt,
            |_, _| {},
        );
        assert!(report.is_complete(), "{:?}", report.failed);
        assert_eq!(report.converted, 4);

        for path in &files {
            assert!(!file_looks_encrypted(path));
            // Byte for byte the recording that went in, openable by anything.
            assert_eq!(std::fs::read(path).unwrap(), audio);
        }
    }

    #[test]
    fn a_second_pass_has_nothing_left_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_dek();
        let (_, files) = archive(dir.path(), &session_audio(5000));

        let roots = [dir.path().to_path_buf()];
        convert_all(&roots, key.as_ref(), Direction::Encrypt, |_, _| {});
        let again = convert_all(&roots, key.as_ref(), Direction::Encrypt, |_, _| {});
        assert_eq!(again.converted, 0);
        assert_eq!(again.untouched, files.len());
    }

    #[test]
    fn a_crash_in_the_middle_leaves_the_recording_where_it_was() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_dek();
        let audio = session_audio(4000);
        let (meeting, _) = archive(dir.path(), &audio);
        let track = meeting.join("audio.mp4");

        // The two states a crash can leave: a part-written replacement, and an
        // original moved aside with the replacement not yet in place.
        std::fs::write(with_suffix(&track, IN_PROGRESS_SUFFIX), b"unfinished").unwrap();
        let moved_aside = meeting.join("mic.mp4");
        std::fs::rename(&moved_aside, with_suffix(&moved_aside, SUPERSEDED_SUFFIX)).unwrap();

        recover_interrupted(&[dir.path().to_path_buf()]);

        assert!(!with_suffix(&track, IN_PROGRESS_SUFFIX).exists());
        assert_eq!(std::fs::read(&track).unwrap(), audio);
        assert!(!with_suffix(&moved_aside, SUPERSEDED_SUFFIX).exists());
        assert_eq!(
            std::fs::read(&moved_aside).unwrap(),
            audio,
            "the recording that was moved aside was not put back"
        );
    }

    #[test]
    fn a_recording_that_cannot_be_read_is_reported_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let key = generate_dek();
        let meeting = dir.path().join("rec-broken");
        std::fs::create_dir_all(&meeting).unwrap();

        // Encrypted with a key this pass does not have — what a restored backup
        // from before a password change would look like.
        let stranger = generate_dek();
        let path = meeting.join("audio.mp4");
        let mut writer =
            EncryptedWriter::create(std::fs::File::create(&path).unwrap(), &stranger).unwrap();
        writer.write_all(&session_audio(500)).unwrap();
        writer.finish().unwrap();
        let before = std::fs::read(&path).unwrap();

        let report = convert_all(
            &[dir.path().to_path_buf()],
            key.as_ref(),
            Direction::Decrypt,
            |_, _| {},
        );
        assert!(!report.is_complete());
        assert_eq!(report.failed.len(), 1);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "a recording that could not be converted must not be damaged"
        );
    }
}
