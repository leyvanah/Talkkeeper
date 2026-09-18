#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]


fn main() {
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }
    // stderr as before, plus a file — a release build on Windows has no
    // console, so without the file a bad run leaves nothing to read.
    app_lib::logging::init(app_lib::paths::logs_dir());

    // Async logger will be initialized lazily when first needed (after Tauri runtime starts)
    log::info!("Starting application...");
    app_lib::run();
}
