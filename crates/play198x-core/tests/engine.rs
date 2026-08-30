//! What the sequencer and mixer actually produce.
//!
//! Every assertion here is a measurement with a stated tolerance. "It rendered
//! some audio" is not a test of a replayer: the two numbers that matter — the
//! row duration and the sample rate a period plays at — are wrong by a
//! constant factor in the ways this engine can plausibly be wrong, and only a
//! measured value catches that.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Cell, SampleSpec, module, paula_rate, square, zero_crossing_hz};
use play198x_core::engine::{Engine, ModulePosition};

const RATE: u32 = 44_100;

/// One pattern, one looped square-wave sample, a C-2 on channel 0 at row 0.
fn one_pattern_module() -> format198x_commodore_amiga_mod::Module {
    module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[vec![Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            ..Cell::default()
        }]],
        &[0],
        1,
    )
}

fn left(interleaved: &[f32]) -> impl Iterator<Item = f32> + '_ {
    interleaved.as_chunks::<2>().0.iter().map(|f| f[0])
}

fn right(interleaved: &[f32]) -> impl Iterator<Item = f32> + '_ {
    interleaved.as_chunks::<2>().0.iter().map(|f| f[1])
}

#[test]
fn a_row_lasts_speed_ticks_at_the_default_tempo() {
    // Default: speed 6, tempo 125 -> tick = 2500/125 = 20 ms -> row = 120 ms.
    // The single most important number in the engine: everything downstream is
    // out by the same factor if this is.
    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![0f32; 2];
    let mut frames = 0;
    // Capped. The plan's version of this test loops until the row changes,
    // which against an engine whose sequencer never advances hangs instead of
    // failing — a stalled clock is exactly one of the faults it is here to
    // catch, so it must report rather than wait.
    while engine.position().row == 0 && frames < RATE as usize {
        frames += engine.render(&mut buf);
    }
    assert!(
        frames < RATE as usize,
        "the row never ended within a second"
    );
    let ms = f64::from(frames as u32) / 44.1;
    assert!(
        (ms - 120.0).abs() < 1.0,
        "row lasted {ms:.2} ms, expected 120 +/- 1"
    );
}

#[test]
fn ticks_and_rows_land_on_exact_frame_boundaries() {
    // 44100 * 2.5 / 125 = 882 frames a tick, exactly, so every boundary is an
    // integer here and can be pinned to the frame rather than to a tolerance.
    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![0f32; 2];
    let mut seen = Vec::new();
    let mut last = engine.position();
    for frame in 0..6_000usize {
        engine.render(&mut buf);
        let now = engine.position();
        if now != last {
            seen.push((frame, now));
            last = now;
        }
    }

    let at = |order, row, tick| ModulePosition {
        order,
        pattern: 0,
        row,
        tick,
    };
    assert_eq!(
        seen,
        vec![
            (882, at(0, 0, 1)),
            (1_764, at(0, 0, 2)),
            (2_646, at(0, 0, 3)),
            (3_528, at(0, 0, 4)),
            (4_410, at(0, 0, 5)),
            (5_292, at(0, 1, 0)),
        ],
        "six 882-frame ticks, then the next row"
    );
}

#[test]
fn a_period_plays_at_the_paula_rate() {
    // rate = 7093789.2 / (2 * period); the 32-byte sample is one square-wave
    // cycle, so the carrier is that rate divided by 32.
    for period in [214u16, 428, 856] {
        let mut source = one_pattern_module();
        source.patterns[0][0][0].period = period;
        let mut engine = Engine::new(source, RATE);

        let mut buf = vec![0f32; RATE as usize * 2];
        assert_eq!(engine.render(&mut buf), RATE as usize);

        // A full second of window. 2.5 ms cannot resolve a 130 Hz carrier at
        // all — it quantises to 0, 200 or 400 Hz, which cost a wrong
        // conclusion about the mixer on 2026-08-25.
        let measured = zero_crossing_hz(&buf, f64::from(RATE), RATE as usize);
        let expected = paula_rate(period) / 32.0;
        let error = (measured - expected).abs() / expected;
        assert!(
            error < 0.02,
            "period {period}: measured {measured:.2} Hz, expected {expected:.2} Hz \
             ({:.2}% off, tolerance 2%)",
            error * 100.0
        );
    }
}

#[test]
fn c2_is_8287_hz_and_the_carrier_follows_from_it() {
    // The reference's own worked example, pinned so a change to the clock
    // constant fails here by name rather than as a vague tuning drift.
    assert!((paula_rate(428) - 8_287.14).abs() < 0.01);

    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![0f32; RATE as usize * 2];
    engine.render(&mut buf);
    let measured = zero_crossing_hz(&buf, f64::from(RATE), RATE as usize);
    assert!(
        (measured - 258.97).abs() < 5.18,
        "C-2 carrier measured {measured:.2} Hz, expected 258.97 +/- 5.18 (2%)"
    );
}

#[test]
fn a_one_shot_sample_stops_at_the_end_of_its_data() {
    // repeat length 0: below the one-word threshold, so it plays once. 32
    // bytes at 8287.14 bytes/s is 3.8615 ms.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 0,
        }],
        &[vec![Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            ..Cell::default()
        }]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    let mut buf = vec![0f32; RATE as usize * 2];
    engine.render(&mut buf);

    let last_sounding = left(&buf)
        .enumerate()
        .filter(|(_, v)| *v != 0.0)
        .map(|(i, _)| i)
        .last()
        .expect("the sample must sound at all");
    let ms = (last_sounding + 1) as f64 * 1_000.0 / f64::from(RATE);
    let expected = 32.0 / paula_rate(428) * 1_000.0;
    assert!(
        (ms - expected).abs() < 0.1,
        "one-shot lasted {ms:.4} ms, expected {expected:.4} +/- 0.1"
    );
}

#[test]
fn a_looped_sample_sounds_for_as_long_as_it_is_asked_to() {
    // Nearest-neighbour resampling never interpolates, so a two-level square
    // wave stays at exactly its two levels: 100/128 * 0.5 = 0.390625.
    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![0f32; RATE as usize * 2];
    engine.render(&mut buf);

    let off_level = left(&buf)
        .filter(|v| (v.abs() - 0.390_625).abs() > 1e-6)
        .count();
    assert_eq!(
        off_level, 0,
        "every one of a second's frames must sit on +/-0.390625"
    );
}

#[test]
fn channels_are_hard_panned_and_scaled_to_leave_no_clipping() {
    // Amiga panning: 0 and 3 left, 1 and 2 right. Two channels can land on
    // one side, so each is halved and a side reaches full scale only when both
    // of its channels do.
    for (channel, expect_left, expect_right) in [
        (0usize, 0.496_093_75f32, 0.0f32),
        (1, 0.0, 0.496_093_75),
        (2, 0.0, 0.496_093_75),
        (3, 0.496_093_75, 0.0),
    ] {
        let source = module(
            &[SampleSpec {
                data: vec![127u8; 4_096],
                volume: 64,
                repeat_start_words: 0,
                repeat_length_words: 0,
            }],
            &[vec![Cell {
                row: 0,
                channel,
                sample: 1,
                period: 428,
                ..Cell::default()
            }]],
            &[0],
            1,
        );
        let mut engine = Engine::new(source, RATE);
        let mut buf = vec![0f32; 1_000 * 2];
        engine.render(&mut buf);

        for (i, frame) in buf.as_chunks::<2>().0.iter().enumerate() {
            assert!(
                (frame[0] - expect_left).abs() < 1e-7 && (frame[1] - expect_right).abs() < 1e-7,
                "channel {channel} frame {i}: got ({}, {}), expected ({expect_left}, {expect_right})",
                frame[0],
                frame[1]
            );
        }
    }
}

#[test]
fn volume_scales_the_output_linearly() {
    // 127/128 (full-scale byte) * volume/64 * 0.5 (voice gain): 127/256 and
    // 127/512, both exact in f32, so these compare at 1e-7 rather than loosely.
    for (volume, expected) in [(64u8, 127.0f32 / 256.0), (32, 127.0 / 512.0), (0, 0.0)] {
        let source = module(
            &[SampleSpec {
                data: vec![127u8; 4_096],
                volume,
                repeat_start_words: 0,
                repeat_length_words: 0,
            }],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            }]],
            &[0],
            1,
        );
        let mut engine = Engine::new(source, RATE);
        let mut buf = vec![0f32; 100 * 2];
        engine.render(&mut buf);
        assert!(
            (buf[0] - expected).abs() < 1e-7,
            "volume {volume}: got {}, expected {expected}",
            buf[0]
        );
    }
}

#[test]
fn song_length_bounds_the_order_table_not_the_pattern_count() {
    // Order table [0, 3], song length 1. The played prefix is one entry, so
    // the song wraps to order 0 after 64 rows. An engine that walked the whole
    // table would report pattern 3 here — a pattern this file does not even
    // contain.
    let source = module(&[SampleSpec::empty()], &[vec![]], &[0, 3], 1);
    let mut engine = Engine::new(source, RATE);

    // A row's worth of frames at a time: after each, the clock sits on the
    // last tick of that row, one frame short of the next one.
    let mut a_row = vec![0f32; 5_292 * 2];
    for row in 0..64 {
        engine.render(&mut a_row);
        assert_eq!(
            engine.position(),
            ModulePosition {
                order: 0,
                pattern: 0,
                row,
                tick: 5
            },
            "row {row}"
        );
    }

    let mut one_frame = vec![0f32; 2];
    engine.render(&mut one_frame);
    assert_eq!(
        engine.position(),
        ModulePosition {
            order: 0,
            pattern: 0,
            row: 0,
            tick: 0
        },
        "the song restarts at order 0, as ProTracker 2.3 does"
    );
}

#[test]
fn pausing_stops_the_clock_as_well_as_the_sound() {
    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![0f32; 1_000 * 2];
    engine.render(&mut buf);
    let paused_at = engine.position();

    engine.set_playing(false);
    let mut silence = vec![0f32; RATE as usize * 2];
    assert_eq!(engine.render(&mut silence), RATE as usize);
    assert_eq!(
        silence.iter().filter(|v| **v != 0.0).count(),
        0,
        "a paused engine renders exact zeroes, not a held note"
    );
    assert_eq!(
        engine.position(),
        paused_at,
        "and does not advance the sequencer"
    );

    engine.set_playing(true);
    engine.render(&mut buf);
    assert_ne!(engine.position(), paused_at, "resuming advances it again");
}

#[test]
fn seeking_moves_the_sequencer_and_cuts_the_sounding_voices() {
    // Order 1 is an empty pattern: nothing retriggers there, so anything still
    // audible after the seek is a voice that should have been cut.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[
            vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            }],
            vec![],
        ],
        &[0, 1],
        2,
    );
    let mut engine = Engine::new(source, RATE);
    let mut buf = vec![0f32; 1_000 * 2];
    engine.render(&mut buf);
    assert!(buf[0] != 0.0, "the note is sounding before the seek");

    engine.seek_order(1);
    assert_eq!(
        engine.position(),
        ModulePosition {
            order: 1,
            pattern: 1,
            row: 0,
            tick: 0
        }
    );
    engine.render(&mut buf);
    assert_eq!(
        buf.iter().filter(|v| **v != 0.0).count(),
        0,
        "the seek cut the voice"
    );

    // Past the end clamps rather than panicking or wrapping.
    engine.seek_order(99);
    assert_eq!(engine.position().order, 1, "seek past the end clamps");
}

#[test]
fn hostile_modules_render_without_panicking() {
    let note = |sample: u8| {
        vec![Cell {
            row: 0,
            channel: 0,
            sample,
            period: 428,
            ..Cell::default()
        }]
    };

    // A zero-length sample: nothing to play, and nothing to index into.
    let empty_sample = module(
        &[SampleSpec {
            data: Vec::new(),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[note(1)],
        &[0],
        1,
    );

    // A loop that starts and ends past the end of the data.
    let loop_past_end = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 30_000,
            repeat_length_words: 30_000,
        }],
        &[note(1)],
        &[0],
        1,
    );

    // An order naming a pattern the file does not contain.
    let missing_pattern = module(&[SampleSpec::empty()], &[vec![]], &[7], 1);

    // No order entries at all.
    let empty_song = module(&[SampleSpec::empty()], &[vec![]], &[0], 0);

    // A sample number past the 31 slots.
    let no_such_sample = module(&[SampleSpec::empty()], &[note(45)], &[0], 1);

    for (name, source, expect_silent) in [
        ("zero-length sample", empty_sample, true),
        ("loop past the end", loop_past_end, false),
        ("order names a missing pattern", missing_pattern, true),
        ("empty song", empty_song, true),
        ("sample number out of range", no_such_sample, true),
    ] {
        let mut engine = Engine::new(source, RATE);
        let mut buf = vec![0f32; RATE as usize * 2];
        assert_eq!(engine.render(&mut buf), RATE as usize, "{name}");
        assert_eq!(
            buf.iter()
                .filter(|v| !v.is_finite() || v.abs() > 1.0)
                .count(),
            0,
            "{name}: every frame must be finite and inside full scale"
        );
        if expect_silent {
            assert_eq!(
                buf.iter().filter(|v| **v != 0.0).count(),
                0,
                "{name}: there is nothing here to sound"
            );
        } else {
            // The clamped loop is empty, so the sample plays once and stops.
            let last = left(&buf)
                .enumerate()
                .filter(|(_, v)| *v != 0.0)
                .map(|(i, _)| i)
                .last()
                .expect("{name}: it must sound at least once");
            let ms = (last + 1) as f64 * 1_000.0 / f64::from(RATE);
            assert!(
                (ms - 3.8615).abs() < 0.1,
                "{name}: sounded for {ms:.4} ms, expected 3.8615 +/- 0.1"
            );
        }
    }
}

#[test]
fn an_odd_length_buffer_writes_whole_frames_only() {
    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![-1.0f32; 7];
    assert_eq!(engine.render(&mut buf), 3, "3 whole frames out of 7 slots");
    assert_eq!(buf[6], -1.0, "the odd slot is left alone");
}

#[test]
fn the_right_hand_channels_are_where_the_left_ones_are_not() {
    // Guards the accessor pair used across this file: a bug that read the same
    // column twice would make several assertions above vacuous.
    let mut engine = Engine::new(one_pattern_module(), RATE);
    let mut buf = vec![0f32; 4];
    engine.render(&mut buf);
    assert_eq!(left(&buf).count(), 2);
    assert_eq!(right(&buf).count(), 2);
    assert_eq!(right(&buf).filter(|v| *v != 0.0).count(), 0);
}
