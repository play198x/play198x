//! Guards the property `ModulePlayer::render`'s doc comment promises but the
//! Rust-side tests in `tests/boundary.rs` cannot see: that calling it from
//! JavaScript allocates nothing.
//!
//! `wasm-pack test --node` compiles this crate's `#[wasm_bindgen_test]`s into
//! the *same* wasm module as the library and calls `ModulePlayer`'s methods
//! directly, Rust to Rust — it never crosses the JS<->wasm boundary a real
//! `AudioWorkletProcessor` calls through, so it cannot see what
//! wasm-bindgen's *generated* glue does at that boundary. A first version of
//! `render` took `&mut [f32]`, which read as allocation-free in Rust while
//! its generated JS glue called `wasm.__wbindgen_malloc` on every
//! invocation — a malloc, a copy in, a copy back and a free, every ~2.7 ms,
//! on the audio-rendering thread. None of the tests one level down would
//! have failed on that version. This one builds the real glue and reads it,
//! so a future change that reintroduces a malloc on `render`'s path fails a
//! test instead of shipping quietly.
//!
//! This is a plain `#[test]`, not `#[wasm_bindgen_test]`: it shells out to
//! `wasm-pack` and reads a file from disk, neither of which is available
//! inside the wasm32 sandbox `wasm-pack test --node` runs in. Run it the
//! ordinary way instead — `cargo test --manifest-path
//! crates/play198x-web/Cargo.toml` — which is the wrong tool for this
//! crate's `#[wasm_bindgen_test]`s (it silently runs zero of them) but the
//! right one for a test that inspects a build artifact rather than wasm
//! behaviour.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;
use std::process::Command;

#[test]
fn the_render_path_never_calls_wbindgen_malloc() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    // `--dev` skips wasm-opt: this test wants the same glue a real build
    // emits, not an optimized one, and skipping the optimizer is the
    // difference between this test taking a second and taking ten. Written
    // under this crate's own `target/`, already gitignored and distinct
    // from the `pkg-web`/`pkg-node` directories this crate actually ships
    // from — this is scratch output for one test, not a release artifact.
    let out_dir = Path::new(manifest_dir).join("target/glue-guard-pkg");

    let status = Command::new("wasm-pack")
        .args(["build", "--dev", "--target", "web"])
        .arg("--out-dir")
        .arg(&out_dir)
        .arg(manifest_dir)
        // `RUSTUP_TOOLCHAIN`, if set in the environment running this test,
        // overrides `rust-toolchain.toml` — stripped so the subprocess
        // honours the pin (1.98) regardless of how this test itself was
        // invoked. See this crate's own toolchain constraint.
        .env_remove("RUSTUP_TOOLCHAIN")
        .status()
        .expect("wasm-pack must be on PATH to run this guard");
    assert!(status.success(), "wasm-pack build failed");

    let glue = std::fs::read_to_string(out_dir.join("play198x_web.js"))
        .expect("wasm-pack build did not produce play198x_web.js");

    let render_fn = extract_method(&glue, "render(frames)").expect(
        "generated glue has no `render(frames)` method on ModulePlayer — \
         has render's signature changed? Update this guard's search string \
         to match",
    );

    assert!(
        !render_fn.contains("__wbindgen_malloc"),
        "ModulePlayer.render's generated JS glue allocates on every call — \
         this is the exact per-call malloc/copy/copy-back/free the \
         allocation-free design exists to avoid. Generated body:\n{render_fn}"
    );
}

/// Slice out one method's body from wasm-bindgen's generated JS class: from
/// `signature` (e.g. `"render(frames)"`) to the closing brace that matches
/// its opening one. A whole-file search for `__wbindgen_malloc` would also
/// trip on `ModulePlayer`'s constructor, which legitimately mallocs once to
/// copy the module's bytes in — that one-time cost at track load is not the
/// property this test is about, and a bare substring search could not tell
/// the two apart.
fn extract_method<'a>(source: &'a str, signature: &str) -> Option<&'a str> {
    let start = source.find(signature)?;
    let body_start = source[start..].find('{')? + start;
    let mut depth = 0i32;
    for (offset, ch) in source[body_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[start..=body_start + offset]);
                }
            }
            _ => {}
        }
    }
    None
}
