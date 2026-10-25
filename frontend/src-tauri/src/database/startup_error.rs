//! What the person sees when the archive cannot be opened at start.
//!
//! This used to be a panic: no window, and the program simply "did not
//! start". The data was never the problem - each migration runs in a
//! transaction, a newer archive is refused before anything is written - so
//! the window says that, says what to do, and offers the data folder.

use super::manager::OpenError;
use crate::ui_language::{archive_not_opened, text, ArchiveProblem, Text};

fn problem(error: &OpenError) -> ArchiveProblem<'_> {
    match error {
        OpenError::NewerArchive { needs } => ArchiveProblem::Newer { needs: needs.as_deref() },
        OpenError::BackupFailed(_) => ArchiveProblem::NotCopied,
        OpenError::Database(_) => ArchiveProblem::Other,
    }
}

/// Shows the window, opens the data folder if asked to, and ends the
/// program: nothing else can run without the archive.
pub fn explain_and_exit(error: &OpenError) -> ! {
    log::error!("The archive could not be opened at start: {error}");
    let message = archive_not_opened(problem(error));
    if ask(text(Text::ErrorTitle), &message) {
        open_data_folder();
    }
    std::process::exit(1);
}

/// A system message box with Yes and No; true for Yes. It needs no window of
/// the app's own, which does not exist yet at this point.
#[cfg(target_os = "windows")]
fn ask(title: &str, message: &str) -> bool {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_ICONERROR, MB_SETFOREGROUND, MB_YESNO,
    };
    let answer = unsafe {
        MessageBoxW(
            None,
            &HSTRING::from(message),
            &HSTRING::from(title),
            MB_YESNO | MB_ICONERROR | MB_SETFOREGROUND,
        )
    };
    answer == IDYES
}

#[cfg(not(target_os = "windows"))]
fn ask(_title: &str, message: &str) -> bool {
    eprintln!("{message}");
    false
}

fn open_data_folder() {
    let folder = crate::paths::install_data_root();
    #[cfg(target_os = "windows")]
    let opened = std::process::Command::new("explorer").arg(&folder).spawn();
    #[cfg(target_os = "macos")]
    let opened = std::process::Command::new("open").arg(&folder).spawn();
    #[cfg(target_os = "linux")]
    let opened = std::process::Command::new("xdg-open").arg(&folder).spawn();
    if let Err(error) = opened {
        log::warn!("Could not open the data folder: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_error_gets_its_own_explanation() {
        let newer = OpenError::NewerArchive { needs: Some("0.2.40".to_string()) };
        assert_eq!(problem(&newer), ArchiveProblem::Newer { needs: Some("0.2.40") });
        assert_eq!(problem(&OpenError::BackupFailed("disk full".into())), ArchiveProblem::NotCopied);
        let other = OpenError::Database(sqlx::Error::Protocol("broken".into()));
        assert_eq!(problem(&other), ArchiveProblem::Other);
    }
}
