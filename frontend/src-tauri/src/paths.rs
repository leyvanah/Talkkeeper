//! Platform-safe local path resolution.
//!
//! Windows and Linux prefer a self-contained `data` directory beside the
//! executable. macOS always uses `~/Library/Application Support/Meetily` because
//! its executable lives inside the signed app bundle; writing runtime data there
//! invalidates the bundle signature.
//!
//! On portable platforms, everything is placed under `<exe_dir>/data`. If that
//! directory is not writable (for example, under `C:\Program Files`), storage
//! transparently falls back to the OS data directory.

use std::path::PathBuf;
use std::sync::OnceLock;

/// The subfolder (relative to the executable) that holds portable app data.
#[cfg(not(target_os = "macos"))]
const DATA_SUBDIR: &str = "data";
/// Folder name used under the OS data directory.
const FALLBACK_APP_NAME: &str = "Meetily";

static ROOT: OnceLock<PathBuf> = OnceLock::new();

/// Returns the app-managed data root, creating it if needed. Resolved once and
/// cached for the lifetime of the process.
///
/// This is the single source of truth that replaces every previous use of
/// Tauri's `app_data_dir()` and `dirs::data_dir()` for app-managed storage.
pub fn install_data_root() -> PathBuf {
    ROOT.get_or_init(|| {
        #[cfg(target_os = "macos")]
        {
            let root = os_data_root();
            log::info!("📁 macOS data root: {}", root.display());
            root
        }

        #[cfg(not(target_os = "macos"))]
        {
            // Preferred: next to the executable, under `data/`.
            if let Some(exe_dir) = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            {
                let candidate = exe_dir.join(DATA_SUBDIR);
                if ensure_writable(&candidate) {
                    log::info!("📁 Portable data root: {}", candidate.display());
                    return candidate;
                }
                log::warn!(
                    "Install directory not writable ({}); falling back to OS data dir",
                    candidate.display()
                );
            }

            let fallback = os_data_root();
            log::info!("📁 Fallback data root: {}", fallback.display());
            fallback
        }
    })
    .clone()
}

fn os_data_root() -> PathBuf {
    let root = dirs::data_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(FALLBACK_APP_NAME);
    let _ = std::fs::create_dir_all(&root);
    root
}

/// Directory that holds all downloaded speech/LLM models
/// (`<root>/models`). Mirrors the previous `app_data_dir/models` layout.
pub fn models_dir() -> PathBuf {
    let dir = install_data_root().join("models");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Directory that holds the application log (`<root>/logs`). Kept beside the
/// rest of the app-managed data so a portable install stays in one folder.
pub fn logs_dir() -> PathBuf {
    let dir = install_data_root().join("logs");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Ensure `dir` exists and is writable by probing an actual file write.
#[cfg(not(target_os = "macos"))]
fn ensure_writable(dir: &PathBuf) -> bool {
    if std::fs::create_dir_all(dir).is_err() {
        return false;
    }
    let probe = dir.join(".write_test");
    match std::fs::write(&probe, b"ok") {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}
