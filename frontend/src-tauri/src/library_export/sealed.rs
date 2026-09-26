//! The transfer package: the export for another installation, sealed.
//!
//! ```text
//! <target>/Talkkeeper transfer 2026-09-26 15-30/
//!     package.json                  plain: what it is, how to derive the key
//!     library.json                  sealed
//!     meetings/<meeting id>/
//!         meeting.json              sealed
//!         audio.mp4, mic.mp4, ...   sealed
//! ```
//!
//! Every file but `package.json` is an encrypted stream
//! ([`crate::security::stream`]) under a key derived with Argon2id from a
//! password chosen for this transfer. The installation's own key never goes
//! into the package: the other installation has its own, and re-encrypts
//! everything with it on import. Folder names are the recordings' ids, so
//! the package's directory listing says how many recordings there are and
//! nothing about them.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use base64::Engine as _;
use chrono::Local;
use rand::RngCore;
use serde::{Deserialize, Serialize};

use super::{safe_name, CollectedMeeting, Manifest, MeetingEntry, MANIFEST_NAME, MEETING_NAME};
use crate::audio::encrypted_audio::AudioSource;
use crate::security::kdf::{derive_kek, Kek, KdfParams, SALT_LEN};
use crate::security::stream::{EncryptedReader, EncryptedWriter, StreamError};

/// The one plain file of a package.
pub const PACKAGE_NAME: &str = "package.json";
const KIND: &str = "talkkeeper-transfer";
/// A package travels on disks and through clouds; its password is all that
/// stands between it and an offline guessing attack.
pub const MIN_PASSWORD_CHARS: usize = 8;

/// What `package.json` says: enough to derive the key, nothing more.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PackageHeader {
    pub kind: String,
    pub format: u32,
    pub kdf: KdfParams,
    /// Base64.
    pub salt: String,
}

/// Heavier than the archive's own unlock: a package is opened once, and it
/// is the thing most likely to end up somewhere it should not be.
fn transfer_params() -> KdfParams {
    KdfParams {
        memory_kib: 64 * 1024,
        iterations: 3,
        parallelism: 1,
    }
}

/// A package states its own parameters; refuse ones that would make opening
/// it a way to exhaust this machine.
fn reasonable(params: &KdfParams) -> bool {
    (8 * 1024..=1024 * 1024).contains(&params.memory_kib)
        && (1..=10).contains(&params.iterations)
        && (1..=8).contains(&params.parallelism)
}

pub fn check_password(password: &str) -> Result<(), String> {
    if password.chars().count() < MIN_PASSWORD_CHARS {
        return Err(format!(
            "The transfer password needs at least {MIN_PASSWORD_CHARS} characters"
        ));
    }
    Ok(())
}

/// A fresh header and the key it describes.
pub fn new_key(password: &str) -> Result<(PackageHeader, Kek), String> {
    check_password(password)?;
    let mut salt = [0u8; SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let params = transfer_params();
    let key = derive_kek(password.as_bytes(), &salt, params).map_err(|error| error.to_string())?;
    Ok((
        PackageHeader {
            kind: KIND.to_string(),
            format: super::FORMAT_VERSION,
            kdf: params,
            salt: base64::engine::general_purpose::STANDARD.encode(salt),
        },
        key,
    ))
}

/// The key a package's header and password give. A wrong password gives a
/// key too; it is the first sealed file that tells.
pub fn key_for(header: &PackageHeader, password: &str) -> Result<Kek, String> {
    if header.kind != KIND {
        return Err("This folder is not a Talkkeeper transfer package".to_string());
    }
    if header.format > super::FORMAT_VERSION {
        return Err("The package was made by a newer version of Talkkeeper".to_string());
    }
    if !reasonable(&header.kdf) {
        return Err("The package asks for key parameters this version will not use".to_string());
    }
    let salt = base64::engine::general_purpose::STANDARD
        .decode(&header.salt)
        .map_err(|_| "The package header is damaged".to_string())?;
    derive_kek(password.as_bytes(), &salt, header.kdf).map_err(|error| error.to_string())
}

pub fn read_header(root: &Path) -> Result<PackageHeader, String> {
    let bytes = fs::read(root.join(PACKAGE_NAME))
        .map_err(|_| "This folder is not a Talkkeeper transfer package".to_string())?;
    serde_json::from_slice(&bytes).map_err(|_| "The package header is damaged".to_string())
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

/// Seal whatever `source` yields into `target`, under a temporary name until
/// it is whole.
fn seal_into(target: &Path, key: &[u8], source: &mut impl Read) -> io::Result<u64> {
    let partial = with_suffix(target, ".part");
    let file = io::BufWriter::with_capacity(1 << 16, fs::File::create(&partial)?);
    let mut writer = EncryptedWriter::create(file, key)?;
    let copied = io::copy(source, &mut writer)?;
    writer.finish()?;
    drop(writer);
    fs::rename(&partial, target)?;
    Ok(copied)
}

/// A sealed file opened for reading. A wrong key and a damaged file look the
/// same, and a package opened with the wrong password is the likely one.
pub fn open_sealed(path: &Path, key: &[u8]) -> Result<EncryptedReader<fs::File>, String> {
    let file = fs::File::open(path).map_err(|error| format!("Could not open a package file: {error}"))?;
    let reader = EncryptedReader::open(file, key).map_err(|error| match error {
        StreamError::WrongKeyOrDamaged => "Wrong password, or the package is damaged".to_string(),
        other => format!("Could not read a package file: {other}"),
    })?;
    if !reader.complete() {
        return Err("A package file was cut short — copy the package again".to_string());
    }
    Ok(reader)
}

pub fn read_sealed(path: &Path, key: &[u8]) -> Result<Vec<u8>, String> {
    let mut reader = open_sealed(path, key)?;
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|_| "Wrong password, or the package is damaged".to_string())?;
    Ok(bytes)
}

/// A package being written.
pub struct SealedExport {
    root: PathBuf,
    key: Kek,
}

impl SealedExport {
    pub fn create(target: &Path, password: &str) -> Result<Self, String> {
        let (header, key) = new_key(password)?;
        let stamp = Local::now().format("%Y-%m-%d %H-%M").to_string();
        let mut root = target.join(format!("Talkkeeper transfer {stamp}"));
        let mut attempt = 2;
        while root.exists() {
            root = target.join(format!("Talkkeeper transfer {stamp} ({attempt})"));
            attempt += 1;
        }
        fs::create_dir_all(root.join("meetings"))
            .map_err(|error| format!("Could not create the package folder: {error}"))?;
        let json = serde_json::to_vec_pretty(&header).map_err(|error| error.to_string())?;
        fs::write(root.join(PACKAGE_NAME), json)
            .map_err(|error| format!("Could not write the package header: {error}"))?;
        Ok(Self { root, key })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn write_meeting(&mut self, meeting: &CollectedMeeting) -> io::Result<(MeetingEntry, u64)> {
        let record = &meeting.record;
        let relative = format!("meetings/{}", safe_name(&record.id, "meeting"));
        let folder = self.root.join(&relative);
        if folder.exists() {
            return Err(io::Error::new(io::ErrorKind::AlreadyExists, "two recordings share an id"));
        }
        fs::create_dir_all(&folder)?;

        let mut bytes = 0;
        for (name, source) in &meeting.audio {
            let mut reader = AudioSource::open(source).map_err(io::Error::other)?;
            bytes += seal_into(&folder.join(name), self.key.as_ref(), &mut reader)?;
        }
        let json = serde_json::to_vec(record).map_err(io::Error::other)?;
        seal_into(&folder.join(MEETING_NAME), self.key.as_ref(), &mut json.as_slice())?;
        Ok((MeetingEntry { id: record.id.clone(), folder: relative }, bytes))
    }

    pub fn write_manifest(&self, manifest: &Manifest) -> io::Result<()> {
        let json = serde_json::to_vec(manifest).map_err(io::Error::other)?;
        seal_into(&self.root.join(MANIFEST_NAME), self.key.as_ref(), &mut json.as_slice())?;
        Ok(())
    }
}

/// A package being read.
pub struct SealedPackage {
    root: PathBuf,
    key: Kek,
}

impl SealedPackage {
    /// Opens the package and reads its list. This is where a wrong password
    /// is found out, before anything is written.
    pub fn open(root: &Path, password: &str) -> Result<(Self, Manifest), String> {
        let header = read_header(root)?;
        let key = key_for(&header, password)?;
        let manifest: Manifest = serde_json::from_slice(&read_sealed(&root.join(MANIFEST_NAME), key.as_ref())?)
            .map_err(|_| "The package's list of recordings is damaged".to_string())?;
        if manifest.format > super::FORMAT_VERSION {
            return Err("The package was made by a newer version of Talkkeeper".to_string());
        }
        Ok((Self { root: root.to_path_buf(), key }, manifest))
    }

    /// The folder of an entry, refused if it would lead outside the package.
    fn folder_of(&self, entry: &MeetingEntry) -> Result<PathBuf, String> {
        let safe = entry
            .folder
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && !part.contains(['\\', ':']));
        if !safe {
            return Err("The package lists a folder outside itself".to_string());
        }
        Ok(self.root.join(&entry.folder))
    }

    pub fn read_meeting(&self, entry: &MeetingEntry) -> Result<super::MeetingRecord, String> {
        let bytes = read_sealed(&self.folder_of(entry)?.join(MEETING_NAME), self.key.as_ref())?;
        serde_json::from_slice(&bytes).map_err(|_| "A recording in the package is damaged".to_string())
    }

    /// A track of an entry, opened for reading as plain audio.
    pub fn open_track(&self, entry: &MeetingEntry, name: &str) -> Result<EncryptedReader<fs::File>, String> {
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Err("The package names a track outside its recording".to_string());
        }
        open_sealed(&self.folder_of(entry)?.join(name), self.key.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library_export::{MeetingRecord, FORMAT_VERSION};

    fn record() -> MeetingRecord {
        MeetingRecord {
            id: "meeting-1".into(),
            title: "Разговор".into(),
            created_at: "2026-09-12T10:00:00+00:00".into(),
            updated_at: "2026-09-12T11:00:00+00:00".into(),
            client_id: None,
            lines: Vec::new(),
            summary: None,
            speakers: Vec::new(),
            tracks: vec!["audio.mp4".into()],
        }
    }

    fn manifest(entries: Vec<MeetingEntry>) -> Manifest {
        Manifest {
            format: FORMAT_VERSION,
            app_version: "test".into(),
            exported_at: String::new(),
            clients: Vec::new(),
            meetings: entries,
        }
    }

    #[test]
    fn a_package_opens_with_its_password_and_with_nothing_else() {
        let target = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("audio.mp4"), b"recorded audio").unwrap();

        let mut export = SealedExport::create(target.path(), "long enough").unwrap();
        let meeting = CollectedMeeting {
            record: record(),
            audio: vec![("audio.mp4".into(), source.path().join("audio.mp4"))],
        };
        let (entry, bytes) = export.write_meeting(&meeting).unwrap();
        export.write_manifest(&manifest(vec![entry.clone()])).unwrap();
        assert_eq!(bytes, 14);

        // On disk: only the header is readable, and it names nobody.
        let root = export.root().to_path_buf();
        let listing = std::fs::read(root.join(&entry.folder).join(MEETING_NAME)).unwrap();
        assert!(!String::from_utf8_lossy(&listing).contains("Разговор"));
        assert!(!std::fs::read(root.join(&entry.folder).join("audio.mp4"))
            .unwrap()
            .windows(8)
            .any(|window| window == b"recorded"));

        assert_eq!(
            SealedPackage::open(&root, "wrong password").err().as_deref(),
            Some("Wrong password, or the package is damaged")
        );
        let (package, listed) = SealedPackage::open(&root, "long enough").unwrap();
        assert_eq!(listed.meetings, vec![entry.clone()]);
        assert_eq!(package.read_meeting(&entry).unwrap(), record());
        let mut audio = Vec::new();
        package.open_track(&entry, "audio.mp4").unwrap().read_to_end(&mut audio).unwrap();
        assert_eq!(audio, b"recorded audio");
    }

    #[test]
    fn a_short_password_is_refused() {
        let target = tempfile::tempdir().unwrap();
        assert!(SealedExport::create(target.path(), "short").is_err());
    }

    #[test]
    fn a_package_cannot_point_outside_itself() {
        let target = tempfile::tempdir().unwrap();
        let export = SealedExport::create(target.path(), "long enough").unwrap();
        export.write_manifest(&manifest(Vec::new())).unwrap();
        let (package, _) = SealedPackage::open(export.root(), "long enough").unwrap();
        for folder in ["../elsewhere", "meetings/../../x", "C:/x", "meetings\\x", ""] {
            let entry = MeetingEntry { id: "m".into(), folder: folder.into() };
            assert!(package.read_meeting(&entry).is_err(), "{folder}");
        }
        let entry = MeetingEntry { id: "m".into(), folder: "meetings/m".into() };
        for track in ["../audio.mp4", "..", "a/b.mp4", ".hidden"] {
            assert!(package.open_track(&entry, track).is_err(), "{track}");
        }
    }

    #[test]
    fn a_header_asking_for_absurd_work_is_refused() {
        let (mut header, _) = new_key("long enough").unwrap();
        header.kdf.memory_kib = 64 * 1024 * 1024;
        assert!(key_for(&header, "long enough").is_err());
        let (mut header, _) = new_key("long enough").unwrap();
        header.kind = "something else".into();
        assert!(key_for(&header, "long enough").is_err());
    }
}
