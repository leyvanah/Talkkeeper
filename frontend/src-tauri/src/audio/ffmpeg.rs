//! Where FFmpeg is.
//!
//! Only the copy that ships with the application is used. FFmpeg is handed the
//! decrypted recording on stdin, so whichever `ffmpeg.exe` answers is trusted
//! with the session: an arbitrary one found on `PATH` or in the working
//! directory is not good enough. Nor is downloading one on the fly — that was
//! an unannounced request to a third-party server, and the archive it fetched
//! was run without checking what it was.

use log::{debug, error};
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};

#[cfg(not(windows))]
const EXECUTABLE_NAME: &str = "ffmpeg";

#[cfg(windows)]
const EXECUTABLE_NAME: &str = "ffmpeg.exe";

static FFMPEG_PATH: Lazy<Option<PathBuf>> = Lazy::new(find_ffmpeg_path_internal);

pub fn find_ffmpeg_path() -> Option<PathBuf> {
    FFMPEG_PATH.clone()
}

/// The places a bundled FFmpeg can be, relative to the executable's folder.
fn candidates(exe_folder: &Path) -> Vec<PathBuf> {
    let mut places = vec![exe_folder.join(EXECUTABLE_NAME)];

    #[cfg(target_os = "macos")]
    places.push(exe_folder.join("../Resources").join(EXECUTABLE_NAME));

    #[cfg(target_os = "linux")]
    places.push(exe_folder.join("lib").join(EXECUTABLE_NAME));

    // Test binaries run from `target/<profile>/deps`, one level below where
    // the build puts the sidecar for `tauri dev`.
    #[cfg(debug_assertions)]
    if let Some(profile_folder) = exe_folder.parent() {
        places.push(profile_folder.join(EXECUTABLE_NAME));
    }

    places
}

fn find_ffmpeg_path_internal() -> Option<PathBuf> {
    let exe_folder = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))?;

    let found = candidates(&exe_folder).into_iter().find(|path| path.is_file());
    match &found {
        Some(path) => debug!("Using the bundled ffmpeg: {:?}", path),
        None => error!("The bundled ffmpeg is missing; reinstalling the application restores it"),
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_places_beside_the_application_are_considered() {
        let folder = Path::new("install").join("app");
        let places = candidates(&folder);
        assert_eq!(places[0], folder.join(EXECUTABLE_NAME));
        // Nothing from PATH, the working directory or a download folder.
        assert!(places.iter().all(|place| place.starts_with("install")));
    }
}
