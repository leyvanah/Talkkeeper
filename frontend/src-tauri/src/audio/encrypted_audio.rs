//! Opening a recording whose bytes on disk may be encrypted.
//!
//! After B3 a recording is an encrypted stream (see
//! [`crate::security::stream`]), and everything that reads audio — the decoder,
//! the player's range requests, re-transcription, diarization — has to get
//! plaintext out of it without any of them learning how. This is the one place
//! that decides which kind of file it is looking at and hands back something
//! that reads like the audio.
//!
//! **The decision is made by what the file starts with, not by its name.** An
//! archive recorded before B3 is plaintext and has to keep opening; so does an
//! imported file that was never ours. A file that is encrypted while the
//! archive is locked is refused with a reason the interface can show, rather
//! than reported as damaged audio.
//!
//! Nothing here writes a decrypted copy anywhere. That is the point: a
//! temporary plaintext WAV of a session in the system temp directory would give
//! away everything the encryption was for.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{anyhow, Result};
use symphonia::core::io::MediaSource;

use crate::security::session::with_current_key;
use crate::security::stream::{file_looks_encrypted, EncryptedReader, EncryptedWriter};

/// A recording open for reading, in whichever form it is stored.
pub enum AudioSource {
    /// Written before B3, or imported from outside the archive.
    Plain(File, u64),
    /// Decrypted frame by frame as it is read.
    Encrypted(EncryptedReader<File>),
}

impl AudioSource {
    /// Open `path`, decrypting it if it is encrypted and the archive is open.
    pub fn open(path: &Path) -> Result<Self> {
        if !file_looks_encrypted(path) {
            return Self::plain(path);
        }
        let opened = with_current_key(|key| Self::encrypted(path, key)).ok_or_else(|| {
            anyhow!(
                "{} is encrypted and the archive is locked — unlock it to open this recording",
                path.display()
            )
        })?;
        opened
    }

    /// Open `path` with an explicit key, for tests and for the migration, which
    /// both hold the key already and must not depend on process-wide state.
    pub fn open_with_key(path: &Path, key: &[u8]) -> Result<Self> {
        if file_looks_encrypted(path) {
            Self::encrypted(path, key)
        } else {
            Self::plain(path)
        }
    }

    fn plain(path: &Path) -> Result<Self> {
        let file = File::open(path)
            .map_err(|error| anyhow!("Could not open {}: {error}", path.display()))?;
        let length = file.metadata()?.len();
        Ok(Self::Plain(file, length))
    }

    fn encrypted(path: &Path, key: &[u8]) -> Result<Self> {
        let file = File::open(path)
            .map_err(|error| anyhow!("Could not open {}: {error}", path.display()))?;
        let reader = EncryptedReader::open(file, key)
            .map_err(|error| anyhow!("Could not decrypt {}: {error}", path.display()))?;
        if !reader.complete() {
            // Not a failure: this is what a recording killed mid-session looks
            // like, and it plays up to where it stopped.
            log::warn!(
                "{} ends without its final frame — reading what was written",
                path.display()
            );
        }
        Ok(Self::Encrypted(reader))
    }

    /// Length of the audio, which for an encrypted file is not the length of
    /// the file: the header and the per-frame tags are not audio.
    pub fn len(&self) -> u64 {
        match self {
            Self::Plain(_, length) => *length,
            Self::Encrypted(reader) => reader.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the bytes on disk were encrypted.
    pub fn was_encrypted(&self) -> bool {
        matches!(self, Self::Encrypted(_))
    }
}

impl Read for AudioSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(file, _) => file.read(out),
            Self::Encrypted(reader) => reader.read(out),
        }
    }
}

impl Seek for AudioSource {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        match self {
            Self::Plain(file, _) => file.seek(to),
            Self::Encrypted(reader) => reader.seek(to),
        }
    }
}

impl MediaSource for AudioSource {
    /// Both kinds seek: that is what the frame layout was chosen for.
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len())
    }
}

/// A recording as a media source for the decoder, ready to be probed.
pub fn media_source(path: &Path) -> Result<Box<dyn MediaSource>> {
    Ok(Box::new(AudioSource::open(path)?))
}

/// The whole of a recording in memory, decrypted if it needs to be.
///
/// For callers that read the entire file anyway — a WAV being turned into
/// samples, say. Nothing is saved by streaming it there, and a path handed to
/// `std::fs::read` would come back as ciphertext.
pub fn read_all(path: &Path) -> Result<Vec<u8>> {
    let mut source = AudioSource::open(path)?;
    let mut bytes = Vec::with_capacity(source.len() as usize);
    source.read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// Somewhere to write audio, encrypted whenever there is a key to encrypt with.
///
/// Used for the tracks of a recording and for the intermediate files that
/// decoding produces. An archive with no password set writes plaintext — there
/// is nothing to encrypt with, and pretending otherwise would mean refusing to
/// record at all.
pub enum AudioSink {
    Plain(io::BufWriter<File>),
    Encrypted(EncryptedWriter<io::BufWriter<File>>),
}

impl AudioSink {
    /// Create `path`, encrypted if the archive is open.
    pub fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::into_file(File::create(path)?)
    }

    /// Wrap an already-created file, so callers that need the file first (a
    /// temporary name, a handle they made themselves) are not made to reopen it.
    pub fn into_file(file: File) -> Result<Self> {
        let buffered = io::BufWriter::with_capacity(1 << 16, file);
        // Asked before the key is borrowed, because borrowing it moves the file
        // into the closure and there would be no way back to writing plainly.
        if !crate::security::session::archive_is_open() {
            // No password: plaintext is what the owner chose. A password and
            // a locked archive: plaintext would be an accident, so refuse.
            if crate::security::session::archive_is_protected() {
                return Err(anyhow!("the archive is locked; the recording file was not written"));
            }
            return Ok(Self::Plain(buffered));
        }
        let writer = with_current_key(|key| EncryptedWriter::create(buffered, key))
            .ok_or_else(|| anyhow!("the archive was locked while a file was being opened"))?;
        Ok(Self::Encrypted(writer?))
    }

    /// Create `path` with an explicit key, for tests and the migration.
    pub fn create_with_key(path: &Path, key: &[u8]) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let buffered = io::BufWriter::with_capacity(1 << 16, File::create(path)?);
        Ok(Self::Encrypted(EncryptedWriter::create(buffered, key)?))
    }

    /// Close the stream: seal the last frame if there is one, and flush.
    pub fn finish(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(file) => file.flush(),
            Self::Encrypted(writer) => writer.finish(),
        }
    }

    /// Whether what lands on disk is encrypted.
    pub fn is_encrypted(&self) -> bool {
        matches!(self, Self::Encrypted(_))
    }
}

impl io::Write for AudioSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(file) => file.write(bytes),
            Self::Encrypted(writer) => writer.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(file) => file.flush(),
            Self::Encrypted(writer) => writer.flush(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::envelope::generate_dek;
    use crate::security::stream::{EncryptedWriter, FRAME_LEN};

    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 241) as u8).collect()
    }

    #[test]
    fn an_encrypted_recording_reads_as_the_audio_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let key = generate_dek();
        let plain = payload(2 * FRAME_LEN + 321);

        let mut writer = EncryptedWriter::create(File::create(&path).unwrap(), &key).unwrap();
        writer.write_all(&plain).unwrap();
        writer.finish().unwrap();

        let mut source = AudioSource::open_with_key(&path, key.as_ref()).unwrap();
        assert!(source.was_encrypted());
        assert_eq!(source.len(), plain.len() as u64);

        let mut read_back = Vec::new();
        source.read_to_end(&mut read_back).unwrap();
        assert_eq!(read_back, plain);

        // The player seeks; the decoder seeks more.
        source.seek(SeekFrom::Start(FRAME_LEN as u64 + 7)).unwrap();
        let mut window = [0u8; 64];
        source.read_exact(&mut window).unwrap();
        assert_eq!(&window[..], &plain[FRAME_LEN + 7..FRAME_LEN + 71]);
    }

    #[test]
    fn a_recording_from_before_b3_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let plain = payload(5000);
        std::fs::write(&path, &plain).unwrap();

        let key = generate_dek();
        let mut source = AudioSource::open_with_key(&path, key.as_ref()).unwrap();
        assert!(!source.was_encrypted(), "a plaintext file must not be decrypted");
        assert_eq!(source.len(), plain.len() as u64);
        let mut read_back = Vec::new();
        source.read_to_end(&mut read_back).unwrap();
        assert_eq!(read_back, plain);
    }

    #[test]
    fn an_encrypted_recording_is_refused_while_the_archive_is_locked() {
        // `open` asks the installed session for the key, and no session is
        // installed in tests — which is exactly the locked case.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let key = generate_dek();
        let mut writer = EncryptedWriter::create(File::create(&path).unwrap(), &key).unwrap();
        writer.write_all(&payload(100)).unwrap();
        writer.finish().unwrap();

        let error = match AudioSource::open(&path) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("an encrypted recording opened with no key in the session"),
        };
        assert!(error.contains("locked"), "unhelpful error: {error}");
    }

    #[test]
    fn the_wrong_key_does_not_quietly_produce_noise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audio.mp4");
        let mut writer =
            EncryptedWriter::create(File::create(&path).unwrap(), &generate_dek()).unwrap();
        writer.write_all(&payload(100)).unwrap();
        writer.finish().unwrap();

        let other = generate_dek();
        assert!(AudioSource::open_with_key(&path, other.as_ref()).is_err());
    }
}

#[cfg(test)]
mod recovery {
    use super::*;
    use std::path::PathBuf;

    /// Decrypt recordings with a keystore that is no longer the app's.
    ///
    /// Not a test of anything — a recovery tool, kept beside the primitives it
    /// uses so it cannot drift from them. It exists because an uninstall wiped
    /// the owner's keystore while his recordings, which live outside the
    /// install directory, survived: the audio was intact and unreadable at the
    /// same moment, and the only copy of the key was in a backup.
    ///
    /// The password is read from the environment and never appears here, in the
    /// output, or in any file this writes. Run it with the keystore that
    /// matches the recordings:
    ///
    /// ```text
    /// TK_KEYSTORE=...\keystore.json TK_PASSWORD=... \
    ///   TK_IN=...\rec-5c2abf... TK_OUT=...\recovered \
    ///   cargo test --lib recover_recordings -- --ignored --nocapture
    /// ```
    ///
    /// `TK_IN` may be one recording folder or the whole folder of them; every
    /// file underneath is copied to `TK_OUT`, decrypted if it was encrypted and
    /// passed through untouched if it was not. Files written under a different
    /// key are listed and skipped, because an archive can hold several.
    #[test]
    #[ignore = "recovery tool: needs TK_KEYSTORE, TK_PASSWORD, TK_IN, TK_OUT"]
    fn recover_recordings() {
        let need = |name: &str| {
            std::env::var(name)
                .unwrap_or_else(|_| panic!("{name} is not set; see the doc comment above"))
        };

        let keystore_path = PathBuf::from(need("TK_KEYSTORE"));
        let password = need("TK_PASSWORD");
        let input = PathBuf::from(need("TK_IN"));
        let output = PathBuf::from(need("TK_OUT"));

        let mut keystore = crate::security::keystore::Keystore::load_from(&keystore_path)
            .expect("the keystore could not be read")
            .expect("there is no keystore at that path");
        let dek = keystore
            .unlock_with_password(&password)
            .expect("that password does not open this keystore");

        let mut done = 0usize;
        let mut plain = 0usize;
        let mut failed = Vec::new();
        walk(&input, &input, &output, &dek[..], &mut done, &mut plain, &mut failed);

        println!("\n=== recovered into {} ===", output.display());
        println!("  decrypted:        {done}");
        println!("  copied as-is:     {plain}");
        println!("  not for this key:  {}", failed.len());
        for (path, why) in &failed {
            println!("    {} - {why}", path.display());
        }

        // Failures are expected and fine: a folder of recordings can hold
        // files written under several keys, and this key opens the ones it
        // opens. What is not fine is opening nothing at all, which means the
        // keystore does not belong to any of these recordings.
        assert!(
            done > 0,
            "not a single file decrypted with this keystore; it does not match these recordings"
        );
    }

    fn walk(
        root: &std::path::Path,
        dir: &std::path::Path,
        output: &std::path::Path,
        key: &[u8],
        done: &mut usize,
        plain: &mut usize,
        failed: &mut Vec<(PathBuf, String)>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                failed.push((dir.to_path_buf(), error.to_string()));
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, output, key, done, plain, failed);
                continue;
            }

            let relative = path.strip_prefix(root).unwrap_or(&path);
            let target = output.join(relative);
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }

            let was_encrypted = file_looks_encrypted(&path);
            match AudioSource::open_with_key(&path, key)
                .and_then(|mut source| {
                    let mut bytes = Vec::with_capacity(source.len() as usize);
                    source.read_to_end(&mut bytes)?;
                    std::fs::write(&target, &bytes)?;
                    Ok(())
                }) {
                Ok(()) => {
                    if was_encrypted {
                        *done += 1;
                    } else {
                        *plain += 1;
                    }
                    println!(
                        "  {} {}",
                        if was_encrypted { "decrypted" } else { "copied   " },
                        relative.display()
                    );
                }
                Err(error) => failed.push((path, error.to_string())),
            }
        }
    }
}
