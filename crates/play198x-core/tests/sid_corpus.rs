#![cfg(feature = "sid")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Deterministic HVSC regression sweep. Media stays outside the repository.
//!
//! Extract HVSC #85, then run:
//! `PLAY198X_HVSC=/path/to/C64Music cargo test -p play198x-core --features sid --test sid_corpus -- --ignored --nocapture`

use play198x_core::player::sid::{SidPlayer, format::SidError};
use std::path::{Path, PathBuf};

const SAMPLE: usize = 5_000;

#[test]
#[ignore = "needs the local HVSC #85 corpus"]
fn callable_psid_sample_preserves_the_rom_free_baseline() {
    let root = std::env::var_os("PLAY198X_HVSC")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/private/tmp/play198x-hvsc85/C64Music"));
    let mut paths = Vec::new();
    collect_sid_paths(&root, &mut paths);
    paths.sort();

    let mut callable = 0usize;
    let mut rom_free = 0usize;
    let mut needs_rom = 0usize;
    let mut init_budget = 0usize;
    let mut parse_or_policy = 0usize;
    for path in paths.into_iter().take(SAMPLE) {
        let bytes = std::fs::read(&path).expect("HVSC entry should be readable");
        match SidPlayer::new(&bytes, 0, 8_000) {
            Ok(mut player) => {
                callable += 1;
                let mut outcome = Ok(());
                for _ in 0..10 {
                    outcome = player.frame();
                    if outcome.is_err() {
                        break;
                    }
                }
                match outcome {
                    Ok(()) => rom_free += 1,
                    Err(SidError::NeedsRom(_)) => needs_rom += 1,
                    Err(_) => init_budget += 1,
                }
            }
            Err(SidError::NeedsRom(_)) => {
                callable += 1;
                needs_rom += 1;
            }
            Err(SidError::InitDidNotReturn | SidError::PlayDidNotReturn) => {
                callable += 1;
                init_budget += 1;
            }
            Err(_) => parse_or_policy += 1,
        }
    }

    let clean = rom_free as f64 / callable.max(1) as f64;
    println!(
        "HVSC sample={SAMPLE} callable={callable} rom_free={rom_free} needs_rom={needs_rom} budget={init_budget} policy={parse_or_policy} clean={:.1}%",
        clean * 100.0
    );
    assert!(
        callable > 4_000,
        "the deterministic sample should be mostly callable PSID"
    );
    assert!(
        clean >= 0.84,
        "ROM-free rate fell below the 89% research baseline's regression band: {:.1}%",
        clean * 100.0
    );
}

fn collect_sid_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_sid_paths(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sid"))
        {
            out.push(path);
        }
    }
}
