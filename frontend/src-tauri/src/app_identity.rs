//! The app's own identifier, and the move away from the inherited one.
//!
//! Until this version the identifier was `com.meetily.ai`, the original
//! Meetily's. Windows keys a few things by it, so an installed Meetily and
//! Talkkeeper shared them: `%APPDATA%\<id>` (the onboarding status and the
//! recording settings), `%LOCALAPPDATA%\<id>\EBWebView` (the window's profile,
//! whose `localStorage` holds the interface settings), the notification
//! identity, and the folders an uninstaller removes when asked to delete data.
//! The archive, the recordings, the keys and the password live in the data
//! folder beside the program and never depended on it.
//!
//! On the first start under the new identifier, [`migrate`] copies what
//! Talkkeeper kept under the old one: the three settings stores and the
//! window's `localStorage`. Nothing else is carried — not the window's
//! IndexedDB, caches or sessions, nor the upstream leftovers (`analytics.json`,
//! `preferences.json`).
//!
//! A release build then removes the old folders, but only when they are
//! Talkkeeper's alone: every entry in them is one this app is known to have
//! written, and no Meetily is installed. Otherwise they are left as they are.
//! A development build only copies — it shares the folders with the installed
//! version, which still uses them.

use std::path::{Path, PathBuf};

/// This app's identifier. Must equal `identifier` in `tauri.conf.json`; Windows
/// uses it for the window profile, the notification identity and the
/// installer's shortcuts.
pub const IDENTIFIER: &str = "io.github.leyvanah.talkkeeper";

/// The identifier this app had before, shared with the original Meetily.
const INHERITED: &str = "com.meetily.ai";

/// The settings stores, by file name (`tauri-plugin-store`, in `%APPDATA%\<id>`).
const STORES: &[&str] = &[
    "onboarding-status.json",
    "recording_preferences.json",
    "recording_preferences-dev.json",
];

/// Everything Talkkeeper is known to have left in the old `%APPDATA%\<id>`.
/// The last two were written by upstream code the fork has since removed.
const KNOWN_ROAMING: &[&str] = &[
    "onboarding-status.json",
    "recording_preferences.json",
    "recording_preferences-dev.json",
    "analytics.json",
    "preferences.json",
];

/// Everything Talkkeeper is known to have left in the old `%LOCALAPPDATA%\<id>`.
const KNOWN_LOCAL: &[&str] = &["EBWebView"];

/// The WebView2 profile folder Tauri creates under `%LOCALAPPDATA%\<id>`.
const WEBVIEW_PROFILE: &str = "EBWebView";

/// Where `localStorage` lives inside a WebView2 profile.
const LOCAL_STORAGE: [&str; 2] = ["Default", "Local Storage"];

/// What the move did, for the log.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub stores_copied: Vec<&'static str>,
    pub local_storage_copied: bool,
    pub inherited_removed: bool,
    /// Why the old folders were kept, when they were.
    pub kept_because: Option<&'static str>,
    pub errors: Vec<String>,
}

/// Copies the settings kept under the old identifier and, in a release build,
/// removes the old folders when they are this app's alone. Never fails: a
/// problem is logged and the app starts with whatever was copied.
pub fn migrate() {
    let (Some(roaming), Some(local)) = (dirs::data_dir(), dirs::data_local_dir()) else {
        return;
    };
    if !roaming.join(INHERITED).exists() && !local.join(INHERITED).exists() {
        return;
    }
    let outcome = migrate_in(
        &roaming,
        &local,
        !cfg!(debug_assertions),
        meetily_is_installed(),
    );
    log_outcome(&outcome);

    #[cfg(windows)]
    {
        if !cfg!(debug_assertions) {
            forget_inherited_notification_identity();
        }
    }
}

fn log_outcome(outcome: &Outcome) {
    if !outcome.stores_copied.is_empty() {
        log::info!(
            "App identifier: copied settings from the old identifier: {}",
            outcome.stores_copied.join(", ")
        );
    }
    if outcome.local_storage_copied {
        log::info!("App identifier: copied the window's interface settings");
    }
    if outcome.inherited_removed {
        log::info!("App identifier: removed the folders of the old identifier");
    } else if let Some(reason) = outcome.kept_because {
        log::info!("App identifier: kept the folders of the old identifier: {reason}");
    }
    for error in &outcome.errors {
        log::warn!("App identifier: {error}");
    }
}

/// The move itself, on explicit roots so tests can run it in a temporary
/// folder. `roaming` and `local` stand for `%APPDATA%` and `%LOCALAPPDATA%`.
pub(crate) fn migrate_in(
    roaming: &Path,
    local: &Path,
    remove_inherited: bool,
    meetily_installed: bool,
) -> Outcome {
    let mut outcome = Outcome::default();
    let old_roaming = roaming.join(INHERITED);
    let new_roaming = roaming.join(IDENTIFIER);
    let old_local = local.join(INHERITED);
    let new_local = local.join(IDENTIFIER);

    for name in STORES {
        let from = old_roaming.join(name);
        let to = new_roaming.join(name);
        if !from.is_file() || to.exists() {
            continue;
        }
        match copy_file(&from, &to) {
            Ok(()) => outcome.stores_copied.push(name),
            Err(error) => outcome.errors.push(format!("could not copy {name}: {error}")),
        }
    }

    // Only into a profile that does not exist yet: WebView2 has not started,
    // and nothing newer can be overwritten.
    let old_storage = LOCAL_STORAGE
        .iter()
        .fold(old_local.join(WEBVIEW_PROFILE), |path, part| path.join(part));
    let new_profile = new_local.join(WEBVIEW_PROFILE);
    if old_storage.is_dir() && !new_profile.exists() {
        match copy_local_storage(&old_storage, &new_local) {
            Ok(()) => outcome.local_storage_copied = true,
            Err(error) => outcome
                .errors
                .push(format!("could not copy the window's interface settings: {error}")),
        }
    }

    if !remove_inherited {
        outcome.kept_because = Some("a development build only copies");
    } else if !outcome.errors.is_empty() {
        outcome.kept_because = Some("not everything was copied");
    } else if meetily_is_installed_or(meetily_installed, &old_roaming) {
        outcome.kept_because = Some("Meetily uses them too");
    } else if !only_known_entries(&old_roaming, KNOWN_ROAMING)
        || !only_known_entries(&old_local, KNOWN_LOCAL)
    {
        outcome.kept_because = Some("they hold files this app did not write");
    } else {
        outcome.inherited_removed = true;
        for (which, dir) in [("roaming", &old_roaming), ("local", &old_local)] {
            if !dir.exists() {
                continue;
            }
            if let Err(error) = std::fs::remove_dir_all(dir) {
                outcome.inherited_removed = false;
                outcome
                    .errors
                    .push(format!("could not remove the old {which} folder: {error}"));
            }
        }
    }
    outcome
}

/// Meetily's own database in the shared folder means Meetily ran here, even
/// when its uninstaller entry is gone.
fn meetily_is_installed_or(installed: bool, old_roaming: &Path) -> bool {
    installed
        || ["meeting_minutes.sqlite", "meeting_minutes.db"]
            .iter()
            .any(|name| old_roaming.join(name).exists())
}

/// True when the folder is missing or every entry in it is one of `known`.
fn only_known_entries(dir: &Path, known: &[&str]) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return !dir.exists();
    };
    entries.into_iter().all(|entry| {
        entry
            .ok()
            .and_then(|entry| entry.file_name().into_string().ok())
            .is_some_and(|name| known.contains(&name.as_str()))
    })
}

/// Copies through a temporary name, so a half-written file is never taken
/// for the real one.
fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let partial = to.with_extension("migrating");
    std::fs::copy(from, &partial)?;
    std::fs::rename(&partial, to)
}

/// Builds `<new_local>\EBWebView\Default\Local Storage` from the old one under
/// a temporary profile name and renames it into place only when complete.
fn copy_local_storage(old_storage: &Path, new_local: &Path) -> std::io::Result<()> {
    let partial_profile = new_local.join(format!("{WEBVIEW_PROFILE}.migrating"));
    if partial_profile.exists() {
        std::fs::remove_dir_all(&partial_profile)?;
    }
    let target = LOCAL_STORAGE
        .iter()
        .fold(partial_profile.clone(), |path, part| path.join(part));
    let result = copy_dir(old_storage, &target)
        .and_then(|()| std::fs::rename(&partial_profile, new_local.join(WEBVIEW_PROFILE)));
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&partial_profile);
    }
    result
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target: PathBuf = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Whether Windows lists an installed Meetily (per user or per machine).
#[cfg(windows)]
fn meetily_is_installed() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};
    use winreg::RegKey;

    const UNINSTALL: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall";
    let views = [
        (HKEY_CURRENT_USER, KEY_READ),
        (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_64KEY),
        (HKEY_LOCAL_MACHINE, KEY_READ | KEY_WOW64_32KEY),
    ];
    views.into_iter().any(|(root, flags)| {
        let Ok(uninstall) = RegKey::predef(root).open_subkey_with_flags(UNINSTALL, flags) else {
            return false;
        };
        uninstall.enum_keys().flatten().any(|name| {
            let display: String = uninstall
                .open_subkey_with_flags(&name, flags)
                .and_then(|key| key.get_value("DisplayName"))
                .unwrap_or_default();
            [name, display]
                .iter()
                .any(|text| text.to_ascii_lowercase().contains("meetily"))
        })
    })
}

#[cfg(not(windows))]
fn meetily_is_installed() -> bool {
    false
}

/// Removes the notification identity this app registered under the old
/// identifier — only if it is ours (it carries our display name).
#[cfg(windows)]
fn forget_inherited_notification_identity() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let path = format!("Software\\Classes\\AppUserModelId\\{INHERITED}");
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey(&path) else {
        return;
    };
    let display: String = key.get_value("DisplayName").unwrap_or_default();
    drop(key);
    if display == "Talkkeeper" {
        if let Err(error) = hkcu.delete_subkey_all(&path) {
            log::warn!("App identifier: could not remove the old notification identity: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// An old layout as this machine has it: the stores, two upstream
    /// leftovers, and a window profile with `localStorage` and IndexedDB.
    fn old_layout(roaming: &Path, local: &Path) {
        let old = roaming.join(INHERITED);
        write(&old.join("onboarding-status.json"), r#"{"completed":true}"#);
        write(&old.join("recording_preferences.json"), r#"{"folder":"x"}"#);
        write(&old.join("analytics.json"), "{}");
        write(&old.join("preferences.json"), "{}");
        let profile = local.join(INHERITED).join(WEBVIEW_PROFILE).join("Default");
        write(&profile.join("Local Storage").join("leveldb").join("000003.log"), "theme");
        write(&profile.join("IndexedDB").join("db").join("data"), "old journal");
    }

    #[test]
    fn the_identifier_is_the_one_tauri_builds_with() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], IDENTIFIER);
    }

    #[test]
    fn settings_and_local_storage_move_and_the_old_folders_go() {
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);

        let outcome = migrate_in(&roaming, &local, true, false);

        assert_eq!(
            outcome.stores_copied,
            vec!["onboarding-status.json", "recording_preferences.json"]
        );
        assert!(outcome.local_storage_copied);
        assert!(outcome.inherited_removed, "{outcome:?}");
        let new_roaming = roaming.join(IDENTIFIER);
        assert_eq!(
            std::fs::read_to_string(new_roaming.join("onboarding-status.json")).unwrap(),
            r#"{"completed":true}"#
        );
        assert!(!new_roaming.join("analytics.json").exists());
        let profile = local.join(IDENTIFIER).join(WEBVIEW_PROFILE).join("Default");
        assert_eq!(
            std::fs::read_to_string(profile.join("Local Storage").join("leveldb").join("000003.log"))
                .unwrap(),
            "theme"
        );
        assert!(!profile.join("IndexedDB").exists(), "only localStorage is carried");
        assert!(!roaming.join(INHERITED).exists());
        assert!(!local.join(INHERITED).exists());
        assert!(!local.join(IDENTIFIER).join("EBWebView.migrating").exists());
    }

    #[test]
    fn a_development_build_copies_but_leaves_the_old_folders() {
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);

        let outcome = migrate_in(&roaming, &local, false, false);

        assert!(outcome.local_storage_copied);
        assert!(!outcome.inherited_removed);
        assert!(roaming.join(INHERITED).join("onboarding-status.json").exists());
        assert!(local.join(INHERITED).exists());
    }

    #[test]
    fn folders_shared_with_meetily_stay() {
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);

        let installed = migrate_in(&roaming, &local, true, true);
        assert_eq!(installed.kept_because, Some("Meetily uses them too"));
        assert!(roaming.join(INHERITED).exists());

        // No uninstaller entry, but Meetily's database is in the folder.
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);
        write(&roaming.join(INHERITED).join("meeting_minutes.sqlite"), "db");
        let database = migrate_in(&roaming, &local, true, false);
        assert_eq!(database.kept_because, Some("Meetily uses them too"));
        assert!(roaming.join(INHERITED).join("meeting_minutes.sqlite").exists());
        assert!(local.join(INHERITED).exists());
    }

    #[test]
    fn an_unknown_file_keeps_the_old_folders() {
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);
        write(&local.join(INHERITED).join("something-else"), "?");

        let outcome = migrate_in(&roaming, &local, true, false);

        assert_eq!(outcome.kept_because, Some("they hold files this app did not write"));
        assert!(roaming.join(INHERITED).exists());
        assert!(local.join(INHERITED).join("something-else").exists());
    }

    #[test]
    fn nothing_newer_is_overwritten() {
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);
        write(&roaming.join(IDENTIFIER).join("onboarding-status.json"), "newer");
        write(&local.join(IDENTIFIER).join(WEBVIEW_PROFILE).join("Default").join("x"), "newer");

        let outcome = migrate_in(&roaming, &local, true, false);

        assert_eq!(outcome.stores_copied, vec!["recording_preferences.json"]);
        assert!(!outcome.local_storage_copied);
        assert_eq!(
            std::fs::read_to_string(roaming.join(IDENTIFIER).join("onboarding-status.json")).unwrap(),
            "newer"
        );
        // Everything of ours is either copied or already there: the old go.
        assert!(outcome.inherited_removed, "{outcome:?}");
    }

    #[test]
    fn a_second_start_does_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let (roaming, local) = (temp.path().join("roaming"), temp.path().join("local"));
        old_layout(&roaming, &local);
        migrate_in(&roaming, &local, true, false);

        let again = migrate_in(&roaming, &local, true, false);

        assert!(again.stores_copied.is_empty());
        assert!(!again.local_storage_copied);
        assert!(again.errors.is_empty());
    }
}
