//! How long a module lasts, and whether it comes back on itself.
//!
//! Every duration here is a number that `rows x speed x tick` gets wrong. That
//! is the point of the file: `Bxx` and `Dxx` change where the walk goes, `Fxy`
//! changes how fast, and `EEx` and `E6x` change how many times, so a duration
//! has to be walked rather than multiplied. A test that would still pass
//! against a multiplication has not tested anything.
//!
//! Every figure is asserted to the exact millisecond rather than with a
//! tolerance. It can be: a tick is `2500 / tempo` ms, and at every tempo used
//! here that is exactly representable in binary floating point, so the sums
//! are exact and a tolerance would only hide arithmetic that had gone wrong.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Cell, SampleSpec, module, square};
use format198x_commodore_amiga_mod::Module;
use play198x_core::engine::Engine;
use std::time::Duration;

const RATE: u32 = 44_100;

/// One row at the default speed 6 and tempo 125.
const ROW_MS: u128 = 120;

/// Frames in one row at the default speed and tempo: 6 ticks of 882.
const ROW_FRAMES: usize = 5_292;

fn square_sample() -> SampleSpec {
    SampleSpec {
        data: square(32, 1, 100),
        volume: 64,
        repeat_start_words: 0,
        repeat_length_words: 16,
    }
}

/// A single-pattern, single-order module holding exactly `cells`.
fn one_pattern(cells: Vec<Cell>) -> Module {
    module(&[square_sample()], &[cells], &[0], 1)
}

/// A C-2 on row 0, and `(effect, param)` alone on row `rows - 1`.
fn ends_with(rows: usize, effect: u8, param: u8) -> Module {
    one_pattern(vec![
        Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            ..Cell::default()
        },
        Cell {
            row: rows - 1,
            channel: 0,
            effect,
            param,
            ..Cell::default()
        },
    ])
}

/// `rows` rows, ended by a `D00` pattern break on the last of them.
fn rows_module(rows: usize) -> Module {
    ends_with(rows, 0x0D, 0x00)
}

// ---------------------------------------------------------------------------
// The two the plan asks for
// ---------------------------------------------------------------------------

#[test]
fn a_four_row_module_at_the_default_tempo_lasts_its_four_rows() {
    let engine = Engine::new(rows_module(4), RATE);
    let timing = engine.timing();
    assert_eq!(timing.duration.as_millis(), 480, "4 rows x 120 ms");
    assert!(
        !timing.loops,
        "a song that runs off its order table has ended"
    );
    assert_eq!(timing.loop_start, None);
}

#[test]
fn a_module_that_jumps_backwards_is_reported_as_looping() {
    // `B00` restarts the song from order 0, which is a position it has already
    // played. A `D00` in the same place ends it instead — same jump target,
    // opposite answer — so this pins that the two are told apart.
    let engine = Engine::new(ends_with(4, 0x0B, 0x00), RATE);
    let timing = engine.timing();
    assert!(timing.loops);
    assert_eq!(timing.loop_start, Some(Duration::ZERO));
    assert_eq!(timing.duration.as_millis(), 480);
}

#[test]
fn a_loop_back_to_the_middle_of_a_song_reports_where_it_comes_back_to() {
    // Three orders of four rows. The third ends with `B01`, so playback comes
    // back to order 1 — 480 ms in, not the top.
    let pattern = |last: (u8, u8)| {
        vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            },
            Cell {
                row: 3,
                channel: 0,
                effect: last.0,
                param: last.1,
                ..Cell::default()
            },
        ]
    };
    let source = module(
        &[square_sample()],
        &[
            pattern((0x0D, 0x00)),
            pattern((0x0D, 0x00)),
            pattern((0x0B, 0x01)),
        ],
        &[0, 1, 2],
        3,
    );

    let timing = Engine::new(source, RATE).timing();
    assert!(timing.loops);
    assert_eq!(timing.duration.as_millis(), 3 * 4 * ROW_MS);
    assert_eq!(timing.loop_start, Some(Duration::from_millis(480)));
}

// ---------------------------------------------------------------------------
// The position effects that change how long a module lasts
// ---------------------------------------------------------------------------

#[test]
fn a_pattern_loop_is_not_a_song_loop() {
    // `E60` on row 0 marks the loop start, `E62` on row 3 plays rows 0..=3
    // twice more, `D00` on row 7 ends the song. Rows 0..=3 are therefore
    // *supposed* to be played three times, and a walk that treats a repeated
    // `(order, row)` as a loop reports this module as looping after 480 ms
    // instead of playing for 1920 ms.
    let source = one_pattern(vec![
        Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            effect: 0x0E,
            param: 0x60,
        },
        Cell {
            row: 3,
            channel: 0,
            effect: 0x0E,
            param: 0x62,
            ..Cell::default()
        },
        Cell {
            row: 7,
            channel: 0,
            effect: 0x0D,
            param: 0x00,
            ..Cell::default()
        },
    ]);

    let timing = Engine::new(source, RATE).timing();
    assert!(
        !timing.loops,
        "a bounded E6x repeat is not the song coming back on itself"
    );
    assert_eq!(timing.loop_start, None);
    // Rows 0..=3 three times, then rows 4..=7 once: 16 rows.
    assert_eq!(timing.duration.as_millis(), 16 * ROW_MS);
}

#[test]
fn a_pattern_delay_lengthens_the_module_without_looping_it() {
    // `EE1` replays row 0 once more without re-fetching it. The row is not
    // revisited — nothing steps back to it — so it must not read as a loop,
    // and the module lasts three rows rather than two.
    let source = one_pattern(vec![
        Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            effect: 0x0E,
            param: 0xE1,
        },
        Cell {
            row: 1,
            channel: 0,
            effect: 0x0D,
            param: 0x00,
            ..Cell::default()
        },
    ]);

    let timing = Engine::new(source, RATE).timing();
    assert!(!timing.loops);
    assert_eq!(timing.duration.as_millis(), 3 * ROW_MS);
}

#[test]
fn setting_the_speed_and_the_tempo_changes_how_long_the_module_lasts() {
    // Four rows either way. Only the clock differs, which is exactly what a
    // `rows x speed x tick` estimate cannot see.
    for (effect_param, expected_ms, why) in [
        (0x03u8, 240u128, "F03: 4 rows x 3 ticks x 20 ms"),
        (
            0x20,
            1_875,
            "F20: tempo 32, so 4 rows x 6 ticks x 78.125 ms",
        ),
    ] {
        let source = one_pattern(vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0F,
                param: effect_param,
            },
            Cell {
                row: 3,
                channel: 0,
                effect: 0x0D,
                param: 0x00,
                ..Cell::default()
            },
        ]);
        let timing = Engine::new(source, RATE).timing();
        assert_eq!(timing.duration.as_millis(), expected_ms, "{why}");
        assert!(!timing.loops, "{why}");
    }
}

#[test]
fn f00_stops_the_module_where_it_stands() {
    // `mt_setspeed` stores the zero and then branches to `_mt_end`
    // (protracker-23b-playroutine.asm:2347), which clears `mt_Enable` and
    // resets the channels. So a module carrying `F00` on row 1 is one row
    // long, however many rows are written after it.
    let source = one_pattern(vec![
        Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            ..Cell::default()
        },
        Cell {
            row: 1,
            channel: 0,
            effect: 0x0F,
            param: 0x00,
            ..Cell::default()
        },
        Cell {
            row: 40,
            channel: 0,
            sample: 1,
            period: 428,
            ..Cell::default()
        },
    ]);

    let timing = Engine::new(source.clone(), RATE).timing();
    assert_eq!(timing.duration.as_millis(), ROW_MS, "one row, then stopped");
    assert!(!timing.loops);

    // And the same in the audio: everything from the stop onwards is silence,
    // or the duration would be describing a module that carries on sounding.
    let mut engine = Engine::new(source, RATE);
    let mut buf = vec![0f32; ROW_FRAMES * 4 * 2];
    assert_eq!(engine.render(&mut buf), ROW_FRAMES * 4);
    let sounding = buf[..ROW_FRAMES * 2]
        .iter()
        .filter(|value| **value != 0.0)
        .count();
    assert!(sounding > 1_000, "row 0 must sound: {sounding} frames did");
    assert_eq!(
        buf[ROW_FRAMES * 2..]
            .iter()
            .filter(|value| **value != 0.0)
            .count(),
        0,
        "nothing may sound after F00"
    );
}

// ---------------------------------------------------------------------------
// Termination
// ---------------------------------------------------------------------------

#[test]
fn a_long_legitimate_module_walks_to_its_end_well_inside_the_cap() {
    // The shape the tick cap is derived against: 128 orders, and every row
    // held for the longest `EEx` pattern delay there is. 128 x 64 rows x 16
    // rounds x 6 ticks = 786,432 ticks — comfortably under the ten million
    // the walk allows, which is what "real headroom" has to mean.
    let cells = (0..64)
        .map(|row| Cell {
            row,
            channel: 0,
            sample: if row == 0 { 1 } else { 0 },
            period: if row == 0 { 428 } else { 0 },
            effect: 0x0E,
            param: 0xEF,
        })
        .collect();
    let source = module(&[square_sample()], &[cells], &[0; 128], 128);

    let timing = Engine::new(source, RATE).timing();
    assert_eq!(
        timing.duration.as_millis(),
        128 * 64 * 16 * 6 * 20,
        "786,432 ticks of 20 ms"
    );
    // A song this long has no end to run off. `song_step` masks the next order
    // with `and.w #$007f` *before* comparing it against the song length, so at
    // order 127 the next one is 0, which is still inside a 128-order song:
    // ProTracker never raises `mt_SongEnd` for one. The visited set is the only
    // thing that stops the walk, and it stops it in the right place — after all
    // 128 orders, back at the top.
    assert!(timing.loops, "the order index wraps inside its 7-bit field");
    assert_eq!(timing.loop_start, Some(Duration::ZERO));
}

#[test]
fn an_order_naming_a_pattern_the_file_does_not_hold_still_takes_its_time() {
    // Real files carry garbage in the order table. Such an order plays as 64
    // empty rows and takes their time — it is neither skipped nor a panic.
    let source = module(
        &[square_sample()],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            },
            Cell {
                row: 3,
                channel: 0,
                effect: 0x0D,
                param: 0x00,
                ..Cell::default()
            },
        ]],
        &[0, 9],
        2,
    );

    let timing = Engine::new(source, RATE).timing();
    assert_eq!(
        timing.duration.as_millis(),
        (4 + 64) * ROW_MS,
        "4 rows of pattern 0, then a whole empty pattern"
    );
    assert!(!timing.loops);
}

#[test]
fn a_position_jump_past_the_end_of_the_song_ends_it() {
    // `B7F` in a one-order song lands past the played prefix, which is the
    // replayer's own end-of-song condition (`cmp.b 950(a0),d0`, line 1434).
    let timing = Engine::new(ends_with(1 + 1, 0x0B, 0x7F), RATE).timing();
    assert_eq!(timing.duration.as_millis(), 2 * ROW_MS);
    assert!(!timing.loops);
}

#[test]
fn no_effect_and_no_parameter_can_make_the_duration_walk_hang_or_panic() {
    // The same sweep the mixer gets, against the walk: every effect number
    // against the parameters that hit its edges, on a module whose order table
    // also names a pattern that does not exist.
    let mut looping = 0usize;
    let mut longest = Duration::ZERO;
    let mut plain = Duration::ZERO;
    for effect in 0u8..16 {
        for param in [0x00u8, 0x01, 0x0F, 0x10, 0x1F, 0x20, 0x7F, 0x80, 0xC3, 0xFF] {
            let source = module(
                &[square_sample()],
                &[vec![
                    Cell {
                        row: 0,
                        channel: 0,
                        sample: 1,
                        period: 428,
                        effect,
                        param,
                    },
                    Cell {
                        row: 1,
                        channel: 0,
                        effect,
                        param,
                        ..Cell::default()
                    },
                ]],
                &[0, 7],
                2,
            );
            let timing = Engine::new(source, RATE).timing();
            assert!(
                timing.duration <= Duration::from_secs(60 * 60),
                "effect {effect:X}{param:02X} walked for {:?}",
                timing.duration
            );
            if timing.loops {
                looping += 1;
                assert!(
                    timing.loop_start.is_some_and(|at| at <= timing.duration),
                    "effect {effect:X}{param:02X}: a loop must start inside the song"
                );
            } else {
                assert_eq!(timing.loop_start, None);
            }
            longest = longest.max(timing.duration);
            if effect == 0 && param == 0 {
                plain = timing.duration;
            }
        }
    }
    // The sweep has to reach the sequencer, or it proves only that nothing
    // hung. Exactly two of the 160 pairs loop, and which two is arithmetic
    // rather than luck: `B00` and `B80` both land on order 0 of a two-order
    // song, because `mt_posjump` stores `xy - 1` masked to 7 bits and the
    // end-of-row step adds one back. Every other `Bxy` here lands past the
    // song length, which is an ending, not a loop.
    assert_eq!(looping, 2, "B00 and B80, and nothing else");
    // And the walk has to be measuring, not just surviving. The module with
    // no effect at all plays both its orders — 64 rows of pattern 0 and 64 of
    // the pattern the file does not hold — and the longest of the whole sweep
    // is `F1F`, which is those same 128 rows at speed 31.
    assert_eq!(plain.as_millis(), 128 * ROW_MS);
    assert_eq!(
        longest.as_millis(),
        128 * 31 * 20,
        "F1F: 128 rows at speed 31"
    );
}

// ---------------------------------------------------------------------------
// `timing` takes `&self`, and has to mean it
// ---------------------------------------------------------------------------

#[test]
fn asking_for_the_timing_does_not_disturb_playback() {
    let mut measured = Engine::new(rows_module(4), RATE);
    let mut control = Engine::new(rows_module(4), RATE);

    let mut first = vec![0f32; 3_000 * 2];
    let mut second = vec![0f32; 3_000 * 2];
    measured.render(&mut first);
    control.render(&mut second);

    let before = measured.timing();
    let again = measured.timing();
    assert_eq!(before, again, "the walk must be repeatable");
    assert_eq!(before.duration.as_millis(), 480);

    let mut after_measured = vec![0f32; 3_000 * 2];
    let mut after_control = vec![0f32; 3_000 * 2];
    measured.render(&mut after_measured);
    control.render(&mut after_control);

    assert_eq!(
        measured.position(),
        control.position(),
        "timing() moved the sequencer"
    );
    assert_eq!(
        after_measured, after_control,
        "timing() changed what playback produces"
    );
}
