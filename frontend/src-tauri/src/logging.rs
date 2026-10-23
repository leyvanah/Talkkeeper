//! The application log, on disk.
//!
//! A release build on Windows is linked with `windows_subsystem = "windows"`
//! and has no console, so everything `env_logger` writes to stderr goes
//! nowhere. When a recording or a summary misbehaved, nothing was left to read
//! afterwards and the only evidence was whatever happened to reach the
//! database. This tees the same records into `<data>/logs/meetily.log`.
//!
//! **What must never reach this file: anything a session said.** The log sits
//! beside the archive, in the clear, and outlives the run. Log counts, lengths,
//! ids, states and durations — never transcript or summary text. Two guards
//! back that rule up rather than trusting it: only `Info` and louder is written
//! by default, and every line is cut at [`MAX_LINE_CHARS`], so a stray body
//! dump cannot quietly fill the file with somebody's words. A test reads the
//! sources for log calls that pass a transcript, summary, note or title along
//! (`no_log_call_passes_meeting_text`).
//!
//! Versions before that rule held wrote every live transcript line here, so
//! the files left by them are erased once on the first start of this one, and
//! again whenever the archive is given a password ([`erase`]).
//!
//! stderr keeps behaving exactly as before, `RUST_LOG` included: the terminal
//! logger is the same `env_logger` instance, and this type only adds a second
//! destination.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{LevelFilter, Log, Metadata, Record};

/// Rotate once the file passes this, keeping a single previous generation.
/// Two files of this size is a rounding error next to one recording, and it is
/// enough to hold a long session's worth of `Info`.
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Longest single line written to disk. Any log call that tries to say more
/// than this is either a bug or a body dump; neither belongs in the file.
const MAX_LINE_CHARS: usize = 2000;

const LOG_FILE_NAME: &str = "meetily.log";
const PREVIOUS_LOG_FILE_NAME: &str = "meetily.prev.log";

/// Left in the log folder once the files written by older versions, which
/// logged live transcript text, have been erased. Its presence is the whole
/// signal; the content only says what it is to someone who opens the folder.
const OLD_LOGS_ERASED_MARKER: &str = "old-logs-erased";

/// The open log file, shared with [`erase`]. Empty when the file could not be
/// opened or the logger was never installed (tests).
static FILE: OnceLock<Mutex<FileSink>> = OnceLock::new();

/// The log folder, for [`erase`] to reach the files even without [`FILE`].
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

struct FileSink {
    writer: BufWriter<File>,
    written: u64,
    path: PathBuf,
    previous_path: PathBuf,
}

impl FileSink {
    fn open(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(LOG_FILE_NAME);
        let previous_path = dir.join(PREVIOUS_LOG_FILE_NAME);
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            writer: BufWriter::new(file),
            written,
            path,
            previous_path,
        })
    }

    fn rotate_if_needed(&mut self) {
        if self.written < MAX_FILE_BYTES {
            return;
        }
        // A failed rotation must not lose the log: keep writing to the current
        // file and try again on the next record.
        if self.writer.flush().is_err() {
            return;
        }
        if std::fs::rename(&self.path, &self.previous_path).is_err() {
            return;
        }
        match OpenOptions::new().create(true).append(true).open(&self.path) {
            Ok(file) => {
                self.writer = BufWriter::new(file);
                self.written = 0;
            }
            Err(_) => {
                // The renamed file is gone from under us and a new one will not
                // open. Put it back so the next record still lands somewhere.
                let _ = std::fs::rename(&self.previous_path, &self.path);
            }
        }
    }

    fn write_line(&mut self, line: &str) {
        self.rotate_if_needed();
        if self.writer.write_all(line.as_bytes()).is_err() {
            return;
        }
        if self.writer.write_all(b"\r\n").is_err() {
            return;
        }
        let _ = self.writer.flush();
        self.written += line.len() as u64 + 2;
    }

    /// Empties the current file and removes the previous generation, keeping
    /// the sink open so the next record still lands in `meetily.log`.
    fn erase(&mut self) -> std::io::Result<()> {
        let _ = self.writer.flush();
        // Truncate through a second handle: on Windows an append-only handle
        // may not shorten its file. The one kept here goes on writing at the
        // new end, which is now the start.
        OpenOptions::new().write(true).truncate(true).open(&self.path)?;
        self.written = 0;
        remove_if_present(&self.previous_path)
    }
}

fn remove_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

fn erase_files_in(dir: &Path) -> std::io::Result<()> {
    remove_if_present(&dir.join(LOG_FILE_NAME))?;
    remove_if_present(&dir.join(PREVIOUS_LOG_FILE_NAME))
}

/// Erases the logs left by versions that wrote transcript text into them —
/// once: the marker it leaves keeps later starts from erasing this version's
/// own, clean log. A failure leaves no marker, so the next start tries again.
fn erase_logs_from_older_versions(dir: &Path) -> std::io::Result<bool> {
    let marker = dir.join(OLD_LOGS_ERASED_MARKER);
    if marker.exists() {
        return Ok(false);
    }
    erase_files_in(dir)?;
    std::fs::write(
        &marker,
        "Logs from versions that wrote transcript text were erased.\r\n",
    )?;
    Ok(true)
}

/// Erases the application log: the current file is emptied, the previous one
/// removed. Called when the archive is given a password — the log is not
/// encrypted, and whatever an older version wrote into it must not outlive
/// the moment the owner asked for the archive to be protected.
pub fn erase() -> std::io::Result<()> {
    if let Some(file) = FILE.get() {
        let mut sink = file
            .lock()
            .map_err(|_| std::io::Error::other("log file lock poisoned"))?;
        return sink.erase();
    }
    match LOG_DIR.get() {
        Some(dir) => erase_files_in(dir),
        None => Ok(()),
    }
}

/// Writes every record to the log file and hands the ones stderr wants to
/// `env_logger`, so a terminal run looks the same as it always did.
struct TeeLogger {
    terminal: env_logger::Logger,
    file: &'static Mutex<FileSink>,
    file_level: LevelFilter,
}

impl Log for TeeLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.terminal.enabled(metadata) || metadata.level() <= self.file_level
    }

    fn log(&self, record: &Record) {
        if self.terminal.matches(record) {
            self.terminal.log(record);
        }
        if record.level() > self.file_level {
            return;
        }
        let line = format_record(record);
        if let Ok(mut file) = self.file.lock() {
            file.write_line(&line);
        }
    }

    fn flush(&self) {
        self.terminal.flush();
        if let Ok(mut file) = self.file.lock() {
            let _ = file.writer.flush();
        }
    }
}

fn format_record(record: &Record) -> String {
    let message = truncate_for_log(&record.args().to_string());
    format!(
        "{} {:<5} {}: {}",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        record.level(),
        record.target(),
        message
    )
}

/// Cuts an over-long message and says so, so a truncated line is never mistaken
/// for the whole story.
fn truncate_for_log(message: &str) -> String {
    if message.chars().count() <= MAX_LINE_CHARS {
        return message.replace(['\n', '\r'], " ");
    }
    let kept: String = message.chars().take(MAX_LINE_CHARS).collect();
    format!("{} …[cut]", kept.replace(['\n', '\r'], " "))
}

/// Longest error text the window may hand over. A backend error is a sentence
/// or two; anything longer is not worth keeping and may be carrying content.
const MAX_FRONTEND_DETAIL_CHARS: usize = 500;

/// The line written for a failure the window reported. The action is a fixed
/// name chosen in the code (`speaker-rename`), kept to a safe alphabet so a
/// caller cannot smuggle text into it; the detail is the original error text,
/// which the owner was shown only as a phrase in their own language.
fn frontend_failure_line(action: &str, detail: &str) -> String {
    let action: String = action
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64)
        .collect();
    let detail = detail.replace(['\n', '\r'], " ");
    let detail = if detail.chars().count() > MAX_FRONTEND_DETAIL_CHARS {
        let kept: String = detail.chars().take(MAX_FRONTEND_DETAIL_CHARS).collect();
        format!("{kept} …[cut]")
    } else {
        detail
    };
    format!("[ui] {action} failed: {detail}")
}

/// Puts a failure from the window into the application log: the window shows
/// the owner a phrase of its own, and the original text would otherwise be
/// lost with the developer console nobody has open. Local only — it writes
/// to the same file as every other record.
#[tauri::command]
pub fn log_frontend_failure(action: String, detail: String) {
    log::error!("{}", frontend_failure_line(&action, &detail));
}

/// Installs the logger. Falls back to stderr alone when the file cannot be
/// opened — a missing log is worth a warning, never a failed start.
///
/// Safe to call more than once: later calls are ignored rather than panicking,
/// which is what `env_logger::init` does.
pub fn init(log_dir: PathBuf) {
    let terminal = env_logger::Builder::from_default_env()
        .filter_level(LevelFilter::Info)
        .build();
    let terminal_level = terminal.filter();

    if FILE.get().is_some() {
        return;
    }
    let _ = LOG_DIR.set(log_dir.clone());
    // Before the file is opened, so nothing is holding it. The outcome is
    // reported once the logger is up and there is somewhere to say it.
    let erased_old_logs = erase_logs_from_older_versions(&log_dir);

    let sink = match FileSink::open(log_dir.clone()) {
        Ok(sink) => sink,
        Err(error) => {
            let _ = log::set_boxed_logger(Box::new(terminal));
            log::set_max_level(terminal_level);
            log::warn!(
                "No log file: {} could not be opened ({error}). Logging to stderr only.",
                log_dir.display()
            );
            return;
        }
    };

    let file_level = LevelFilter::Info;
    let max_level = terminal_level.max(file_level);
    if FILE.set(Mutex::new(sink)).is_err() {
        return;
    }
    let Some(file) = FILE.get() else {
        return;
    };
    let logger = TeeLogger {
        terminal,
        file,
        file_level,
    };

    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max_level);
        log::info!("Log file: {}", log_dir.join(LOG_FILE_NAME).display());
        match erased_old_logs {
            Ok(true) => log::info!("Logs written by older versions were erased"),
            Ok(false) => {}
            Err(error) => log::warn!(
                "Logs written by older versions could not be erased yet ({error}); trying again next start"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frontend_failure_keeps_the_action_name_and_the_text() {
        let line = frontend_failure_line("speaker-rename", "Database is locked");
        assert_eq!(line, "[ui] speaker-rename failed: Database is locked");
    }

    #[test]
    fn a_frontend_failure_cannot_put_text_in_the_action_name() {
        let line = frontend_failure_line("rename: Anna said hello\nnext", "x");
        assert_eq!(line, "[ui] renameAnnasaidhellonext failed: x");
    }

    #[test]
    fn a_long_frontend_failure_is_cut_and_stays_on_one_line() {
        let long = format!("first\n{}", "y".repeat(MAX_FRONTEND_DETAIL_CHARS * 2));
        let line = frontend_failure_line("save", &long);
        assert!(line.ends_with("…[cut]"));
        assert!(!line.contains('\n'));
        assert!(line.chars().count() < MAX_FRONTEND_DETAIL_CHARS + 40);
    }

    #[test]
    fn a_long_message_is_cut_and_says_so() {
        let long = "x".repeat(MAX_LINE_CHARS + 500);
        let cut = truncate_for_log(&long);
        assert!(cut.ends_with("…[cut]"));
        assert_eq!(cut.chars().count(), MAX_LINE_CHARS + "…[cut]".chars().count() + 1);
    }

    #[test]
    fn a_short_message_is_left_alone() {
        assert_eq!(truncate_for_log("started"), "started");
    }

    #[test]
    fn newlines_never_split_one_record_across_lines() {
        // A record that spanned lines would look like several events in the
        // file, and could forge a plausible-looking log entry.
        assert_eq!(truncate_for_log("a\nb\r\nc"), "a b  c");
    }

    #[test]
    fn the_file_rotates_and_keeps_one_previous_generation() {
        let dir = std::env::temp_dir().join(format!("meetily-log-test-{}", uuid::Uuid::new_v4()));
        let mut sink = FileSink::open(dir.clone()).unwrap();

        // Count what this loop wrote, not `sink.written` — that one resets on
        // rotation, which is the whole point of the test and would spin here
        // forever.
        let line = "y".repeat(1024);
        let mut pushed: u64 = 0;
        while pushed < MAX_FILE_BYTES + 4096 {
            sink.write_line(&line);
            pushed += line.len() as u64 + 2;
        }

        assert!(dir.join(PREVIOUS_LOG_FILE_NAME).is_file());
        assert!(dir.join(LOG_FILE_NAME).is_file());
        let current = std::fs::metadata(dir.join(LOG_FILE_NAME)).unwrap().len();
        assert!(current < MAX_FILE_BYTES, "rotation left {current} bytes behind");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopening_appends_rather_than_erasing_the_previous_run() {
        let dir = std::env::temp_dir().join(format!("meetily-log-test-{}", uuid::Uuid::new_v4()));
        {
            let mut sink = FileSink::open(dir.clone()).unwrap();
            sink.write_line("first run");
        }
        {
            let mut sink = FileSink::open(dir.clone()).unwrap();
            sink.write_line("second run");
        }

        let contents = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap();
        assert!(contents.contains("first run"));
        assert!(contents.contains("second run"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn test_dir() -> PathBuf {
        std::env::temp_dir().join(format!("meetily-log-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn erasing_empties_the_open_file_and_removes_the_previous_one() {
        let dir = test_dir();
        let mut sink = FileSink::open(dir.clone()).unwrap();
        sink.write_line("old line");
        std::fs::write(dir.join(PREVIOUS_LOG_FILE_NAME), "older line").unwrap();

        sink.erase().unwrap();
        sink.write_line("new line");

        let contents = std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap();
        assert_eq!(contents, "new line\r\n");
        assert!(!dir.join(PREVIOUS_LOG_FILE_NAME).exists());

        drop(sink);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn logs_from_older_versions_are_erased_once() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(LOG_FILE_NAME), "transcribed: secret phrase").unwrap();
        std::fs::write(dir.join(PREVIOUS_LOG_FILE_NAME), "transcribed: secret").unwrap();

        assert!(erase_logs_from_older_versions(&dir).unwrap());
        assert!(!dir.join(LOG_FILE_NAME).exists());
        assert!(!dir.join(PREVIOUS_LOG_FILE_NAME).exists());

        // This version's own log survives every later start.
        std::fs::write(dir.join(LOG_FILE_NAME), "started").unwrap();
        assert!(!erase_logs_from_older_versions(&dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join(LOG_FILE_NAME)).unwrap(), "started");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_install_has_nothing_to_erase_and_is_marked_anyway() {
        let dir = test_dir();
        std::fs::create_dir_all(&dir).unwrap();
        assert!(erase_logs_from_older_versions(&dir).unwrap());
        assert!(dir.join(OLD_LOGS_ERASED_MARKER).is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- The rule in the module comment, checked against the sources ----

    /// Names that, in this code base, hold what a meeting said or what was
    /// written about it. A log call may say how long one is, never pass it on.
    const MEETING_TEXT_NAMES: &[&str] = &[
        "transcript",
        "text",
        "cleaned_text",
        "cleaned_result",
        "final_result",
        "final_text",
        "segment_text",
        "trimmed",
        "summary",
        "markdown",
        "final_markdown",
        "answer",
        "question",
        "prompt",
        "note",
        "notes",
        "title",
        "meeting_name",
        "meeting_title",
        "reply",
        "content",
        // What is typed into search, and the names given to speakers.
        "query",
        "search_query",
        "speaker_name",
        "old_name",
        "new_name",
        "person_name",
    ];

    /// Every macro that ends up in `log`, the crate's own wrappers included.
    const LOG_MACROS: &[&str] = &[
        "trace",
        "debug",
        "info",
        "warn",
        "error",
        "perf_debug",
        "perf_trace",
        "async_debug",
        "async_info",
        "async_warn",
    ];

    /// What may follow such a name: its size or presence, never its value.
    const SIZE_ONLY: &[&str] = &["len", "is_empty", "is_some", "is_none", "chars"];

    #[derive(Debug, PartialEq)]
    enum Token {
        Ident(String),
        Str(String),
        Punct(char),
    }

    /// Just enough of a Rust lexer to tell code from comments, strings and
    /// char literals. Returns each token with its line.
    fn tokenize(source: &str) -> Vec<(Token, usize)> {
        let chars: Vec<char> = source.chars().collect();
        let mut tokens = Vec::new();
        let mut line = 1;
        let mut i = 0;
        let is_ident = |c: char| c.is_alphanumeric() || c == '_';
        while i < chars.len() {
            let c = chars[i];
            let next = chars.get(i + 1).copied();
            if c == '\n' {
                line += 1;
                i += 1;
            } else if c.is_whitespace() {
                i += 1;
            } else if c == '/' && next == Some('/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            } else if c == '/' && next == Some('*') {
                let mut depth = 0;
                while i < chars.len() {
                    if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                        depth += 1;
                        i += 2;
                    } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        if chars[i] == '\n' {
                            line += 1;
                        }
                        i += 1;
                    }
                }
            } else if c == '"' {
                let start_line = line;
                let mut text = String::new();
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' {
                        text.push(chars[i]);
                        i += 1;
                    }
                    if let Some(&ch) = chars.get(i) {
                        if ch == '\n' {
                            line += 1;
                        }
                        text.push(ch);
                    }
                    i += 1;
                }
                i += 1;
                tokens.push((Token::Str(text), start_line));
            } else if c == 'r'
                && matches!(next, Some('"') | Some('#'))
                && (i == 0 || !is_ident(chars[i - 1]) || chars[i - 1] == 'b')
                && {
                    let mut j = i + 1;
                    while chars.get(j) == Some(&'#') {
                        j += 1;
                    }
                    chars.get(j) == Some(&'"')
                }
            {
                let start_line = line;
                let mut hashes = 0;
                i += 1;
                while chars[i] == '#' {
                    hashes += 1;
                    i += 1;
                }
                i += 1;
                let mut text = String::new();
                loop {
                    if i >= chars.len() {
                        break;
                    }
                    if chars[i] == '"' && (1..=hashes).all(|k| chars.get(i + k) == Some(&'#')) {
                        i += 1 + hashes;
                        break;
                    }
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    text.push(chars[i]);
                    i += 1;
                }
                // A `b` before the `r` was already pushed as an identifier.
                if let Some((Token::Ident(prev), _)) = tokens.last() {
                    if prev == "b" {
                        tokens.pop();
                    }
                }
                tokens.push((Token::Str(text), start_line));
            } else if c == '\'' {
                if next == Some('\\') {
                    // An escaped char literal: '\n', '\'', '\u{..}'.
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    // A lifetime or a label.
                    i += 1;
                }
            } else if is_ident(c) {
                let start = i;
                while i < chars.len() && is_ident(chars[i]) {
                    i += 1;
                }
                tokens.push((Token::Ident(chars[start..i].iter().collect()), line));
            } else {
                tokens.push((Token::Punct(c), line));
                i += 1;
            }
        }
        tokens
    }

    /// The names a format string reads by itself: `{text}`, `{text:?}`.
    fn inline_names(format: &str) -> Vec<String> {
        let mut names = Vec::new();
        let mut chars = format.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '{' {
                continue;
            }
            if chars.peek() == Some(&'{') {
                chars.next();
                continue;
            }
            let name: String = std::iter::from_fn(|| chars.next_if(|c| c.is_alphanumeric() || *c == '_')).collect();
            if !name.is_empty() && !name.chars().all(|c| c.is_ascii_digit()) {
                names.push(name);
            }
        }
        names
    }

    /// Every log call in `source` that hands one of [`MEETING_TEXT_NAMES`] to
    /// the log, as `line: name`.
    fn meeting_text_in_log_calls(source: &str) -> Vec<String> {
        let tokens = tokenize(source);
        // A file may import the macros under other names:
        // `use log::{info as log_info}`, `use tracing::{info as log_info}`.
        let mut macros: Vec<String> = LOG_MACROS.iter().map(|name| name.to_string()).collect();
        for window in tokens.windows(3) {
            if let [(Token::Ident(base), _), (Token::Ident(word), _), (Token::Ident(alias), _)] = window {
                if word == "as" && LOG_MACROS.contains(&base.as_str()) {
                    macros.push(alias.clone());
                }
            }
        }
        let mut found = Vec::new();
        let mut i = 0;
        while i + 2 < tokens.len() {
            let is_call = matches!(&tokens[i].0, Token::Ident(name) if macros.contains(name))
                && tokens[i + 1].0 == Token::Punct('!')
                && tokens[i + 2].0 == Token::Punct('(');
            if !is_call {
                i += 1;
                continue;
            }
            let line = tokens[i].1;
            let mut depth = 0;
            let mut end = i + 2;
            while end < tokens.len() {
                match tokens[end].0 {
                    Token::Punct('(') => depth += 1,
                    Token::Punct(')') => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                end += 1;
            }
            let call = &tokens[i + 3..end.min(tokens.len())];
            let Some(format_at) = call.iter().position(|(t, _)| matches!(t, Token::Str(_))) else {
                i = end;
                continue;
            };
            if let Token::Str(format) = &call[format_at].0 {
                for name in inline_names(format) {
                    if MEETING_TEXT_NAMES.contains(&name.as_str()) {
                        found.push(format!("{line}: {{{name}}}"));
                    }
                }
            }
            let args = &call[format_at + 1..];
            for (k, (token, _)) in args.iter().enumerate() {
                let Token::Ident(name) = token else { continue };
                if !MEETING_TEXT_NAMES.contains(&name.as_str()) {
                    continue;
                }
                let after = args.get(k + 1).map(|(t, _)| t);
                let method = match args.get(k + 2) {
                    Some((Token::Ident(m), _)) if after == Some(&Token::Punct('.')) => m.as_str(),
                    _ => "",
                };
                // `.chars()` only as `.chars().count()`; `.chars().take(80)` is text.
                let counted = matches!(args.get(k + 6), Some((Token::Ident(m), _)) if m == "count");
                let size_only = SIZE_ONLY.contains(&method) && (method != "chars" || counted);
                // `reply.cut_off` is a field of the reply, judged by its own
                // name: `reply.text` is caught on `text`.
                let field = !method.is_empty() && args.get(k + 3).map(|(t, _)| t) != Some(&Token::Punct('('));
                let called = after == Some(&Token::Punct('('));
                let path = after == Some(&Token::Punct(':'));
                if !(size_only || field || called || path) {
                    found.push(format!("{line}: {name}"));
                }
            }
            i = end;
        }
        found
    }

    fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                rust_files(&path, into);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                into.push(path);
            }
        }
    }

    #[test]
    fn the_source_check_sees_a_leak_and_lets_a_length_through() {
        let leaky = r#"
            info!("Worker {} transcribed: {}", worker_id, transcript);
            log::debug!("result: '{}'", result.text);
            warn!("said {text}");
            perf_debug!("start: {}", text.chars().take(80).collect::<String>());
            info!("answer: {}", reply.text);
            info!("said {}", transcript.trim());
        "#;
        assert_eq!(
            meeting_text_in_log_calls(leaky),
            vec!["2: transcript", "3: text", "4: {text}", "5: text", "6: text", "7: transcript"]
        );

        // The macros under an imported name are log calls too.
        let aliased = r#"
            use log::{error as log_error, info as log_info};
            log_info!("meeting: {:?}", meeting_name);
            log_error!("search '{}' failed: {}", query, e);
        "#;
        assert_eq!(
            meeting_text_in_log_calls(aliased),
            vec!["3: meeting_name", "4: query"]
        );

        let clean = r#"
            // info!("commented out: {}", transcript);
            info!("Worker {} transcribed {} chars", worker_id, transcript.chars().count());
            info!("{}", report.summary(direction));
            let s = "info!(\"in a string\", text)";
            info!("summary {} chars, empty: {}", summary.len(), notes.is_empty());
            info!("cut off: {}", if reply.cut_off { "yes" } else { "no" });
        "#;
        assert!(meeting_text_in_log_calls(clean).is_empty());
    }

    #[test]
    fn no_log_call_passes_meeting_text() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        assert!(files.len() > 50, "found only {} source files", files.len());

        let mut leaks = Vec::new();
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let relative = file.strip_prefix(&src).unwrap().display().to_string();
            for leak in meeting_text_in_log_calls(&source) {
                leaks.push(format!("{relative}:{leak}"));
            }
        }
        assert!(
            leaks.is_empty(),
            "log calls that pass meeting text along (log its length instead):\n{}",
            leaks.join("\n")
        );
    }
}
