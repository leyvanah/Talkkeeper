// Refuse a Windows helper that would load the C runtime from System32.
//
// The static runtime is set in .cargo/config.toml, which cargo reads only when
// it runs inside llama-helper/. A build started elsewhere (`cargo build -p
// llama-helper` from the workspace root) or with RUSTFLAGS set in the
// environment silently gets the dynamic runtime, and the helper then crashes
// on machines with an old one. Stop such a build instead of shipping it.

use std::env;

fn main() {
    println!("cargo:rerun-if-env-changed=LLAMA_STATIC_CRT");

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }

    let crt_static = env::var("CARGO_CFG_TARGET_FEATURE")
        .unwrap_or_default()
        .split(',')
        .any(|feature| feature == "crt-static");
    let llama_static = env::var("LLAMA_STATIC_CRT").as_deref() == Ok("1");

    if !(crt_static && llama_static) {
        panic!(
            "llama-helper must link the C runtime statically on Windows \
             (see llama-helper/.cargo/config.toml). Build it from inside \
             llama-helper/, without RUSTFLAGS in the environment: \
             `cd llama-helper && cargo build --release`."
        );
    }
}
