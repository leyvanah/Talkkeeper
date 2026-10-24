//! Whether a model download fits on the disk it is going to.
//!
//! Without this a download ran for minutes and failed with a general
//! "download failed" when the disk filled up — and sometimes left the
//! unfinished file behind, taking the space it had used. Now the space is
//! checked before the first byte, a full disk during the download says so
//! in the same words, and every downloader removes what it could not finish.
//!
//! The error carries a fixed marker and the two numbers, so the window can
//! say how much is needed and how much is free in the interface language
//! (`frontend/src/lib/disk-space.ts`).

use std::fmt::Display;
use std::path::{Path, PathBuf};

use anyhow::anyhow;

/// Kept free beyond the download itself. A disk filled to the last byte
/// fails everything else first — the database, the log, the recording.
pub const MARGIN_BYTES: u64 = 256 * 1024 * 1024;

/// The marker a refused download's error carries, followed by
/// `needed=<bytes> available=<bytes>`.
pub const NO_SPACE: &str = "NO_DISK_SPACE";

/// What a download needs on disk, margin included.
pub fn needed_with_margin(download_bytes: u64) -> u64 {
    download_bytes.saturating_add(MARGIN_BYTES)
}

/// Whether `download_bytes` fit into `available` with the margin left over.
pub fn has_room(download_bytes: u64, available: u64) -> bool {
    download_bytes
        .checked_add(MARGIN_BYTES)
        .is_some_and(|needed| needed <= available)
}

/// The error for a download that does not fit. `needed` includes the margin,
/// so "needed" is never below "available" in the message.
pub fn no_space_error(download_bytes: u64, available: u64) -> anyhow::Error {
    anyhow!(
        "{} needed={} available={}: not enough free disk space for the download",
        NO_SPACE,
        needed_with_margin(download_bytes),
        available
    )
}

/// Whether an error (or the text of one) is a refusal for lack of space.
pub fn is_no_space(error: &impl Display) -> bool {
    error.to_string().contains(NO_SPACE)
}

/// Whether a write failed because the disk is full.
pub fn is_out_of_space(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::StorageFull {
        return true;
    }
    // ERROR_HANDLE_DISK_FULL and ERROR_DISK_FULL on Windows, ENOSPC elsewhere.
    #[cfg(windows)]
    let codes: &[i32] = &[39, 112];
    #[cfg(not(windows))]
    let codes: &[i32] = &[28];
    error.raw_os_error().is_some_and(|code| codes.contains(&code))
}

/// Free bytes on the volume holding `dir`, or `None` when it cannot be told.
/// `dir` need not exist yet: the nearest existing ancestor decides.
pub fn available_bytes(dir: &Path) -> Option<u64> {
    let existing = dir.ancestors().find(|path| path.exists())?;
    let target = plain_path(std::fs::canonicalize(existing).ok()?);
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|disk| target.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().as_os_str().len())
        .map(|disk| disk.available_space())
}

/// `canonicalize` on Windows answers `\\?\C:\...`, which no mount point
/// starts with.
fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = text.strip_prefix(r"\\?\") {
        return PathBuf::from(rest);
    }
    path
}

/// Refuses a download of `download_bytes` into `dir` that would not fit.
/// When the free space cannot be told, the download goes ahead: a full disk
/// is then still caught on the first failed write.
pub fn ensure_room(dir: &Path, download_bytes: u64) -> anyhow::Result<()> {
    match available_bytes(dir) {
        Some(available) if !has_room(download_bytes, available) => {
            log::warn!(
                "Refusing a download of {} bytes: {} bytes free",
                download_bytes,
                available
            );
            Err(no_space_error(download_bytes, available))
        }
        Some(_) => Ok(()),
        None => {
            log::warn!("Could not tell the free disk space; downloading without the check");
            Ok(())
        }
    }
}

/// The error for a failed write or flush: a full disk is reported like the
/// check before the download, anything else as `what: error`.
pub fn write_error(
    error: std::io::Error,
    dir: &Path,
    remaining_bytes: u64,
    what: impl Display,
) -> anyhow::Error {
    if is_out_of_space(&error) {
        no_space_error(remaining_bytes, available_bytes(dir).unwrap_or(0))
    } else {
        anyhow!("{}: {}", what, error)
    }
}

/// A file being downloaded that is removed if the download does not finish:
/// on an error, an early return or a panic alike. `keep` once it is complete.
pub struct UnfinishedFile {
    path: Option<PathBuf>,
}

impl UnfinishedFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }

    /// The download finished: leave the file where it is.
    pub fn keep(mut self) {
        self.path = None;
    }
}

impl Drop for UnfinishedFile {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            match std::fs::remove_file(&path) {
                Ok(()) => log::info!("Removed unfinished download {}", path.display()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => log::warn!(
                    "Could not remove unfinished download {}: {}",
                    path.display(),
                    error
                ),
            }
        }
    }
}

/// Whether downloads of `download_bytes` in total fit into the models folder.
/// Asked by onboarding before the first of its downloads, so that recognition
/// is not fetched only for the summary model to fail halfway.
#[tauri::command]
pub fn check_model_download_space(download_bytes: u64) -> Result<(), String> {
    ensure_room(&crate::paths::models_dir(), download_bytes).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn room_counts_the_margin() {
        assert!(has_room(1000 * MIB, 1000 * MIB + MARGIN_BYTES));
        assert!(!has_room(1000 * MIB, 1000 * MIB + MARGIN_BYTES - 1));
        // Free space equal to the download is not enough.
        assert!(!has_room(1000 * MIB, 1000 * MIB));
        assert!(has_room(0, MARGIN_BYTES));
        assert!(!has_room(u64::MAX, u64::MAX));
    }

    #[test]
    fn the_refusal_carries_both_numbers() {
        let error = no_space_error(2_000, 500);
        let text = error.to_string();
        assert!(is_no_space(&error));
        assert!(text.starts_with(NO_SPACE));
        assert!(text.contains(&format!("needed={}", 2_000 + MARGIN_BYTES)));
        assert!(text.contains("available=500"));
        // Wrapped by a command, it is still recognised.
        assert!(is_no_space(&format!("Failed to download model: {}", text)));
        assert!(!is_no_space(&"Download timeout"));
    }

    #[test]
    fn a_full_disk_is_told_from_other_write_errors() {
        assert!(is_out_of_space(&std::io::Error::from(
            std::io::ErrorKind::StorageFull
        )));
        #[cfg(windows)]
        {
            assert!(is_out_of_space(&std::io::Error::from_raw_os_error(112)));
            assert!(is_out_of_space(&std::io::Error::from_raw_os_error(39)));
        }
        assert!(!is_out_of_space(&std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )));

        let dir = tempfile::tempdir().unwrap();
        let other = write_error(
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            dir.path(),
            10,
            "Failed to write",
        );
        assert!(!is_no_space(&other));
        let full = write_error(
            std::io::Error::from(std::io::ErrorKind::StorageFull),
            dir.path(),
            10,
            "Failed to write",
        );
        assert!(is_no_space(&full));
    }

    #[test]
    fn a_folder_not_made_yet_is_measured_on_its_volume() {
        let dir = tempfile::tempdir().unwrap();
        let later = dir.path().join("models").join("gigaam");
        assert!(available_bytes(&later).is_some());
    }

    /// Against the real disk the tests run on: one byte more than is free
    /// is refused, with the free space the disk reported.
    #[test]
    fn a_download_larger_than_the_disk_is_refused_before_it_starts() {
        let dir = tempfile::tempdir().unwrap();
        let free = available_bytes(dir.path()).expect("free space of the temp volume");

        let refused = ensure_room(dir.path(), free + 1).unwrap_err();
        assert!(is_no_space(&refused));
        assert!(refused.to_string().contains(&format!("needed={}", free + 1 + MARGIN_BYTES)));

        if free > MARGIN_BYTES {
            assert!(ensure_room(dir.path(), 0).is_ok());
        }
    }

    #[test]
    fn an_unfinished_file_goes_unless_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dropped = dir.path().join("dropped.part");
        let kept = dir.path().join("kept.bin");
        std::fs::write(&dropped, b"half").unwrap();
        std::fs::write(&kept, b"whole").unwrap();

        drop(UnfinishedFile::new(&dropped));
        UnfinishedFile::new(&kept).keep();

        assert!(!dropped.exists());
        assert!(kept.exists());
        // Nothing to remove is not an error.
        drop(UnfinishedFile::new(dir.path().join("never-written")));
    }
}
