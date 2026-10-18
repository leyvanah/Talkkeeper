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
//!
//! Every HTTP client of the application is made here ([`client`],
//! [`client_builder`], [`blocking_client`]), because the check alone is not
//! enough: `reqwest` takes the Windows system proxy (and `HTTP_PROXY`) on its
//! own and ignores the system's "bypass for local addresses" list. A request
//! to a local Ollama would pass the check — the address is local — and then
//! go to the proxy with the meeting text in it. A request to this computer
//! therefore never takes a proxy; anything else keeps the system's, which
//! model downloads behind a company proxy depend on.

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

/// A client builder for requests to `url`: without any proxy when `url` is on
/// this computer, with the system's otherwise. For callers that need their
/// own timeouts; the rest take [`client`].
pub fn client_builder(url: &str) -> reqwest::ClientBuilder {
    let builder = reqwest::Client::builder();
    if is_loopback_url(url) {
        builder.no_proxy()
    } else {
        builder
    }
}

/// The client for requests to `url` (see [`client_builder`]). Made fresh each
/// time, as the `reqwest::Client::new()` calls it replaces were: a shared one
/// would carry pooled connections across async runtimes.
pub fn client(url: &str) -> reqwest::Client {
    client_builder(url)
        .build()
        // What `reqwest::Client::new` does too: without a TLS backend there is
        // no client to fall back on.
        .expect("the HTTP client could not be created")
}

/// The blocking counterpart of [`client`]. Made fresh each time: a blocking
/// client must not be dropped inside the async runtime, so it is not kept.
pub fn blocking_client(url: &str) -> Result<reqwest::blocking::Client, String> {
    let builder = reqwest::blocking::Client::builder();
    let builder = if is_loopback_url(url) {
        builder.no_proxy()
    } else {
        builder
    };
    builder
        .build()
        .map_err(|error| format!("Failed to create HTTP client: {error}"))
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

    /// Every HTTP client is made in this file; one made anywhere else would
    /// take the system proxy for a local request again.
    #[test]
    fn no_http_client_is_made_outside_this_module() {
        fn rust_files(dir: &std::path::Path, into: &mut Vec<PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    rust_files(&path, into);
                } else if path.extension().is_some_and(|ext| ext == "rs") {
                    into.push(path);
                }
            }
        }
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        assert!(files.len() > 50, "found only {} source files", files.len());

        let constructors = ["Client::new(", "Client::builder(", "ClientBuilder::new(", "reqwest::get("];
        let mut found = Vec::new();
        for file in files {
            if file.file_name().is_some_and(|name| name == "network_policy.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&file).unwrap();
            for (number, line) in source.lines().enumerate() {
                let code = line.split("//").next().unwrap_or("");
                for constructor in constructors {
                    let mut from = 0;
                    while let Some(at) = code[from..].find(constructor) {
                        let at = from + at;
                        // `Client::new(` but not `SidecarClient::new(`.
                        let before = code[..at].chars().last();
                        if !before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            found.push(format!(
                                "{}:{}: {}",
                                file.strip_prefix(&src).unwrap().display(),
                                number + 1,
                                line.trim()
                            ));
                        }
                        from = at + constructor.len();
                    }
                }
            }
        }
        assert!(
            found.is_empty(),
            "make HTTP clients with network_policy::client / client_builder / blocking_client:\n{}",
            found.join("\n")
        );
    }

    /// A listener on this computer that answers every request with an empty
    /// 200 and counts what reached it.
    fn counting_server() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        (address, hits)
    }

    #[test]
    fn a_request_to_this_computer_never_goes_through_a_proxy() {
        let (target, target_hits) = counting_server();
        let (proxy, proxy_hits) = counting_server();
        let local_url = format!("http://{target}/api/tags");

        // A proxy as a company network, a VPN client or a debugging tool
        // would set it. `HTTP_PROXY` is what reqwest reads first, ahead of
        // the Windows setting; the address-blind handling is the same.
        let saved: Vec<(&str, Option<std::ffi::OsString>)> = ["HTTP_PROXY", "NO_PROXY", "no_proxy"]
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect();
        std::env::set_var("HTTP_PROXY", format!("http://{proxy}"));
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");

        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            // What every client in the application did before: the local
            // request goes to the proxy. This also proves the trap is armed.
            reqwest::Client::builder()
                .build()
                .unwrap()
                .get(&local_url)
                .send()
                .await
                .unwrap();
            assert_eq!(proxy_hits.load(Ordering::SeqCst), 1, "the trap did not catch a plain client");
            assert_eq!(target_hits.load(Ordering::SeqCst), 0);

            // Ours goes straight to this computer.
            client(&local_url).get(&local_url).send().await.unwrap();
            assert_eq!(target_hits.load(Ordering::SeqCst), 1);
            assert_eq!(proxy_hits.load(Ordering::SeqCst), 1);

            // A remote address keeps the system's proxy, which downloads
            // behind a company proxy need. Nothing leaves: the trap answers.
            let remote = "http://models.example.invalid/file.bin";
            client(remote).get(remote).send().await.unwrap();
            assert_eq!(proxy_hits.load(Ordering::SeqCst), 2);
        });
        drop(runtime);

        // The blocking client, used outside any async runtime.
        blocking_client(&local_url)
            .unwrap()
            .get(&local_url)
            .send()
            .unwrap();
        assert_eq!(target_hits.load(Ordering::SeqCst), 2);
        assert_eq!(proxy_hits.load(Ordering::SeqCst), 2);

        for (name, value) in saved {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
    }
}
