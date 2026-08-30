#![cfg(feature = "ay")]
//! Runs the local World of Spectrum AY archive.
//!
//! Every other `ay` test proves the host runs against tunes this crate
//! wrote itself, checked frame-by-frame against a known-good answer. None of
//! that proves anything about a tune nobody here authored — this is the
//! first time any of it meets real files, and it exists to answer two
//! separate questions: how much of the archive plays at all, and how loud
//! the mix gets when it does (the AY chip's three channels already sum to
//! roughly full scale before the beeper is added, so the headroom left for
//! the beeper on top is a measurement and not an assumption; `render`'s
//! clamp is a backstop behind that, and the figures below are taken before
//! it so they can see past it).
//!
//! This sweep runs two passes over the same archive. The first plays only
//! song 0 of each file (`AyPlayer::new(&bytes, 0, ...)`) — cheap, and kept
//! as the sweep's original figures so a regression against the measured
//! baselines in its assertions stays visible on its own. The second plays
//! *every* song of every file: the archive holds 1,915 songs across these
//! 696 files and 278 of them carry more than one song, so a song-0-only
//! sweep never exercised 1,219 songs — 63.7% of the archive's total tune
//! count. That is not a hypothetical gap: it is exactly what let a
//! `HiReg`/`LoReg` byte-swap bug survive four separate reviews, because
//! song 0 is *usually* the one case where the two register halves hold the
//! same byte (a subtune index of 0 in both) and so is blind to which
//! half is which. Counted rather than assumed: **74 of the 696 files have
//! song 0 with `byte@+8 != byte@+9`**, so 622 were blind to the field
//! order and 74 were not — the swap was a no-op for the 622 and wrong
//! output for the 74, and playing every song is what finally exercises the
//! other 1,219.
//!
//! What a song-0-only sweep still could not see is field order *as such* —
//! a swap shows up here as "N tunes went quiet", which has many other
//! explanations. The instrument for that is `tests/ay_format.rs`'s literal
//! byte array; this sweep stays a coverage and headroom measurement, not a
//! correctness proof for any one field.
//!
//! The all-songs pass also measures the two pieces of port traffic that
//! separate a 128K host from a 64 KB one, because both are things a sweep
//! can see and a unit test cannot: how much of a real archive pages memory,
//! and how much of it reads the sound chip back. Only that pass — the
//! song-0 figures above it are held byte-for-byte against their own measured
//! baseline, and adding counters to them would say nothing a pass over every
//! song does not say better.
//! - Writes that reach the 128K paging latch, which moves the RAM bank at
//!   0xC000 (`SpectrumHost::paging_written`, `paging_values_seen`). The
//!   tunes that do it are listed by name, not just counted: they are the
//!   group any change to the memory model is aimed at, so they have to be
//!   comparable tune by tune between runs.
//! - Reads of any port, and of the AY's register-read port in particular,
//!   which the chip answers (`AyPlayer::any_port_read`, `ay_read`,
//!   `ay_reads_non_ff`).
//!
//! `#[ignore]`d: it needs the Time Capsule mounted, which CI does not have,
//! and it reads media that is never committed to this repository — the
//! archive is read from its mounted path, never copied in.
//!
//! Run with:
//!   cargo test -p play198x-core --features ay --test ay_corpus -- --ignored --nocapture

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use play198x_core::container::Container;
use play198x_core::player::ay::AyPlayer;
use play198x_core::player::ay::format::{self, AyError};

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
/// "this tune's init routine never returns", and even the breakdown by
/// variant only gets you as far as [`AyError::InitDidNotReturn`] — telling
/// *why* an init routine didn't return needs more than a counter can carry,
/// see the comment beside this sweep's audible-share assertion for what was
/// actually established about that.
#[derive(Default, Debug)]
struct Failures {
    not_an_ay_file: u32,
    truncated: u32,
    bad_pointer: u32,
    no_such_song: u32,
    init_did_not_return: u32,
    too_large: u32,
}

impl Failures {
    fn record(&mut self, err: &AyError) {
        match err {
            AyError::NotAnAyFile => self.not_an_ay_file += 1,
            AyError::Truncated => self.truncated += 1,
            AyError::BadPointer => self.bad_pointer += 1,
            AyError::NoSuchSong => self.no_such_song += 1,
            AyError::InitDidNotReturn => self.init_did_not_return += 1,
            AyError::TooLarge => self.too_large += 1,
        }
    }

    fn total(&self) -> u32 {
        self.not_an_ay_file
            + self.truncated
            + self.bad_pointer
            + self.no_such_song
            + self.init_did_not_return
            + self.too_large
    }
}

/// Peak-amplitude distribution across every tune that parsed, bucketed
/// rather than histogrammed — the mix-headroom question this sweep exists
/// to answer (see the module doc) only needs "how many are quiet, how many
/// sit near full scale, how many are already over it", not a fine curve.
/// `silent` doubles as this sweep's audible/silent split: a tune whose peak
/// never clears [`AUDIBLE_THRESHOLD`] lands here and nowhere else.
/// How much of a tune's playback its interrupt routine failed to return in.
///
/// Counted in frames, not as a per-tune flag. A flag cannot tell three
/// overrunning frames of 250 from 235 of 250, and the difference between
/// those two is exactly the shape a host bug takes: a tune that overran
/// occasionally starts overrunning permanently, while the flag reports both
/// states identically and the corpus total does not move. That is how a
/// change that set five tunes running loose for most of every frame passed
/// a sweep whose overrun figure was unchanged at 128/1536.
///
/// So the sweep reports the size of the problem as well as its incidence:
/// how many tunes overran at all, how many overran throughout, the total
/// across the corpus, and the worst single tune.
#[derive(Default, Debug)]
struct Overruns {
    /// Tunes whose routine overran at least one frame.
    tunes: u32,
    /// Tunes whose routine overran every frame sampled — a tune whose play
    /// routine simply never returns, rather than one that occasionally
    /// takes too long.
    always: u32,
    /// Overrunning frames, summed across every tune.
    frames: u64,
    /// The most overrunning frames any single tune had.
    worst: u32,
}

impl Overruns {
    fn record(&mut self, overran: u32) {
        if overran == 0 {
            return;
        }
        self.tunes += 1;
        if overran >= FRAMES {
            self.always += 1;
        }
        self.frames += u64::from(overran);
        self.worst = self.worst.max(overran);
    }
}

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
    /// (1.0, 1.5]: over full scale, but not by a wide margin. Reachable
    /// only because this sweep buckets the pre-clamp peak. Empty on both
    /// passes as measured, and asserted empty on both — see the bars at the
    /// end of each.
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

/// One tune that wrote the 128K paging port, kept by name.
///
/// The tunes that page memory and the tunes that read the AY back are one
/// small group, and both are things this host can either model or drop on
/// the floor. Recording the group per tune — rather than as a count — is
/// what makes "did modelling it help *these*?" answerable: a group-level
/// total can hold still while half its members swap places.
struct PagingTune {
    name: String,
    song: usize,
    /// Bitwise-OR of every value the tune wrote to the port; see
    /// [`play198x_core::host::spectrum::SpectrumHost::paging_values_seen`].
    values: u8,
    /// Pre-clamp peak, the same figure [`PeakBuckets`] buckets.
    peak: f32,
    /// Frames of [`FRAMES`] whose interrupt routine overran. Listed per
    /// tune because this is the group whose memory model changed, so it is
    /// the group where a routine that stops returning would show first.
    overran: u32,
    /// Whether this tune also read the AY's register-read port, so the
    /// overlap between the paging cohort and the read-back cohort is
    /// visible per tune rather than inferred from two equal counts.
    reads_ay: bool,
}

/// One tune that read the AY's register-read port, kept by name — the
/// read-side counterpart to [`PagingTune`], and listed for the same reason.
struct AyReadTune {
    name: String,
    song: usize,
    /// How many of its reads the chip answered with something other than
    /// the unattached bus; see [`AyPlayer::ay_reads_non_ff`].
    non_ff: u32,
    peak: f32,
    /// As [`PagingTune::overran`].
    overran: u32,
    /// Whether this tune also pages memory. The two cohorts are the same
    /// size and are not the same tunes, which a pair of counts alone would
    /// read as one group.
    pages: bool,
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
    // How far, not just whether, each tune's interrupt routine overran its
    // one-frame budget. `frame()` returns that fact per frame rather than
    // discarding it, and this is the only place across the whole archive
    // that can say how much of the corpus it costs — a stub player raises
    // no `INT`, so a routine waiting on one spins. See [`Overruns`].
    let mut overruns = Overruns::default();
    // The largest pre-clamp sample anywhere in the archive, so the headroom
    // margin is a number rather than a bucket.
    let mut max_peak = 0.0f32;

    let samples_per_frame = (SAMPLE_RATE / 50) as usize;
    let mut out = vec![0.0f32; samples_per_frame * 2];

    let walk = ay_files(&root);

    // Borrowed rather than consumed: the extended, every-song pass below
    // walks the same bytes again.
    for file in &walk.files {
        match AyPlayer::new(&file.bytes, 0, SAMPLE_RATE) {
            Err(err) => failures.record(&err),
            Ok(mut player) => {
                parsed += 1;
                let mut overran = 0u32;
                for _ in 0..FRAMES {
                    overran += u32::from(!player.frame());
                    player.render_frame(&mut out);
                }
                // Taken before `render`'s clamp, not from `out`. A peak
                // read back from the rendered output is at most 1.0 by
                // construction, so bucketing it would make the two
                // over-full-scale buckets unreachable and the "peaks over
                // 1.0" line tautologically zero — the metric could not
                // observe the thing it is named for.
                let peak = player.peak_before_clamp();
                max_peak = max_peak.max(peak);
                peaks.record(peak);
                overruns.record(overran);
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
    println!(
        "parsed {parsed}  failed {}  damaged archives (dropped from the walk) {}",
        failures.total(),
        walk.damaged_archives
    );
    println!("failure breakdown: {failures:?}");
    println!("audible {audible}  silent {}", peaks.silent);
    println!("peak distribution: {peaks:?}");
    println!(
        "peaks over 1.0, measured before render's clamp: {}/{parsed}  (largest {max_peak:.4})",
        peaks.over_one()
    );
    println!(
        "sound source: beeper-only {beeper_only}  ay-only {ay_only}  both {both}  neither {neither}"
    );
    println!(
        "interrupt overruns: {}/{parsed} tunes overran at least one frame, {} overran all {FRAMES}, {} overrunning frames in total, worst tune {}/{FRAMES}",
        overruns.tunes, overruns.always, overruns.frames, overruns.worst
    );

    assert!(parsed > 0, "no .ay files were found under {ARCHIVE}");

    // `render` clamps to [-1, 1] as a backstop; AC-coupling the chip's
    // output is what actually keeps the mix inside range. Measured
    // pre-clamp on 2026-08-29, the backstop does not engage anywhere in the
    // archive: the loudest tunes sit exactly on 1.0, and none goes past it.
    //
    // The bar stays `clipping` rather than tightening to `over_one`. A tune
    // crossing full scale by a hair is the backstop doing its job, and a
    // sweep that failed on it would be reporting a mix that is loud rather
    // than a mix that is broken. Asserting it against post-clamp output
    // would be true no matter how bad the mix got, which is why this is
    // measured before the clamp at all.
    assert_eq!(
        peaks.clipping, 0,
        "{} of {parsed} files drive the mix past 1.5 before the clamp",
        peaks.clipping
    );

    // A host bug does not have to stop a tune parsing or make it silent to
    // be a host bug. The shape it takes instead is a play routine that used
    // to come back and now does not: the tune still renders, still sounds
    // roughly like itself, and spends most of every frame executing
    // something nobody wrote. Counted in frames rather than tunes because
    // the tune count cannot see it — a routine overrunning 3 frames of 250
    // and one overrunning 235 are the same tune either way, and a change
    // that put five tunes into permanent overrun once passed this sweep
    // with the tune count unmoved.
    //
    // Measured baseline: 2,968 overrunning frames across 553 tunes, of a
    // possible 138,250, with 2,000 of those belonging to the 8 tunes whose
    // routine never returns at all. The bar leaves room for six more such
    // tunes before it trips — tight enough that a change putting a handful
    // of tunes into permanent overrun is caught, rather than a bar so
    // slack it only notices hundreds.
    assert!(
        overruns.frames < 4_500,
        "interrupt routines overran {} frames, against a measured baseline of 2,968",
        overruns.frames
    );

    // `audible / parsed` cannot catch a parser regression on its own: break
    // parsing on 90% of the archive and the survivors could still clear an
    // 85% audible share, so this test would keep passing while reporting a
    // parser that had mostly stopped working. This checks the denominator
    // directly. Measured baseline: 553/696 parsed. The floor sits at 500,
    // real margin under that — room for a handful more legitimately
    // InitDidNotReturn files to turn up without tripping this — while still
    // catching the shape of regression the audible-share assertion below
    // cannot.
    assert!(
        parsed >= 500,
        "parsed count dropped well below the measured baseline of 553/696: {parsed}"
    );

    // Measured against the real archive on 2026-08-29, on the final host:
    //   parsed 553  failed 143 (all InitDidNotReturn; BadPointer and
    //                           TooLarge are both 0, so neither `follow`'s
    //                           signed/unsigned fallback nor `parse`'s
    //                           allocation caps refuse a real file)
    //   audible 539  silent 14  -> 539/553 = 97.5%
    //   peaks (pre-clamp): silent 14, quiet 29, moderate 237, full 273,
    //                      hot 0, clipping 0; largest exactly 1.0000
    //   sound source: beeper-only 11  ay-only 527  both 6  neither 9
    //   overruns: 14 tunes overran at least one frame, 8 of them every
    //             frame, 2,968 overrunning frames in total
    //
    // The 143 are all the tune's *init* routine, not its interrupt routine:
    // `AyError::InitDidNotReturn` is raised only by `new()`'s own
    // `call(init)`. `frame()` returns its own `call(interrupt)` result
    // rather than discarding it, and this sweep counts it — as overrunning
    // frames, above — but a frame that overran is still played and is not
    // a parse failure.
    //
    // What is known about the 143, rather than one flat "expected":
    //   - 8 of 143 are the budget itself: a run at 100x `INIT_BUDGET`
    //     parses 561 rather than 553. Real work that needed more cycles
    //     than a stub init is normally given, not tunes waiting on
    //     anything.
    //   - The rest divide into tunes halted on their own `HALT` and tunes
    //     spinning at a fixed PC, both consistent with waiting for a 50Hz
    //     `INT` this stub player never raises. That split was measured at
    //     18 and 125 against the flat-64KB host and has not been re-derived
    //     since; the total is 143 either way, and the analysis of the
    //     later-song failures on the all-songs pass reaches the same place
    //     by a different route — 67 of those 73 declare their play routine
    //     at 0x0000 or 0x001B, which is the format's way of saying the init
    //     routine *is* the player.
    //
    // Swapping the `HiReg`/`LoReg` field order does not move any figure in
    // this sweep — 143 either way. That is a statement about this sweep, not
    // about the format: it plays song 0 only, and song 0's index is 0, so
    // the two halves hold the same byte there and the swap is a no-op.
    // Field order is settled in `tests/ay_format.rs` against a literal byte
    // array, which is an instrument that can actually see it.
    //
    // The bar below sits at 85%, comfortably under the measured 97.5%, so a
    // real regression in host correctness has room to show up before the
    // bar does, without the bar itself being so tight that ordinary archive
    // noise (a handful more real INT-waiting or budget-limited tunes, say)
    // trips it.
    assert!(
        audible * 100 >= parsed * 85,
        "fewer than 85% of the parsed archive made a sound: {audible}/{parsed}"
    );

    // Tunes whose routine never returns at all, as distinct from one that
    // occasionally runs long. Measured at 8; the bar leaves room for four
    // more before it trips, which is the same kind of margin the audible
    // share above carries.
    assert!(
        overruns.always <= 12,
        "{} tunes overran every one of {FRAMES} frames, against a measured 8",
        overruns.always
    );

    // ---- Extended sweep: every song of every file, not just song 0 ----
    //
    // A second, independent pass over the same bytes rather than a
    // replacement for the one above: the song-0 pass and its assertions
    // stay exactly as measured, so a regression that shows only on song 0
    // (or only beyond it) is visible either way. This pass carries its own
    // assertions, at the end and set from its own figures — a pass over
    // 63.7% of the archive that gated on nothing would be measuring the
    // majority of the corpus and guarding none of it, which is the gap it
    // was added to close.
    let mut all_parsed = 0u32;
    let mut all_failures = Failures::default();
    let mut all_peaks = PeakBuckets::default();
    let mut all_beeper_only = 0u32;
    let mut all_ay_only = 0u32;
    let mut all_both = 0u32;
    let mut all_neither = 0u32;
    let mut all_overruns = Overruns::default();
    let mut all_max_peak = 0.0f32;
    let mut total_songs = 0u32;
    let mut multi_song_files = 0u32;
    // A file whose bytes `format::parse` itself refuses. Counted apart from
    // `all_failures`, which is per-*song*: this sweep cannot know how many
    // songs such a file would have claimed, so it contributes nothing to
    // `total_songs` either.
    let mut file_parse_failed = 0u32;
    // A file where song 0 played but a later song in the same file did
    // not — the shape of thing a song-0-only sweep can never report on its
    // own, counted once per file regardless of how many later songs failed.
    let mut later_song_failed_after_song0_ok = 0u32;

    // Port 0x7FFD (128K paging) writes — see `SpectrumHost::paging_written`
    // and `paging_values_seen`'s docs for what is and is not recorded.
    let mut paging_written_tunes = 0u32;
    let mut paging_default_only_tunes = 0u32;
    let mut paging_nondefault_tunes = 0u32;
    let mut paging_nondefault_values: BTreeSet<u8> = BTreeSet::new();
    // Every tune in the paging cohort, named, so a run can be compared with
    // another tune by tune rather than count by count. See [`PagingTune`].
    let mut paging_cohort: Vec<PagingTune> = Vec::new();

    // Port reads — see `AyPlayer::any_port_read`, `ay_read` and
    // `ay_reads_non_ff`'s docs.
    let mut any_port_read_tunes = 0u32;
    let mut ay_read_tunes = 0u32;
    let mut ay_read_non_ff_tunes = 0u32;
    let mut ay_read_non_ff_events = 0u64;
    // Named for the same reason the paging cohort is: see [`AyReadTune`].
    let mut ay_read_cohort: Vec<AyReadTune> = Vec::new();

    for corpus_file in &walk.files {
        let bytes = &corpus_file.bytes;
        let file = match format::parse(bytes) {
            Ok(file) => file,
            Err(_) => {
                file_parse_failed += 1;
                continue;
            }
        };
        total_songs += file.songs.len() as u32;
        if file.songs.len() > 1 {
            multi_song_files += 1;
        }
        let mut song0_ok = false;
        let mut later_song_failed = false;
        for song_idx in 0..file.songs.len() {
            match AyPlayer::new(bytes, song_idx, SAMPLE_RATE) {
                Err(err) => {
                    all_failures.record(&err);
                    if song_idx > 0 && song0_ok {
                        later_song_failed = true;
                    }
                }
                Ok(mut player) => {
                    if song_idx == 0 {
                        song0_ok = true;
                    }
                    all_parsed += 1;
                    let mut overran = 0u32;
                    for _ in 0..FRAMES {
                        overran += u32::from(!player.frame());
                        player.render_frame(&mut out);
                    }
                    let peak = player.peak_before_clamp();
                    all_max_peak = all_max_peak.max(peak);
                    all_peaks.record(peak);
                    all_overruns.record(overran);
                    match (player.host.speaker_written, player.host.ay_written) {
                        (true, true) => all_both += 1,
                        (true, false) => all_beeper_only += 1,
                        (false, true) => all_ay_only += 1,
                        (false, false) => all_neither += 1,
                    }
                    if player.host.paging_written {
                        paging_written_tunes += 1;
                        if player.host.paging_values_seen == 0 {
                            paging_default_only_tunes += 1;
                        } else {
                            paging_nondefault_tunes += 1;
                            paging_nondefault_values.insert(player.host.paging_values_seen);
                        }
                        paging_cohort.push(PagingTune {
                            name: corpus_file.name.clone(),
                            song: song_idx,
                            values: player.host.paging_values_seen,
                            peak,
                            overran,
                            reads_ay: player.ay_read,
                        });
                    }
                    if player.any_port_read {
                        any_port_read_tunes += 1;
                    }
                    if player.ay_read {
                        ay_read_tunes += 1;
                        if player.ay_reads_non_ff > 0 {
                            ay_read_non_ff_tunes += 1;
                            ay_read_non_ff_events += u64::from(player.ay_reads_non_ff);
                        }
                        ay_read_cohort.push(AyReadTune {
                            name: corpus_file.name.clone(),
                            song: song_idx,
                            non_ff: player.ay_reads_non_ff,
                            peak,
                            overran,
                            pages: player.host.paging_written,
                        });
                    }
                }
            }
        }
        if later_song_failed {
            later_song_failed_after_song0_ok += 1;
        }
    }

    let all_audible = all_parsed - all_peaks.silent;

    println!();
    println!("--- ay corpus sweep: all songs, not just song 0 ---");
    println!(
        "files with a file-level parse failure (song count unknown, excluded from total_songs): {file_parse_failed}"
    );
    println!("total songs across the corpus: {total_songs}  multi-song files: {multi_song_files}");
    println!("parsed {all_parsed}  failed {}", all_failures.total());
    println!("failure breakdown: {all_failures:?}");
    println!("audible {all_audible}  silent {}", all_peaks.silent);
    println!("peak distribution: {all_peaks:?}");
    println!(
        "peaks over 1.0, measured before render's clamp: {}/{all_parsed}  (largest {all_max_peak:.4})",
        all_peaks.over_one()
    );
    println!(
        "sound source: beeper-only {all_beeper_only}  ay-only {all_ay_only}  both {all_both}  neither {all_neither}"
    );
    println!(
        "interrupt overruns: {}/{all_parsed} tunes overran at least one frame, {} overran all {FRAMES}, {} overrunning frames in total, worst tune {}/{FRAMES}",
        all_overruns.tunes, all_overruns.always, all_overruns.frames, all_overruns.worst
    );

    println!();
    println!("--- divergence from the song-0-only pass above ---");
    println!(
        "files where song 0 played but a later song in the same file did not: {later_song_failed_after_song0_ok}"
    );
    for (name, song0_count, all_count) in [
        (
            "NotAnAyFile",
            failures.not_an_ay_file,
            all_failures.not_an_ay_file,
        ),
        ("Truncated", failures.truncated, all_failures.truncated),
        ("BadPointer", failures.bad_pointer, all_failures.bad_pointer),
        (
            "NoSuchSong",
            failures.no_such_song,
            all_failures.no_such_song,
        ),
        (
            "InitDidNotReturn",
            failures.init_did_not_return,
            all_failures.init_did_not_return,
        ),
        ("TooLarge", failures.too_large, all_failures.too_large),
    ] {
        if song0_count == 0 && all_count > 0 {
            println!("new failure variant beyond song 0: {name} ({all_count}, zero on song 0)");
        }
    }
    println!("max peak before clamp: song 0 only {max_peak:.4}  all songs {all_max_peak:.4}");
    println!(
        "peaks.hot: song 0 only {}  all songs {}",
        peaks.hot, all_peaks.hot
    );
    println!(
        "peaks.clipping: song 0 only {}  all songs {}",
        peaks.clipping, all_peaks.clipping
    );

    println!();
    println!("--- port 0x7FFD (128K paging) writes ---");
    println!("tunes that write 0x7FFD at all: {paging_written_tunes}/{all_parsed}");
    println!("  wrote only the power-on default (0x00): {paging_default_only_tunes}");
    println!(
        "  wrote something else at least once: {paging_nondefault_tunes}  (distinct OR'd nonzero values seen, per tune: {paging_nondefault_values:?})"
    );
    paging_cohort.sort_by(|a, b| (&a.name, a.song).cmp(&(&b.name, b.song)));
    let cohort_audible = paging_cohort
        .iter()
        .filter(|tune| tune.peak > AUDIBLE_THRESHOLD)
        .count();
    let cohort_peak = paging_cohort
        .iter()
        .fold(0.0f32, |largest, tune| largest.max(tune.peak));
    println!(
        "  cohort: {} tunes, {cohort_audible} audible, {} silent, largest peak {cohort_peak:.4}",
        paging_cohort.len(),
        paging_cohort.len() - cohort_audible
    );
    for tune in &paging_cohort {
        println!(
            "  {:<44} song {:<3} 0x7FFD bits 0x{:02X}  peak {:.4}  overran {:>3}/{FRAMES}{}",
            tune.name,
            tune.song,
            tune.values,
            tune.peak,
            tune.overran,
            if tune.reads_ay {
                "  reads the AY back"
            } else {
                ""
            }
        );
    }

    println!();
    println!("--- port reads (the AY answers its own port; everything else reads 0xFF) ---");
    println!("tunes that read any I/O port at all: {any_port_read_tunes}/{all_parsed}");
    println!("tunes that read the AY's register-read port: {ay_read_tunes}/{all_parsed}");
    println!(
        "  of those, tunes the chip gave something other than 0xFF: {ay_read_non_ff_tunes}/{ay_read_tunes}  (total such read events across those tunes: {ay_read_non_ff_events})"
    );
    ay_read_cohort.sort_by(|a, b| (&a.name, a.song).cmp(&(&b.name, b.song)));
    let read_cohort_audible = ay_read_cohort
        .iter()
        .filter(|tune| tune.peak > AUDIBLE_THRESHOLD)
        .count();
    println!(
        "  cohort: {} tunes, {read_cohort_audible} audible, {} silent",
        ay_read_cohort.len(),
        ay_read_cohort.len() - read_cohort_audible
    );
    for tune in &ay_read_cohort {
        println!(
            "  {:<44} song {:<3} non-0xFF reads {:<6} peak {:.4}  overran {:>3}/{FRAMES}{}",
            tune.name,
            tune.song,
            tune.non_ff,
            tune.peak,
            tune.overran,
            if tune.pages { "  pages memory" } else { "" }
        );
    }

    assert!(
        total_songs > 0,
        "no songs were found across the corpus on the all-songs pass"
    );

    // This pass covers the 1,219 songs the song-0 pass never reaches — 63.7%
    // of the archive — and until now it gated on nothing at all, which left
    // the majority of the corpus measured and unguarded. The bars below are
    // its own, set from its own measurement rather than scaled from the
    // song-0 pass, because the two passes do not fail the same way: a bug
    // that only shows on a subtune is invisible above and lands here.
    //
    // Measured against the real archive on 2026-08-29, on the final host:
    //   total songs 1,915 across 696 files; 278 files carry more than one
    //   parsed 1,536  failed 379 (all InitDidNotReturn)
    //   audible 1,490  silent 46  -> 1,490/1,536 = 97.0%
    //   peaks (pre-clamp): silent 46, quiet 108, moderate 569, full 813,
    //                      hot 0, clipping 0; largest exactly 1.0000
    //   overruns: 85 tunes, 32 of them every frame, 14,085 frames in total
    //
    // **The four bars marked (discriminating) fail against the host this
    // branch replaced**, and that has been run rather than reasoned about:
    // with the frame's trailing cycles clocking the CPU again and the HALT
    // byte back at the sentinel, this pass reports hot 2, largest 1.0294,
    // 128 overrunning tunes and 17,599 overrunning frames, and each of the
    // four trips. A bar nobody has watched fail is a bar nobody has tested,
    // and the first four written here did not fail against that host at
    // all — they were decoration on the blind spot this pass exists to
    // close.
    //
    // The other three stay, and are honestly not discriminating: a parse
    // floor, an audible share and a clipping bar all pass against the
    // broken host. They guard a different shape of regression — a parser
    // that stops accepting files, a host that stops making sound, a mix
    // that loses its headroom — and the fact that one bug got past them is
    // an argument for the four above, not against these three.

    // (discriminating: 2 on the pre-fix host.) The bucket says how many
    // songs cross full scale before the clamp; `all_max_peak` below says by
    // how much. Both, because a single song a hair over and a mix that has
    // lost its headroom are different faults and should not report
    // identically.
    assert_eq!(
        all_peaks.hot, 0,
        "{} of {all_parsed} songs exceed full scale before the clamp",
        all_peaks.hot
    );
    // (discriminating: 1.0294 on the pre-fix host.)
    assert!(
        all_max_peak <= 1.0,
        "the loudest song reached {all_max_peak} before the clamp"
    );
    // (discriminating: 128 on the pre-fix host.) Measured 85; the bar
    // leaves room for fifteen more songs whose routine stops returning.
    assert!(
        all_overruns.tunes <= 100,
        "{} songs overran at least one frame, against a measured 85",
        all_overruns.tunes
    );
    // (discriminating: 17,599 on the pre-fix host.) 14,085 of a possible
    // 384,000, with 8,000 of them belonging to the 32 songs whose routine
    // never returns. Room for seven more such songs before it trips.
    assert!(
        all_overruns.frames < 16_000,
        "interrupt routines overran {} frames, against a measured baseline of 14,085",
        all_overruns.frames
    );

    assert_eq!(
        all_peaks.clipping, 0,
        "{} of {all_parsed} songs drive the mix past 1.5 before the clamp",
        all_peaks.clipping
    );
    assert!(
        all_parsed >= 1_400,
        "songs parsed dropped well below the measured baseline of 1,536/1,915: {all_parsed}"
    );
    assert!(
        all_audible * 100 >= all_parsed * 90,
        "fewer than 90% of the parsed songs made a sound: {all_audible}/{all_parsed}"
    );
    // Not discriminating either, and in the other direction: the pre-fix
    // host reported 21 against this host's 32, because stopping the CPU
    // running loose made ten beeper routines' permanent overruns visible
    // as permanent. It guards the future rather than the past.
    assert!(
        all_overruns.always <= 45,
        "{} songs overran every one of {FRAMES} frames, against a measured 32",
        all_overruns.always
    );
}

/// What one walk of the archive produced: every `.ay` file's bytes, plus how
/// many archives along the way could not be opened or listed at all.
struct Walk {
    files: Vec<CorpusFile>,
    /// A `.zip` that `Container::open` or `.entries()` refused. Counted
    /// rather than silently skipped: a walker that drops damaged input from
    /// its own denominator with no record of having done so is exactly the
    /// kind of silent loss this sweep exists to catch elsewhere in the
    /// archive, and would otherwise inflate every percentage above by
    /// shrinking `parsed`'s denominator without saying so. Zero on every
    /// run so far (this archive's 696 `.zip` files each open cleanly), but
    /// the count is printed either way rather than assumed.
    damaged_archives: u32,
}

/// One `.ay` file's bytes, under the name it arrived with.
///
/// The name is carried so the paging report below can say *which* tunes
/// drive the 128K paging port, not only how many. That group is small
/// enough to name, and naming it is what lets a later run be compared tune
/// by tune: a count alone could hold steady at 17 while its membership
/// changed underneath, which is exactly the move a regression would make.
struct CorpusFile {
    name: String,
    bytes: Vec<u8>,
}

/// Every `.ay` file under `root`, read directly from disk, plus every `.ay`
/// entry inside an `.ay.zip` sibling, read through this crate's own
/// container reader rather than a second unzipper — the same path a real
/// caller opening one of these archives would take.
fn ay_files(root: &Path) -> Walk {
    let mut walk = Walk {
        files: Vec::new(),
        damaged_archives: 0,
    };
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
                // Through `Container`, like the branch below, and not
                // `fs::read`. This archive is 696 `.zip` files and no loose
                // `.ay` at all, so nothing here has ever run — which is
                // exactly why it should not be the one path in this walk
                // that reads a whole file of unknown size straight into
                // memory. `Container::open` handles a plain file and
                // applies the same cap as the rest.
                let name = file_name(&path);
                if let Ok(container) = Container::open(&path)
                    && let Ok(bytes) = container.read(&name)
                {
                    walk.files.push(CorpusFile { name, bytes });
                }
            } else if lower.ends_with(".zip") {
                let Ok(container) = Container::open(&path) else {
                    walk.damaged_archives += 1;
                    continue;
                };
                let Ok(zip_entries) = container.entries() else {
                    walk.damaged_archives += 1;
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
                        walk.files.push(CorpusFile {
                            name: format!("{}!{}", file_name(&path), zip_entry.path),
                            bytes: inner,
                        });
                    }
                }
            }
        }
    }
    walk
}

/// The last component of `path`, for naming a tune in this sweep's output.
///
/// The leading directories are the machine this archive happens to be
/// mounted on, which is not a fact about the corpus and would make one
/// run's output diff against another's for no reason.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}
