//! Keeps the app's windows on the app's own pages.
//!
//! A page loaded into a window is fetched by the webview itself, past
//! `network_policy`, and the windows are frameless, so a foreign page shown in
//! one has neither a title nor a way back but Alt+Left. The pages render model
//! answers as text rather than links (`frontend/src/lib/model-markdown.ts`);
//! this is the backstop for anything else that would navigate a window away:
//! every navigation to an address that is not the app's own is refused, so the
//! window stays on the app's page.
//!
//! The refusal alone cancels a navigation that has already started, and by
//! then WebView2 may have sent the request to the foreign address. So on
//! Windows every request a window makes passes a filter first (see
//! [`may_request`]): one to an address that is not the app's own is answered
//! inside the webview with an empty refusal and never reaches the network —
//! a navigation, an image, a `fetch`, a frame alike, in the built app and in a
//! development build, whatever the "This computer only" setting says. The
//! window has no business on the network in either mode: cloud providers are
//! called from Rust, through `network_policy`.
//!
//! Links the app opens on purpose ("About", licences) go to the system browser
//! through `open_external_url` and never navigate a window, so they are not
//! affected.

use tauri::plugin::TauriPlugin;
use tauri::{Manager, Runtime, Url};

/// The plugin that keeps every window from navigating away from the app, and
/// on Windows keeps its requests to foreign addresses from being sent.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
        .on_webview_ready(|webview| {
            #[cfg(windows)]
            request_filter::install(&webview);
            #[cfg(not(windows))]
            let _ = webview;
        })
        .on_navigation(|webview, url| {
            let dev_url = if tauri::is_dev() {
                webview.app_handle().config().build.dev_url.clone()
            } else {
                None
            };
            let allowed = is_app_page(url, dev_url.as_ref());
            if !allowed {
                // Only the scheme: the address may carry text from a meeting.
                log::warn!(
                    "Refused to navigate window '{}' away from the app ({} address)",
                    webview.label(),
                    url.scheme()
                );
            }
            allowed
        })
        .build()
}

/// Whether `url` is one of the app's own pages: the built frontend, or in a
/// development build the dev server it was started with.
pub fn is_app_page(url: &Url, dev_url: Option<&Url>) -> bool {
    match url.scheme() {
        // The built frontend on macOS and Linux.
        "tauri" => url.host_str() == Some("localhost") && url.port().is_none(),
        // The built frontend on Windows and Android, served only at the
        // default port. Any `*.localhost` name resolves to loopback, so
        // `tauri.localhost:<port>` is whatever local service listens there.
        // (`port()` is also None for an explicit default port.)
        "http" | "https" if url.host_str() == Some("tauri.localhost") => url.port().is_none(),
        "http" | "https" => dev_url.is_some_and(|dev| same_origin(url, dev)),
        // An empty page, such as the one the recovery code is printed from.
        "about" => url.path() == "blank",
        _ => false,
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme()
        && a.host_str() == b.host_str()
        && a.port_or_known_default() == b.port_or_known_default()
}

/// The names the app serves itself under on Windows: its pages, the IPC
/// channel to the backend, and the recordings for the player
/// (`audio::recording_protocol`). Answered inside the webview, never by the
/// network — but only at the default port, since any `*.localhost` name with
/// a port is whatever local service listens there.
const OWN_HOSTS: [&str; 3] = ["tauri.localhost", "ipc.localhost", "recording.localhost"];

/// Whether a window may send a request for `address` at all: one of the app's
/// own addresses, the dev server in a development build, or something that
/// never leaves the process (`data:`, `blob:`, `about:`).
///
/// Everything else is refused, including `file:` — a `file://host/share`
/// address is a network share — and anything that does not parse.
pub fn may_request(address: &str, dev_url: Option<&Url>) -> bool {
    let Ok(url) = Url::parse(address) else {
        return false;
    };
    match url.scheme() {
        "data" | "blob" | "about" => true,
        "tauri" => url.host_str() == Some("localhost") && url.port().is_none(),
        "http" | "https" | "ws" | "wss" => {
            let own = url
                .host_str()
                .is_some_and(|host| OWN_HOSTS.contains(&host))
                && url.port().is_none();
            own || dev_url.is_some_and(|dev| same_host_and_port(&url, dev))
        }
        _ => false,
    }
}

/// The dev server's own origin, also over its websocket (hot reload), which
/// uses `ws:` on the same host and port.
fn same_host_and_port(url: &Url, dev: &Url) -> bool {
    let secure = |url: &Url| matches!(url.scheme(), "https" | "wss");
    secure(url) == secure(dev)
        && url.host_str() == dev.host_str()
        && url.port_or_known_default() == dev.port_or_known_default()
}

/// The WebView2 side of the filter: every request a window makes is offered
/// to [`may_request`] before it is sent, and a refused one gets an empty 403
/// from inside the webview.
#[cfg(windows)]
mod request_filter {
    use super::may_request;
    use tauri::{Manager, Runtime, Webview};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2WebResourceRequestedEventArgs, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
    };
    use webview2_com::{take_pwstr, WebResourceRequestedEventHandler};
    use windows::core::{HSTRING, PWSTR};

    pub fn install<R: Runtime>(webview: &Webview<R>) {
        let label = webview.label().to_string();
        let dev_url = if tauri::is_dev() {
            webview.app_handle().config().build.dev_url.clone()
        } else {
            None
        };
        let installed = webview.with_webview(move |platform| {
            // SAFETY: this runs on the webview's own thread, which is where
            // WebView2 objects may be used; the handler is called there too.
            let result = unsafe {
                let environment = platform.environment();
                platform.controller().CoreWebView2().and_then(|core| {
                    // Every address, every kind of resource: the main page, a
                    // frame, an image, a `fetch`. The app's own pages pass
                    // through this filter as well, and are handed on.
                    core.AddWebResourceRequestedFilter(
                        &HSTRING::from("*"),
                        COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                    )?;
                    let mut token = 0i64;
                    let handler_label = label.clone();
                    core.add_WebResourceRequested(
                        &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                            let Some(args) = args else {
                                return Ok(());
                            };
                            if !may_send(&args, dev_url.as_ref())? {
                                // Only the label: the address may carry text
                                // from a meeting.
                                log::warn!(
                                    "Refused a request from window '{handler_label}' to an address outside the app"
                                );
                                let refusal = environment.CreateWebResourceResponse(
                                    None,
                                    403,
                                    &HSTRING::from("Forbidden"),
                                    &HSTRING::new(),
                                )?;
                                args.SetResponse(&refusal)?;
                            }
                            Ok(())
                        })),
                        &mut token,
                    )
                })
            };
            if let Err(error) = result {
                log::error!("Could not filter the requests of window '{label}': {error}");
            }
        });
        if let Err(error) = installed {
            log::error!("Could not reach the webview to filter its requests: {error}");
        }
    }

    unsafe fn may_send(
        args: &ICoreWebView2WebResourceRequestedEventArgs,
        dev_url: Option<&tauri::Url>,
    ) -> windows::core::Result<bool> {
        let request = args.Request()?;
        let mut uri = PWSTR::null();
        request.Uri(&mut uri)?;
        Ok(may_request(&take_pwstr(uri), dev_url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn the_built_frontend_is_allowed() {
        assert!(is_app_page(&url("http://tauri.localhost/meeting-details?id=1"), None));
        assert!(is_app_page(&url("https://tauri.localhost/"), None));
        assert!(is_app_page(&url("http://tauri.localhost:80/"), None));
        assert!(is_app_page(&url("tauri://localhost/person"), None));
        assert!(is_app_page(&url("about:blank"), None));
    }

    #[test]
    fn a_foreign_page_is_refused() {
        assert!(!is_app_page(&url("http://127.0.0.1:8765/leak?q=Sekretnaya-fraza"), None));
        assert!(!is_app_page(&url("https://example.com/"), None));
        assert!(!is_app_page(&url("http://tauri.localhost.example.com/"), None));
        assert!(!is_app_page(&url("tauri://example.com/"), None));
        // Another port on the app's own name is a local service, not the app.
        assert!(!is_app_page(&url("http://tauri.localhost:8765/leak?q=Sekretnaya-fraza"), None));
        assert!(!is_app_page(&url("https://tauri.localhost:8765/"), None));
        assert!(!is_app_page(&url("http://tauri.localhost:443/"), None));
        assert!(!is_app_page(&url("tauri://localhost:8765/"), None));
        // Other `*.localhost` names are loopback too.
        assert!(!is_app_page(&url("http://recording.localhost/"), None));
        assert!(!is_app_page(&url("http://evil.tauri.localhost/"), None));
        assert!(!is_app_page(&url("http://localhost/"), None));
        assert!(!is_app_page(&url("file:///C:/Windows/win.ini"), None));
        assert!(!is_app_page(&url("data:text/html,hello"), None));
        assert!(!is_app_page(&url("about:srcdoc"), None));
    }

    #[test]
    fn the_dev_server_only_in_a_development_build() {
        let dev = url("http://localhost:3118");
        assert!(is_app_page(&url("http://localhost:3118/settings"), Some(&dev)));
        assert!(!is_app_page(&url("http://localhost:3118/settings"), None));
        assert!(!is_app_page(&url("http://localhost:8765/"), Some(&dev)));
        assert!(!is_app_page(&url("https://localhost:3118/"), Some(&dev)));
        assert!(!is_app_page(&url("http://127.0.0.1:3118/"), Some(&dev)));
    }

    #[test]
    fn a_window_may_reach_the_app_itself() {
        for own in [
            "http://tauri.localhost/meeting-details?id=1",
            "https://tauri.localhost/_next/static/chunk.js",
            "http://ipc.localhost/plugin%3Astore%7Cget",
            "http://recording.localhost/C%3A%2Frec-1%2Faudio.mp4",
            "tauri://localhost/",
            "data:image/png;base64,AAAA",
            "blob:http://tauri.localhost/0b1c",
            "about:blank",
        ] {
            assert!(may_request(own, None), "{own}");
        }
    }

    #[test]
    fn a_window_may_not_reach_anything_else() {
        for foreign in [
            "http://127.0.0.1:8765/nav",
            "https://example.com/pixel.png",
            "wss://example.com/socket",
            "http://localhost:11434/api/generate",
            "http://tauri.localhost:8765/",
            "http://recording.localhost:8080/",
            "http://evil.tauri.localhost/",
            "http://tauri.localhost.example.com/",
            "file://host/share/archive.zip",
            "file:///C:/Windows/win.ini",
            "ftp://example.com/",
            "not an address",
            "",
        ] {
            assert!(!may_request(foreign, None), "{foreign}");
        }
    }

    #[test]
    fn the_dev_server_may_be_reached_only_in_a_development_build() {
        let dev = url("http://localhost:3118");
        assert!(may_request("http://localhost:3118/settings", Some(&dev)));
        assert!(may_request("ws://localhost:3118/_next/webpack-hmr", Some(&dev)));
        assert!(!may_request("http://localhost:3118/settings", None));
        assert!(!may_request("http://localhost:8765/", Some(&dev)));
        assert!(!may_request("https://localhost:3118/", Some(&dev)));
        assert!(!may_request("http://127.0.0.1:3118/", Some(&dev)));
    }
}
