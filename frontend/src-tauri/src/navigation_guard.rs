//! Keeps the app's windows on the app's own pages.
//!
//! A page loaded into a window is fetched by the webview itself, past
//! `network_policy`, and the windows are frameless, so a foreign page shown in
//! one has neither a title nor a way back but Alt+Left. The pages render model
//! answers as text rather than links (`frontend/src/lib/model-markdown.ts`);
//! this is the backstop for anything else that would navigate a window away:
//! every navigation to an address that is not the app's own is refused.
//!
//! Links the app opens on purpose ("About", licences) go to the system browser
//! through `open_external_url` and never navigate a window, so they are not
//! affected.

use tauri::plugin::TauriPlugin;
use tauri::{Manager, Runtime, Url};

/// The plugin that refuses navigation away from the app, for every window.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    tauri::plugin::Builder::new("navigation-guard")
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
        "tauri" => url.host_str() == Some("localhost"),
        // The built frontend on Windows and Android.
        "http" | "https" if url.host_str() == Some("tauri.localhost") => true,
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
        assert!(is_app_page(&url("tauri://localhost/person"), None));
        assert!(is_app_page(&url("about:blank"), None));
    }

    #[test]
    fn a_foreign_page_is_refused() {
        assert!(!is_app_page(&url("http://127.0.0.1:8765/leak?q=Sekretnaya-fraza"), None));
        assert!(!is_app_page(&url("https://example.com/"), None));
        assert!(!is_app_page(&url("http://tauri.localhost.example.com/"), None));
        assert!(!is_app_page(&url("tauri://example.com/"), None));
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
}
