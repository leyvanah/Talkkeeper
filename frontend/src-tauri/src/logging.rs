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
//! dump cannot quietly fill the file with somebody's words.
//!
//! stderr keeps behaving exactly as before, `RUST_LOG` included: the terminal
//! logger is the same `env_logger` instance, and this type only adds a second
//! destination.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

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
}

/// Writes every record to the log file and hands the ones stderr wants to
/// `env_logger`, so a terminal run looks the same as it always did.
struct TeeLogger {
    terminal: env_logger::Logger,
    file: Mutex<FileSink>,
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
    let logger = TeeLogger {
        terminal,
        file: Mutex::new(sink),
        file_level,
    };

    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(max_level);
        log::info!("Log file: {}", log_dir.join(LOG_FILE_NAME).display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
