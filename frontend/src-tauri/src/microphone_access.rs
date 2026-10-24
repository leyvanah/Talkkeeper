//! Windows microphone privacy switches (Settings → Privacy & security →
//! Microphone). With access denied there, WASAPI still lists the microphones
//! and only opening one fails, so the device list alone cannot tell the user
//! why recording does not start. Read-only: the app never changes them, it
//! only opens that Settings page for the user.

/// The consent-store key for the microphone; `Value` is "Allow" or "Deny".
#[cfg(windows)]
const CONSENT_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";

/// The Settings page with those switches.
#[cfg(windows)]
const SETTINGS_URI: &str = "ms-settings:privacy-microphone";

/// A missing value means the switch was never touched, which is "on".
fn denies(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case("deny"))
}

/// Blocked when any switch on the path to this app is off: access for the
/// whole device (HKLM), for apps of this user (HKCU) or for desktop apps,
/// which this app is (HKCU\…\NonPackaged).
fn blocked(device: Option<&str>, apps: Option<&str>, desktop_apps: Option<&str>) -> bool {
    denies(device) || denies(apps) || denies(desktop_apps)
}

#[cfg(windows)]
fn consent_value(root: &winreg::RegKey, path: &str) -> Option<String> {
    root.open_subkey(path)
        .ok()?
        .get_value::<String, _>("Value")
        .ok()
}

/// Whether Windows denies this app the microphone. Always false elsewhere.
#[tauri::command]
pub fn microphone_access_blocked() -> bool {
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
        use winreg::RegKey;
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let device = consent_value(&hklm, CONSENT_KEY);
        let apps = consent_value(&hkcu, CONSENT_KEY);
        let desktop_apps = consent_value(&hkcu, &format!("{CONSENT_KEY}\\NonPackaged"));
        blocked(device.as_deref(), apps.as_deref(), desktop_apps.as_deref())
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Opens the microphone page of Windows Settings. Not a network call: the
/// target is a fixed `ms-settings:` URI, nothing comes from the caller.
#[tauri::command]
pub fn open_microphone_privacy_settings() -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt, ptr};
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};

        let wide = |s: &str| -> Vec<u16> { OsStr::new(s).encode_wide().chain(Some(0)).collect() };
        let operation = wide("open");
        let target = wide(SETTINGS_URI);
        let result = unsafe {
            ShellExecuteW(
                ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                ptr::null(),
                ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            return Err(format!("Could not open the microphone settings (code {})", result as isize));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err("Only available on Windows".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_switches_allow() {
        assert!(!blocked(None, None, None));
        assert!(!blocked(Some("Allow"), Some("Allow"), Some("Allow")));
    }

    #[test]
    fn any_switch_off_blocks() {
        assert!(blocked(Some("Deny"), Some("Allow"), Some("Allow")));
        assert!(blocked(None, Some("Deny"), None));
        assert!(blocked(Some("Allow"), None, Some("Deny")));
    }

    #[test]
    fn value_is_read_loosely() {
        assert!(denies(Some(" deny ")));
        assert!(!denies(Some("Prompt")));
        assert!(!denies(Some("")));
    }
}
