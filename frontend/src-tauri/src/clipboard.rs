//! Copying text out of the archive without handing it to Windows to keep.
//!
//! A plain clipboard write is picked up by clipboard history (Win+V) and, when
//! the owner has turned it on, synced to their other devices through the cloud
//! clipboard. A transcript or a recovery code copied once would then outlive
//! the paste — outside the archive and its password, and possibly on someone
//! else's server. Windows honours three registered formats that opt a single
//! write out of both; this puts the text there together with them.
//!
//! The text still sits on the clipboard until something replaces it — that is
//! what copying is — but Windows does not keep a copy or upload it.

/// What the command answers on a platform with no private write, so the
/// window falls back to an ordinary copy there and only there.
pub const UNSUPPORTED: &str = "unsupported";

/// Puts `text` on the clipboard, excluded from clipboard history and from the
/// cloud clipboard.
#[tauri::command]
pub fn copy_text_privately(window: tauri::WebviewWindow, text: String) -> Result<(), String> {
    // An owner window rather than none: with no owner, EmptyClipboard leaves
    // the clipboard ownerless and SetClipboardData may refuse.
    #[cfg(windows)]
    let owner = window
        .hwnd()
        .map(|hwnd| hwnd.0 as *mut std::ffi::c_void)
        .unwrap_or(std::ptr::null_mut());
    #[cfg(not(windows))]
    let owner = {
        let _ = window;
        ()
    };
    imp::write(owner, &text)
}

#[cfg(windows)]
mod imp {
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{GlobalFree, HGLOBAL, HWND};
    use windows_sys::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

    /// The opt-out formats. The first works by being present at all; the other
    /// two by holding a DWORD zero. All three, because which one a given
    /// Windows build reads has changed over the years.
    pub(super) const OPT_OUT: [&str; 3] = [
        "ExcludeClipboardContentFromMonitorProcessing",
        "CanIncludeInClipboardHistory",
        "CanUploadToCloudClipboard",
    ];

    pub fn write(owner: HWND, text: &str) -> Result<(), String> {
        let _open = OpenClipboardGuard::open(owner)?;
        // Everything goes in inside one open and close: nothing watching the
        // clipboard ever sees the text without the opt-out next to it.
        unsafe {
            if EmptyClipboard() == 0 {
                return Err(last_error("EmptyClipboard"));
            }
        }
        for name in OPT_OUT {
            set(register(name)?, &0u32.to_le_bytes())?;
        }
        set(CF_UNICODETEXT as u32, &utf16_with_nul(text))
    }

    /// The text as the clipboard's Unicode format wants it: UTF-16LE with a
    /// terminating zero.
    pub(super) fn utf16_with_nul(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_le_bytes)
            .collect()
    }

    fn register(name: &str) -> Result<u32, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let format = unsafe { RegisterClipboardFormatW(wide.as_ptr()) };
        if format == 0 {
            return Err(last_error("RegisterClipboardFormatW"));
        }
        Ok(format)
    }

    /// Copies `data` into movable global memory and hands it to the clipboard.
    fn set(format: u32, data: &[u8]) -> Result<(), String> {
        unsafe {
            let memory: HGLOBAL = GlobalAlloc(GMEM_MOVEABLE, data.len());
            if memory.is_null() {
                return Err(last_error("GlobalAlloc"));
            }
            let target = GlobalLock(memory) as *mut u8;
            if target.is_null() {
                let error = last_error("GlobalLock");
                GlobalFree(memory);
                return Err(error);
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), target, data.len());
            GlobalUnlock(memory);
            // Once accepted, the memory belongs to the clipboard; until then
            // it is still ours to free.
            if SetClipboardData(format, memory).is_null() {
                let error = last_error("SetClipboardData");
                GlobalFree(memory);
                return Err(error);
            }
        }
        Ok(())
    }

    fn last_error(call: &str) -> String {
        format!("{call} failed: {}", std::io::Error::last_os_error())
    }

    /// The clipboard is shared and another program may hold it for a moment,
    /// so opening is retried briefly; it is always closed again on the way out.
    struct OpenClipboardGuard;

    impl OpenClipboardGuard {
        fn open(owner: HWND) -> Result<Self, String> {
            for _ in 0..10 {
                if unsafe { OpenClipboard(owner) } != 0 {
                    return Ok(Self);
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(last_error("OpenClipboard"))
        }
    }

    impl Drop for OpenClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn write(_owner: (), _text: &str) -> Result<(), String> {
        Err(super::UNSUPPORTED.to_string())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::*;

    #[test]
    fn the_text_goes_in_as_terminated_utf16() {
        assert_eq!(utf16_with_nul("Да"), vec![0x14, 0x04, 0x30, 0x04, 0, 0]);
        assert_eq!(utf16_with_nul(""), vec![0, 0]);
    }

    /// Replaces whatever is on this machine's clipboard, so it does not run
    /// with the rest: `cargo test clipboard -- --ignored`.
    #[test]
    #[ignore = "writes to the real clipboard"]
    fn a_private_copy_carries_the_opt_out_and_the_text() {
        use windows_sys::Win32::System::DataExchange::{
            CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
            RegisterClipboardFormatW,
        };
        use windows_sys::Win32::System::Memory::{GlobalLock, GlobalUnlock};
        use windows_sys::Win32::System::Ole::CF_UNICODETEXT;

        write(std::ptr::null_mut(), "Секретная фраза").unwrap();

        unsafe {
            assert_ne!(OpenClipboard(std::ptr::null_mut()), 0);
            for name in OPT_OUT {
                let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                let format = RegisterClipboardFormatW(wide.as_ptr());
                assert_ne!(IsClipboardFormatAvailable(format), 0, "{name} is missing");
            }
            let memory = GetClipboardData(CF_UNICODETEXT as u32);
            assert!(!memory.is_null());
            let start = GlobalLock(memory) as *const u16;
            let mut length = 0;
            while *start.add(length) != 0 {
                length += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(start, length));
            GlobalUnlock(memory);
            CloseClipboard();
            assert_eq!(text, "Секретная фраза");
        }
    }
}
