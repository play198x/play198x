#![cfg(feature = "ay")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Runs the local World of Spectrum AY archive.
//!
//! Every other `ay` test proves the host runs against tunes this crate
//! wrote itself, checked frame-by-frame against a known-good answer. None of
//! that proves anything about a tune nobody here authored — this is the
//! first time any of it meets real files, and it exists to answer two
//! separate questions: how much of the archive plays at all, and how loud
//! the mix gets when it does (the AY chip's three channels already sum to
//! roughly full scale before the beeper is added, and there is no clipping
//! clamp yet — a decision deferred to the whole-branch review until this
//! sweep supplied the real numbers to decide it with).
//!
//! `#[ignore]`d: it needs the Time Capsule mounted, which CI does not have,
//! and it reads media that is never committed to this repository — the
//! archive is read from its mounted path, never copied in.
//!
//! Run with:
//!   cargo test -p play198x-core --features ay --test ay_corpus -- --ignored --nocapture

use std::path::{Path, PathBuf};

use play198x_core::container::Container;
use play198x_core::player::ay::AyPlayer;
use play198x_core::player::ay::format::AyError;

/// Where the World of Spectrum AY archive is mounted on this machine.
const ARCHIVE: &str = "/Volumes/Data/WOS-Archive/music/ay";

/// Sample rate `AyPlayer` is asked to render at. Arbitrary within reason —
/// 48kHz is the rate the rest of this crate's `ay` tests use.
const SAMPLE_RATE: u32 = 48_000;

/// How many 50Hz frames to run each tune before measuring its peak. 250
/// frames is 5 seconds: long enough to run past a loader-style intro and
/// into a tune's main loop, short enough that 696 files finish in a
/// reasonable time.
const FRAMES: u32 = 250;

/// A peak at or below this is judged inaudible. Not zero: a DC-blocked
/// beeper's settled residual, or a chip channel left at a very low volume,
/// can sit just above nothing without being a sound anyone would hear.
const AUDIBLE_THRESHOLD: f32 = 0.01;

/// Why a file failed to produce a playable [`AyPlayer`], broken down by
/// [`AyError`] variant rather than collapsed into one count.
///
/// A flat `failed` number cannot tell "this crate's parser is broken" from
/// "this tune installs its own interrupt handler and waits for a real Z80
/// `INT`, which slice 1 does not simulate by design" — both would show up
/// as `InitDidNotReturn` here, but only the breakdown lets a human tell
/// which is which. See `AyError`'s own doc for what each variant means.
#[derive(Default, Debug)]
struct Failures {
    not_an_ay_file: u32,
    truncated: u32,
    bad_pointer: u32,
    no_such_song: u32,
    init_did_not_return: u32,
}

impl Failures {
    fn record(&mut self, err: &AyError) {
        match err {
            AyError::NotAnAyFile => self.not_an_ay_file += 1,
            AyError::Truncated => self.truncated += 1,
            AyError::BadPointer => self.bad_pointer += 1,
            AyError::NoSuchSong => self.no_such_song += 1,
            AyError::InitDidNotReturn => self.init_did_not_return += 1,
        }
    }

    fn total(&self) -> u32 {
        self.not_an_ay_file
            + self.truncated
            + self.bad_pointer
            + self.no_such_song
            + self.init_did_not_return
    }
}

/// Peak-amplitude distribution across every tune that parsed, bucketed
/// rather than histogrammed — the mix-headroom question this sweep exists
/// to answer (see the module doc) only needs "how many are quiet, how many
/// sit near full scale, how many are already over it", not a fine curve.
/// `silent` doubles as this sweep's audible/silent split: a tune whose peak
/// never clears [`AUDIBLE_THRESHOLD`] lands here and nowhere else.
#[derive(Default, Debug)]
struct PeakBuckets {
    /// <= AUDIBLE_THRESHOLD: no audible output in the frames sampled.
    silent: u32,
    /// (AUDIBLE_THRESHOLD, 0.25]
    quiet: u32,
    /// (0.25, 0.6]
    moderate: u32,
    /// (0.6, 1.0]: at or under full scale.
    full: u32,
    /// (1.0, 1.5]: over full scale, but not by a wide margin.
    hot: u32,
    /// > 1.5: badly over full scale.
    clipping: u32,
}

impl PeakBuckets {
    fn record(&mut self, peak: f32) {
        if peak <= AUDIBLE_THRESHOLD {
            self.silent += 1;
        } else if peak <= 0.25 {
            self.quiet += 1;
        } else if peak <= 0.6 {
            self.moderate += 1;
        } else if peak <= 1.0 {
            self.full += 1;
        } else if peak <= 1.5 {
            self.hot += 1;
        } else {
            self.clipping += 1;
        }
    }

    fn over_one(&self) -> u32 {
        self.hot + self.clipping
    }
}

#[test]
#[ignore = "needs the local archive"]
fn the_local_archive_plays() {
    let root = PathBuf::from(ARCHIVE);
    if !root.exists() {
        eprintln!("archive not mounted at {ARCHIVE}; nothing measured");
        return;
    }

    let mut parsed = 0u32;
    let mut failures = Failures::default();
    let mut peaks = PeakBuckets::default();
    // The mix-headroom question this sweep exists to settle is specifically
    // about a tune driving the AY chip and the beeper *together* — that is
    // the case with no headroom margin at all, since the chip alone already
    // sums close to full scale. Beeper-only and chip-only tunes are each
    // interesting on their own but do not bear on that question the same
    // way, so all four combinations are tallied rather than just "both".
    let mut beeper_only = 0u32;
    let mut ay_only = 0u32;
    let mut both = 0u32;
    let mut neither = 0u32;

    let samples_per_frame = (SAMPLE_RATE / 50) as usize;
    let mut out = vec![0.0f32; samples_per_frame * 2];

    for bytes in ay_files(&root) {
        match AyPlayer::new(&bytes, 0, SAMPLE_RATE) {
            Err(err) => failures.record(&err),
            Ok(mut player) => {
                parsed += 1;
                let mut peak = 0.0f32;
                for _ in 0..FRAMES {
                    player.frame();
                    player.render(&mut out);
                    for sample in &out {
                        peak = peak.max(sample.abs());
                    }
                }
                peaks.record(peak);
                match (player.host.speaker_written, player.host.ay_written) {
                    (true, true) => both += 1,
                    (true, false) => beeper_only += 1,
                    (false, true) => ay_only += 1,
                    (false, false) => neither += 1,
                }
            }
        }
    }

    let audible = parsed - peaks.silent;

    println!("--- ay corpus sweep: {ARCHIVE} ---");
    println!("parsed {parsed}  failed {}", failures.total());
    println!("failure breakdown: {failures:?}");
    println!("audible {audible}  silent {}", peaks.silent);
    println!("peak distribution: {peaks:?}");
    println!(
        "peaks over 1.0 (no clipping clamp exists yet): {}/{parsed}",
        peaks.over_one()
    );
    println!(
        "sound source: beeper-only {beeper_only}  ay-only {ay_only}  both {both}  neither {neither}"
    );

    assert!(parsed > 0, "no .ay files were found under {ARCHIVE}");

    // Measured against the real archive on 2026-08-29 (696 files total; see
    // task-8-report.md for the full breakdown this run produced):
    //   parsed 551  failed 145 (143 InitDidNotReturn, 2 BadPointer)
    //   audible 513  silent 38  -> 513/551 = 93.1%
    // The 143 `InitDidNotReturn` failures are expected, not a host bug: they
    // are tunes whose interrupt routine waits on a real Z80 `INT` this
    // slice's stub player never raises (see the plan's scope note on the
    // stub) — out of slice 1 by design. The 2 `BadPointer` are worth a
    // one-off look but are too few to move this bar.
    //
    // The bar below sits at 85%, comfortably under the measured 93.1%, so a
    // real regression in host correctness has room to show up before the
    // bar does, without the bar itself being so tight that ordinary
    // archive noise (a handful more real INT-waiting tunes, say) trips it.
    assert!(
        audible * 100 >= parsed * 85,
        "fewer than 85% of the parsed archive made a sound: {audible}/{parsed}"
    );
}

/// Every `.ay` file under `root`, read directly from disk, plus every `.ay`
/// entry inside an `.ay.zip` sibling, read through this crate's own
/// container reader rather than a second unzipper — the same path a real
/// caller opening one of these archives would take.
fn ay_files(root: &Path) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let lower = path.to_string_lossy().to_ascii_lowercase();
            if lower.ends_with(".ay") {
                if let Ok(bytes) = std::fs::read(&path) {
                    out.push(bytes);
                }
            } else if lower.ends_with(".zip") {
                let Ok(container) = Container::open(&path) else {
                    continue;
                };
                let Ok(zip_entries) = container.entries() else {
                    continue;
                };
                for zip_entry in zip_entries {
                    // `Entry::path` is documented as not sanitised: a
                    // hostile archive can name an entry anything at all.
                    // Safe here because it is only ever handed back to
                    // `read`, never joined onto a directory and never
                    // written anywhere.
                    if zip_entry.path.to_ascii_lowercase().ends_with(".ay")
                        && let Ok(inner) = container.read(&zip_entry.path)
                    {
                        out.push(inner);
                    }
                }
            }
        }
    }
    out
}
