//! The interface language, for what the backend shows on its own.
//!
//! The window keeps the owner's choice of language; the tray menu, the icon's
//! tooltip and the Windows notifications are drawn here, outside the window,
//! and used to come out in one fixed language each — the tray always Russian,
//! the notifications always English. The window now tells the backend its
//! language at start and on every change, and the choice is kept in a small
//! file so the tray is right before the window has loaded.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use tauri::{AppHandle, Runtime};

const FILE: &str = "ui_language.json";

/// The interface languages the backend has words for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    fn parse(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().as_str() {
            "ru" => Some(Self::Ru),
            "en" => Some(Self::En),
            _ => None,
        }
    }

    fn code(self) -> &'static str {
        match self {
            Self::Ru => "ru",
            Self::En => "en",
        }
    }
}

/// Russian until told otherwise: it is the interface's default language.
static CURRENT: AtomicU8 = AtomicU8::new(0);

fn to_u8(lang: Lang) -> u8 {
    match lang {
        Lang::Ru => 0,
        Lang::En => 1,
    }
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::SeqCst) {
        1 => Lang::En,
        _ => Lang::Ru,
    }
}

fn path() -> PathBuf {
    crate::paths::install_data_root().join(FILE)
}

/// Reads the language the window last chose. Missing or unreadable keeps
/// the default.
pub fn load() {
    let stored = std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| value.get("language")?.as_str().and_then(Lang::parse));
    if let Some(lang) = stored {
        CURRENT.store(to_u8(lang), Ordering::SeqCst);
    }
}

/// The window's language changed (or the window started): remember it and
/// redraw the tray in it. Notifications pick it up as they are raised.
#[tauri::command]
pub fn set_ui_language<R: Runtime>(app: AppHandle<R>, language: String) -> Result<(), String> {
    let lang = Lang::parse(&language).ok_or_else(|| format!("Unknown language: {language}"))?;
    let changed = current() != lang;
    CURRENT.store(to_u8(lang), Ordering::SeqCst);
    if changed {
        let text = serde_json::json!({ "language": lang.code() }).to_string();
        if let Err(error) = std::fs::write(path(), text) {
            log::warn!("Could not remember the interface language: {error}");
        }
        crate::tray::update_tray_menu(&app);
    }
    Ok(())
}

/// Everything the backend says on its own, in both languages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    TrayLoadingModel,
    TrayStartRecording,
    TrayStarting,
    TrayPause,
    TrayStopRecording,
    TrayPausing,
    TrayResume,
    TrayResuming,
    TrayStopping,
    TrayOpenWindow,
    TraySettings,
    TrayQuit,
    TooltipRecording,
    TooltipPaused,
    TooltipStopping,
    RecordingStarted,
    RecordingStopped,
    RecordingPaused,
    RecordingResumed,
    TranscriptionComplete,
    TestNotification,
    ErrorTitle,
}

pub fn text(key: Text) -> &'static str {
    words(current(), key)
}

fn words(lang: Lang, key: Text) -> &'static str {
    use Text::*;
    match (lang, key) {
        (Lang::Ru, TrayLoadingModel) => "⏳ Загружается модель распознавания…",
        (Lang::En, TrayLoadingModel) => "⏳ Loading the speech recognition model…",
        (Lang::Ru, TrayStartRecording) => "Начать запись",
        (Lang::En, TrayStartRecording) => "Start recording",
        (Lang::Ru, TrayStarting) => "🔄 Запись запускается…",
        (Lang::En, TrayStarting) => "🔄 Starting the recording…",
        (Lang::Ru, TrayPause) => "⏸ Пауза",
        (Lang::En, TrayPause) => "⏸ Pause",
        (Lang::Ru, TrayStopRecording) => "⏹ Остановить запись",
        (Lang::En, TrayStopRecording) => "⏹ Stop recording",
        (Lang::Ru, TrayPausing) => "⏸ Ставим на паузу…",
        (Lang::En, TrayPausing) => "⏸ Pausing…",
        (Lang::Ru, TrayResume) => "▶ Продолжить запись",
        (Lang::En, TrayResume) => "▶ Resume recording",
        (Lang::Ru, TrayResuming) => "▶ Продолжаем…",
        (Lang::En, TrayResuming) => "▶ Resuming…",
        (Lang::Ru, TrayStopping) => "⏹ Останавливаем…",
        (Lang::En, TrayStopping) => "⏹ Stopping…",
        (Lang::Ru, TrayOpenWindow) => "Открыть окно",
        (Lang::En, TrayOpenWindow) => "Open window",
        (Lang::Ru, TraySettings) => "Настройки",
        (Lang::En, TraySettings) => "Settings",
        (Lang::Ru, TrayQuit) => "Выйти",
        (Lang::En, TrayQuit) => "Quit",
        (Lang::Ru, TooltipRecording) => "Talkkeeper — идёт запись",
        (Lang::En, TooltipRecording) => "Talkkeeper — recording",
        (Lang::Ru, TooltipPaused) => "Talkkeeper — запись на паузе",
        (Lang::En, TooltipPaused) => "Talkkeeper — recording paused",
        (Lang::Ru, TooltipStopping) => "Talkkeeper — запись завершается",
        (Lang::En, TooltipStopping) => "Talkkeeper — finishing the recording",
        (Lang::Ru, RecordingStarted) => {
            "Запись началась. Предупредите собеседников, что разговор записывается."
        }
        (Lang::En, RecordingStarted) => {
            "Recording has started. Please inform others in the meeting that you are recording."
        }
        (Lang::Ru, RecordingStopped) => "Запись остановлена и сохранена",
        (Lang::En, RecordingStopped) => "Recording has been stopped and saved",
        (Lang::Ru, RecordingPaused) => "Запись на паузе",
        (Lang::En, RecordingPaused) => "Recording has been paused",
        (Lang::Ru, RecordingResumed) => "Запись продолжается",
        (Lang::En, RecordingResumed) => "Recording has been resumed",
        (Lang::Ru, TranscriptionComplete) => "Транскрипция готова",
        (Lang::En, TranscriptionComplete) => "Transcription has been completed",
        (Lang::Ru, TestNotification) => "Проверка: уведомления работают",
        (Lang::En, TestNotification) => {
            "This is a test notification to verify the system is working correctly"
        }
        (Lang::Ru, ErrorTitle) => "Talkkeeper — ошибка",
        (Lang::En, ErrorTitle) => "Talkkeeper Error",
    }
}

/// "Meeting starts in N minutes", with the plural Russian needs.
pub fn meeting_reminder(minutes: u64) -> String {
    match current() {
        Lang::En => format!("Meeting starts in {minutes} minutes"),
        Lang::Ru => format!("Встреча через {minutes} {}", russian_minutes(minutes)),
    }
}

fn russian_minutes(n: u64) -> &'static str {
    match (n % 10, n % 100) {
        (1, rest) if rest != 11 => "минуту",
        (2..=4, rest) if !(12..=14).contains(&rest) => "минуты",
        _ => "минут",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_text_has_both_languages_and_they_differ() {
        use Text::*;
        for key in [
            TrayLoadingModel, TrayStartRecording, TrayStarting, TrayPause, TrayStopRecording,
            TrayPausing, TrayResume, TrayResuming, TrayStopping, TrayOpenWindow, TraySettings,
            TrayQuit, TooltipRecording, TooltipPaused, TooltipStopping, RecordingStarted,
            RecordingStopped, RecordingPaused, RecordingResumed, TranscriptionComplete,
            TestNotification, ErrorTitle,
        ] {
            let ru = words(Lang::Ru, key);
            let en = words(Lang::En, key);
            assert!(!ru.is_empty() && !en.is_empty(), "{key:?}");
            assert_ne!(ru, en, "{key:?} is not translated");
        }
    }

    #[test]
    fn the_window_codes_are_understood() {
        assert_eq!(Lang::parse("ru"), Some(Lang::Ru));
        assert_eq!(Lang::parse(" EN "), Some(Lang::En));
        assert_eq!(Lang::parse("de"), None);
    }

    #[test]
    fn russian_minutes_agree_with_the_number() {
        assert_eq!(russian_minutes(1), "минуту");
        assert_eq!(russian_minutes(3), "минуты");
        assert_eq!(russian_minutes(5), "минут");
        assert_eq!(russian_minutes(11), "минут");
        assert_eq!(russian_minutes(21), "минуту");
        assert_eq!(russian_minutes(12), "минут");
        assert_eq!(russian_minutes(22), "минуты");
    }
}
