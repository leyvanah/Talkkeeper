//! Refusing network locations for paths the window hands to a command.
//!
//! A few commands write or read whatever path the window names: the readable
//! library export, the transfer package and its import, the key backup. The
//! window got that path from a dialog the owner answered, but nothing on this
//! side can tell — and a network share (`\\host\share`, or a drive letter
//! mapped to one) is the network, reached over SMB, past `network_policy` and
//! the window's request filter. Library contents and the key copy are exactly
//! what must not go there by mistake or by a script in the window.
//!
//! So such a path is refused before it is touched at all: the check reads the
//! text and the drive's type, and does not open anything on the share.

use std::path::Path;

/// Refuses `path` unless it is an absolute path on a local drive.
pub fn refuse_network_location(path: &Path) -> Result<(), String> {
    if is_network_location(path) {
        return Err("Network folders are not allowed here: choose a folder on this computer".into());
    }
    Ok(())
}

/// Whether `path` names a network location, or is not an absolute path on a
/// local drive at all (a relative path, a device path).
pub fn is_network_location(path: &Path) -> bool {
    let text = path.to_string_lossy();
    // `\\host\share`, `\\?\UNC\host\share`, `\\.\device` — and the same with
    // forward slashes, which Windows accepts too. A verbatim `\\?\C:\…` is a
    // local path written the long way; it is judged by its drive below.
    let verbatim_local = text
        .strip_prefix(r"\\?\")
        .filter(|rest| !rest.get(..4).is_some_and(|head| head.eq_ignore_ascii_case(r"UNC\")));
    let local_text = match verbatim_local {
        Some(rest) => rest,
        None if text.starts_with(r"\\") || text.starts_with("//") => return true,
        None => text.as_ref(),
    };
    let Some(drive) = drive_letter(local_text) else {
        // Not `X:\…`: relative, or rooted without a drive. Neither is a place
        // the owner chose in a dialog.
        return true;
    };
    drive_is_remote(drive)
}

/// The drive letter of `X:\…` or `X:/…`.
fn drive_letter(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    (chars.next() == Some(':') && matches!(chars.next(), Some('\\' | '/'))).then_some(letter)
}

#[cfg(windows)]
fn drive_is_remote(letter: char) -> bool {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    // DRIVE_REMOTE: a drive letter mapped to a network share.
    const DRIVE_REMOTE: u32 = 4;
    let root: Vec<u16> = format!("{letter}:\\").encode_utf16().chain([0]).collect();
    // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call.
    unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
}

#[cfg(not(windows))]
fn drive_is_remote(_letter: char) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_share_is_refused_however_it_is_written() {
        for share in [
            r"\\127.0.0.1\c$",
            r"\\host\share\export",
            "//host/share/export",
            r"\\?\UNC\host\share",
            r"\\?\unc\host\share",
            r"\\.\pipe\name",
        ] {
            assert!(is_network_location(Path::new(share)), "{share}");
            assert!(refuse_network_location(Path::new(share)).is_err(), "{share}");
        }
    }

    #[test]
    fn a_path_without_a_drive_is_refused() {
        for odd in ["", "export", r"..\export", r"\export", "C:export"] {
            assert!(is_network_location(Path::new(odd)), "{odd:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_local_folder_is_allowed() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().to_path_buf();
        assert!(!is_network_location(&local), "{}", local.display());
        let verbatim = local.canonicalize().unwrap();
        assert!(!is_network_location(&verbatim), "{}", verbatim.display());
        let forward = PathBuf::from(local.to_string_lossy().replace('\\', "/"));
        assert!(!is_network_location(&forward), "{}", forward.display());
    }
}
