//! Whether the application may send anything to a machine other than this one.
//!
//! On by default: summaries, the live assistant and speech recognition talk
//! only to addresses on this computer (a local Ollama, a local
//! OpenAI-compatible server). A cloud provider or a remote endpoint is refused
//! before the request is built, so a transcript cannot leave by a setting
//! chosen by mistake or left over from before. Turning it off is a deliberate
//! act in Settings.
//!
//! Downloading a model the owner asked for is not covered: it sends nothing
//! about the archive, and each file is checked against a pinned hash
//! (`crate::model_integrity`).

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const FILE: &str = "network_policy.json";

static LOCAL_ONLY: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Serialize, Deserialize)]
struct Stored {
    local_only: bool,
}

fn path() -> PathBuf {
    crate::paths::install_data_root().join(FILE)
}

/// Reads the stored choice. Missing or unreadable means local-only.
pub fn load() {
    let local_only = std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
        .map(|stored| stored.local_only)
        .unwrap_or(true);
    LOCAL_ONLY.store(local_only, Ordering::SeqCst);
    log::info!("Network policy: local only = {local_only}");
}

pub fn local_only() -> bool {
    LOCAL_ONLY.load(Ordering::SeqCst)
}

/// Whether `url` points at this computer.
pub fn is_loopback_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url.trim()) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

/// Refuses `url` when local-only mode is on and it is not on this computer.
pub fn check(url: &str) -> Result<(), String> {
    decide(local_only(), url)
}

fn decide(local_only: bool, url: &str) -> Result<(), String> {
    if !local_only || is_loopback_url(url) {
        return Ok(());
    }
    let host = url::Url::parse(url.trim())
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_string))
        .unwrap_or_else(|| "an unknown address".to_string());
    Err(format!(
        "Local-only mode is on: the request to {host} was not sent. \
         Use a local model, or turn local-only mode off in Settings → Protection."
    ))
}

#[tauri::command]
pub fn get_local_only_mode() -> bool {
    local_only()
}

#[tauri::command]
pub fn set_local_only_mode(enabled: bool) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&Stored { local_only: enabled })
        .map_err(|error| error.to_string())?;
    std::fs::write(path(), text).map_err(|error| format!("Could not save the setting: {error}"))?;
    LOCAL_ONLY.store(enabled, Ordering::SeqCst);
    log::info!("Network policy changed: local only = {enabled}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_computer_is_recognised_in_every_spelling() {
        for url in [
            "http://localhost:11434",
            "http://LOCALHOST:8080/v1",
            "http://127.0.0.1:1234",
            "http://127.1.2.3",
            "http://[::1]:8000/v1",
        ] {
            assert!(is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn anything_else_is_not() {
        for url in [
            "https://api.openai.com/v1/models",
            "http://192.168.1.10:11434",
            "http://localhost.example.com",
            "http://0.0.0.0:11434",
            "not a url",
            "",
        ] {
            assert!(!is_loopback_url(url), "{url}");
        }
    }

    #[test]
    fn local_only_refuses_remote_and_lets_local_through() {
        assert!(decide(true, "http://localhost:11434/api/generate").is_ok());
        let refused = decide(true, "https://api.anthropic.com/v1/messages").unwrap_err();
        assert!(refused.contains("api.anthropic.com"));
        assert!(decide(false, "https://api.anthropic.com/v1/messages").is_ok());
    }
}
