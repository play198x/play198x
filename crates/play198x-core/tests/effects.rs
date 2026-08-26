//! ProTracker's effects, measured one at a time.
//!
//! Every fixture here is a synthetic single-effect module built in code: one
//! held note and one effect. Real music confounds these measurements — an
//! attempt on 2026-08-25 measured a real module's vibrato and got the *row*
//! rate instead, because at speed 5 the 0.100 s row period dominated the
//! envelope it was looking at.
//!
//! Each test measures the observable that belongs to its effect: pitch
//! effects by period per tick and by pitch per waveform cycle, volume effects
//! by the envelope, position effects by `Position`, and sample offset by the
//! first frame's amplitude.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Cell, SampleSpec, left, modulation_hz, module, pitch_track, square};
use format198x_commodore_amiga_mod::Module;
use play198x_core::engine::Engine;

const RATE: u32 = 44_100;

/// Frames in one row at the default speed 6 and tempo 125: 6 * 882.
const ROW_FRAMES: usize = 5_292;

/// Frames in one tick at the default tempo 125: 44100 * 2.5 / 125.
const TICK_FRAMES: usize = 882;

/// A looping square wave, one cycle every 32 bytes, at full volume.
fn square_sample() -> SampleSpec {
    SampleSpec {
        data: square(32, 1, 100),
        volume: 64,
        repeat_start_words: 0,
        repeat_length_words: 16,
    }
}

/// One pattern whose row 0 plays C-2 on channel 0 with `(effect, param)`, and
/// whose remaining rows repeat the same effect with no new note.
///
/// The held note is what makes a per-tick effect measurable past the first
/// row: the effect keeps running, and nothing retriggers to hide it.
fn held(effect: u8, param: u8, rows: usize) -> Module {
    let cells = (0..rows)
        .map(|row| Cell {
            row,
            channel: 0,
            sample: if row == 0 { 1 } else { 0 },
            period: if row == 0 { 428 } else { 0 },
            effect,
            param,
        })
        .collect();
    module(&[square_sample()], &[cells], &[0], 1)
}

/// Render exactly `rows` rows at the default speed and tempo, discarding the
/// audio. Leaves the engine on the last tick of the last row rendered.
fn render_rows(engine: &mut Engine, rows: usize) {
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    for _ in 0..rows {
        engine.render(&mut buf);
    }
}

#[test]
fn a_per_tick_effect_runs_speed_minus_one_times_per_row() {
    // Volume slide down (A01) from full volume, speed 6. Six ticks in the row,
    // but the row's first tick fetches the row and does not run `fx_tab` — so
    // the slide applies five times, not six.
    //
    // A single dispatch table, or a loop that runs `speed` times, produces 58
    // here: a silent 20% error in every per-tick effect at the default speed.
    let mut engine = Engine::new(held(0x0A, 0x01, 1), RATE);
    render_rows(&mut engine, 1);
    assert_eq!(
        engine.debug_channel_volume(0),
        64 - 5,
        "the slide must apply speed-1 times, not speed"
    );
}

// ---------------------------------------------------------------------------
// Measurement helpers
// ---------------------------------------------------------------------------

/// Amplitude of a full-scale byte at `volume`, after the mixer's voice gain.
fn level(sample_byte: i8, volume: u8) -> f32 {
    f32::from(sample_byte) / 128.0 * f32::from(volume) / 64.0 * 0.5
}

// ---------------------------------------------------------------------------
// Three tables, not one
// ---------------------------------------------------------------------------

/// A sample whose first 1024 bytes are silent and whose remainder is a square
/// wave — so where playback starts is audible in the very first frame.
fn silent_then_square() -> SampleSpec {
    let mut data = vec![0u8; 1_024];
    data.extend(square(32, 32, 100));
    SampleSpec {
        data,
        volume: 64,
        repeat_start_words: 0,
        repeat_length_words: 0,
    }
}

#[test]
fn sample_offset_acts_before_the_note_starts_it() {
    // `9` is in `prefx_tab`, which runs *before* the period is set and the
    // sample restarted. An implementation that ran it after — the only place a
    // single table could put it — would start the sample at zero and this
    // frame would be silence.
    let source = module(
        &[silent_then_square()],
        &[vec![Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            effect: 0x09,
            param: 0x04,
        }]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    let mut buf = vec![0f32; 64 * 2];
    engine.render(&mut buf);
    assert!(
        (buf[0] - level(100, 64)).abs() < 1e-6,
        "904 must start 1024 bytes in and sound at once; got {}",
        buf[0]
    );

    // The control: the same sample without the effect is silent for
    // 1024 / 8287.14 = 123.6 ms.
    let control = module(
        &[silent_then_square()],
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
    let mut engine = Engine::new(control, RATE);
    let mut buf = vec![0f32; RATE as usize * 2];
    engine.render(&mut buf);
    let first = left(&buf).position(|value| value != 0.0).unwrap();
    let ms = first as f64 * 1_000.0 / f64::from(RATE);
    assert!(
        (ms - 123.6).abs() < 0.5,
        "without the effect it must stay silent for 123.6 ms; got {ms:.2}"
    );
}

#[test]
fn sample_offset_also_acts_on_a_row_with_no_note() {
    // `9` is in `morefx_tab` too, so it sets where the *next* trigger starts
    // even on a row that carries no note. A single table cannot hold one
    // effect in two places, and would leave row 2 starting from zero.
    let source = module(
        &[silent_then_square()],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            },
            // No note: only `morefx_tab` runs here.
            Cell {
                row: 1,
                channel: 0,
                effect: 0x09,
                param: 0x04,
                ..Cell::default()
            },
            // A note with no sample number, so nothing resets the offset.
            Cell {
                row: 2,
                channel: 0,
                period: 428,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    let mut buf = vec![0f32; ROW_FRAMES * 3 * 2];
    engine.render(&mut buf);
    let at_row_two = buf[ROW_FRAMES * 2 * 2];
    assert!(
        (at_row_two - level(100, 64)).abs() < 1e-6,
        "the offset set on a note-less row must apply to the next trigger; \
         row 2's first frame was {at_row_two}"
    );
}

#[test]
fn a_note_tick_effect_does_not_also_run_as_a_per_tick_effect() {
    // `C` sets the volume once, from `morefx_tab`. It is not in `fx_tab`, so a
    // collapsed table would run it every tick — which is invisible for `C`
    // itself, but the same collapse makes `A` run six times instead of five.
    // What this pins is the other half: `A` must *not* run at the note tick.
    //
    // A20 from volume 32 with no note on later rows: five steps a row up.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 32,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0A,
                param: 0x20,
            },
            Cell {
                row: 1,
                channel: 0,
                effect: 0x0A,
                param: 0x20,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 1);
    assert_eq!(engine.debug_channel_volume(0), 32 + 10, "five steps of 2");
    render_rows(&mut engine, 1);
    assert_eq!(engine.debug_channel_volume(0), 32 + 20, "ten steps of 2");
}

// ---------------------------------------------------------------------------
// Volume effects: the envelope
// ---------------------------------------------------------------------------

#[test]
fn a_volume_slide_up_stops_at_full_volume() {
    // A80 from 32: five steps of 8 is 72, and the ceiling is 64.
    let mut engine = Engine::new(
        module(
            &[SampleSpec {
                data: square(32, 1, 100),
                volume: 32,
                repeat_start_words: 0,
                repeat_length_words: 16,
            }],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0A,
                param: 0x80,
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    render_rows(&mut engine, 1);
    assert_eq!(engine.debug_channel_volume(0), 64);
}

#[test]
fn a_volume_slide_down_stops_at_silence() {
    // A0F from 32: five steps of 15 is -43, and the floor is 0.
    let mut engine = Engine::new(
        module(
            &[SampleSpec {
                data: square(32, 1, 100),
                volume: 32,
                repeat_start_words: 0,
                repeat_length_words: 16,
            }],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0A,
                param: 0x0F,
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    render_rows(&mut engine, 1);
    assert_eq!(engine.debug_channel_volume(0), 0);
}

#[test]
fn set_volume_applies_at_the_note_tick_with_or_without_a_note() {
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0C,
                param: 0x20,
            },
            // No note: `C` still applies, from `morefx_tab`.
            Cell {
                row: 1,
                channel: 0,
                effect: 0x0C,
                param: 0x50,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);

    // The very first frame already carries the new volume: `C` runs on the
    // note tick, before any audio comes out of it.
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    engine.render(&mut buf);
    assert_eq!(engine.debug_channel_volume(0), 32);
    assert!(
        (buf[0] - level(100, 32)).abs() < 1e-6,
        "C20 must be audible from frame 0; got {}",
        buf[0]
    );

    engine.render(&mut buf);
    assert_eq!(engine.debug_channel_volume(0), 64, "C50 clamps to 64");
}

/// A held note at stored volume 32 with one tremolo command, rendered a tick
/// at a time. Returns the engine; the caller reads the peak per tick.
fn tremolo_from_volume_32(param: u8) -> Engine {
    Engine::new(
        module(
            &[SampleSpec {
                data: square(32, 1, 100),
                volume: 32,
                repeat_start_words: 0,
                repeat_length_words: 16,
            }],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x07,
                param,
            }]],
            &[0],
            1,
        ),
        RATE,
    )
}

/// Asserts the per-tick peak against `level(100, volume)` for each expected
/// stored-volume value in turn.
fn assert_tick_volumes(engine: &mut Engine, expected: &[u8]) {
    let mut buf = vec![0f32; TICK_FRAMES * 2];
    for (tick, expected_volume) in expected.iter().copied().enumerate() {
        engine.render(&mut buf);
        let peak = left(&buf).fold(0f32, |peak, value| peak.max(value.abs()));
        let expected = level(100, expected_volume);
        assert!(
            (peak - expected).abs() < 1e-6,
            "tick {tick}: peak {peak}, expected {expected} (volume {expected_volume})"
        );
    }
}

#[test]
fn tremolo_swings_the_volume_around_the_stored_one() {
    // 7A7: speed 10, amplitude 7, from volume 32. Tremolo shifts the product
    // by 6 where vibrato shifts by 7 (`mt_tre_set` against `mt_vib_set` in
    // `protracker-23f-replay-cia.s`), so an offset is `SINE[pos & 31] * 7 / 64`
    // — twice what the same amplitude gives vibrato. At positions 0, 10, 20, 30
    // and 40 that is 0, +23, +25, +5 and -19, negated past position 32.
    //
    // Amplitude 7 rather than 15 on purpose: 32 +/- 25 leaves headroom at both
    // ends, so this test measures depth and nothing else. The clamp is
    // `tremolo_clamps_the_volume_at_both_ends`.
    let mut engine = tremolo_from_volume_32(0xA7);
    assert_tick_volumes(&mut engine, &[32, 32, 55, 57, 37, 13]);
}

#[test]
fn tremolo_clamps_the_volume_at_both_ends() {
    // 7AF from volume 32. At the full shift-by-6 depth an amplitude-15 offset
    // reaches +/-59, so the same fixture runs off both ends of `0..=64`:
    // positions 10 and 20 want 81 and 87, and position 40 wants -10.
    // `mt_Tremolo3` clamps in a word — `BPL`/`CLR.W` below zero, then
    // `CMP.W #64`/`MOVE.W #64` above 64 — so they sound as 64, 64 and 0.
    //
    // Under the old shift-by-7 reading none of these clamped, which is why the
    // clamp needs pinning now and did not before.
    let mut engine = tremolo_from_volume_32(0xAF);
    assert_tick_volumes(&mut engine, &[32, 32, 64, 64, 43, 0]);
}

// ---------------------------------------------------------------------------
// Pitch effects: the period per tick, and the pitch per waveform cycle
// ---------------------------------------------------------------------------

#[test]
fn arpeggio_steps_base_x_y_with_the_tick_counter() {
    // 047 on a C-2 (period 428, note 12): +4 semitones is note 16 (339) and
    // +7 is note 19 (285). `arptab` is 0, 1, -1 repeating, so the cycle is
    // base, x, y, base, x, y across the row's six ticks.
    let mut engine = Engine::new(held(0x00, 0x47, 1), RATE);
    let mut buf = vec![0f32; TICK_FRAMES * 2];
    for (tick, expected) in [428u16, 339, 285, 428, 339, 285].into_iter().enumerate() {
        engine.render(&mut buf);
        assert_eq!(
            engine.debug_channel_period(0),
            expected,
            "tick {tick} of the arpeggio"
        );
    }
}

#[test]
fn portamento_up_subtracts_once_per_tick_and_stops_at_113() {
    // 102: five steps of 2 a row, so four rows take 428 down to 388.
    let mut engine = Engine::new(held(0x01, 0x02, 8), RATE);
    render_rows(&mut engine, 4);
    assert_eq!(engine.debug_channel_period(0), 428 - 40);

    // And the pitch that comes out follows: 7093789.2 / (2 * 388) / 32 bytes.
    let mut buf = vec![0f32; RATE as usize / 2 * 2];
    let mut engine = Engine::new(held(0x01, 0x00, 64), RATE);
    engine.render(&mut buf);
    let steady = pitch_track(&buf, f64::from(RATE));
    let mean = steady.iter().map(|(_, hz)| hz).sum::<f64>() / steady.len() as f64;
    let expected = 7_093_789.2 / (2.0 * 428.0) / 32.0;
    assert!(
        (mean - expected).abs() / expected < 0.01,
        "a zero parameter must not move the pitch: {mean:.2} Hz, expected {expected:.2}"
    );

    // 1FF clamps at the top of the register rather than wrapping.
    let mut engine = Engine::new(held(0x01, 0xFF, 8), RATE);
    render_rows(&mut engine, 2);
    assert_eq!(engine.debug_channel_period(0), 113);
}

#[test]
fn portamento_down_adds_once_per_tick_and_stops_at_856() {
    // 220: five steps of 32 a row.
    let mut engine = Engine::new(held(0x02, 0x20, 8), RATE);
    render_rows(&mut engine, 1);
    assert_eq!(engine.debug_channel_period(0), 428 + 160);
    render_rows(&mut engine, 2);
    assert_eq!(engine.debug_channel_period(0), 856, "clamped, not wrapped");
}

#[test]
fn tone_portamento_bends_towards_the_note_instead_of_playing_it() {
    // Row 1 carries C-3 (period 214) with 310. `3` is in `prefx_tab`, so the
    // note retargets the slide and never reaches the period register: an
    // implementation that dispatched it from a single table would jump
    // straight to 214 on row 1's first tick.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[vec![
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
                period: 214,
                effect: 0x03,
                param: 0x10,
                ..Cell::default()
            },
            Cell {
                row: 2,
                channel: 0,
                effect: 0x03,
                ..Cell::default()
            },
            Cell {
                row: 3,
                channel: 0,
                effect: 0x03,
                ..Cell::default()
            },
            Cell {
                row: 4,
                channel: 0,
                effect: 0x03,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);

    // Row 1's note tick: still the old period, not the new note.
    let mut a_tick = vec![0f32; TICK_FRAMES * 2];
    render_rows(&mut engine, 1);
    engine.render(&mut a_tick);
    assert_eq!(
        engine.debug_channel_period(0),
        428,
        "the note must retarget the slide, not play"
    );

    // Then 16 a tick: five steps take it to 348 by the end of row 1.
    for _ in 0..5 {
        engine.render(&mut a_tick);
    }
    assert_eq!(engine.debug_channel_period(0), 428 - 80);

    // And it stops on the target rather than sliding past it.
    render_rows(&mut engine, 3);
    assert_eq!(engine.debug_channel_period(0), 214, "arrived, not overshot");
}

#[test]
fn vibrato_runs_at_the_replayers_rate_and_not_the_community_specs() {
    // 4AF held: speed 10, amplitude 15. Vibrato is a `fx_tab` effect, so its
    // position advances on `speed - 1` ticks a row: (10 * 5) / 64 cycles per
    // 120 ms row = 6.51 Hz.
    //
    // The widely-cited community specification says `(x * ticks) / 64`, which
    // would be 7.81 Hz — a 20% error. libxmp agrees with the replayer here and
    // libopenmpt does not, so a consensus of implementations is not available
    // as an authority: the replayer source arbitrates.
    // The tolerance is set from what the engine measures, not guessed: four
    // seconds of render puts all three of these within 0.2% of the formula,
    // and 2% is comfortably inside the 20% that separates the two candidate
    // formulas.
    for (speed_nibble, expected) in [(0xAu8, 6.5104f64), (0x5, 3.2552), (0x4, 2.6042)] {
        let param = (speed_nibble << 4) | 0x0F;
        let mut engine = Engine::new(held(0x04, param, 64), RATE);
        let mut buf = vec![0f32; RATE as usize * 4 * 2];
        engine.render(&mut buf);
        let measured = modulation_hz(&pitch_track(&buf, f64::from(RATE)));
        assert!(
            (measured - expected).abs() / expected < 0.02,
            "vibrato speed {speed_nibble:X}: measured {measured:.3} Hz, \
             expected {expected:.3} +/- 2%"
        );
        // (x * ticks) / 64, the community specification's version.
        let community = expected * 6.0 / 5.0;
        assert!(
            (measured - community).abs() / community > 0.10,
            "measured {measured:.3} Hz is indistinguishable from the community \
             spec's {community:.3} Hz — the tolerance cannot tell them apart"
        );
    }
}

#[test]
fn vibrato_shifts_the_playing_period_without_integrating_it() {
    // The offsets for 4AF at positions 0, 10, 20, 30 and 40 are 0, +24, +27,
    // +5 and -21, applied to 428 and never stored back.
    let mut engine = Engine::new(held(0x04, 0xAF, 8), RATE);
    let mut buf = vec![0f32; TICK_FRAMES * 2];
    for (tick, expected) in [428u16, 428, 452, 455, 433, 407].into_iter().enumerate() {
        engine.render(&mut buf);
        assert_eq!(
            engine.debug_channel_period(0),
            expected,
            "tick {tick} of the vibrato"
        );
    }

    // Four rows of it, then a row with nothing on it. The stored period is
    // re-asserted there, and it is still 428 — a player that wrote the offset
    // back would have integrated the sweep and sailed away by now.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[(0..4)
            .map(|row| Cell {
                row,
                channel: 0,
                sample: if row == 0 { 1 } else { 0 },
                period: if row == 0 { 428 } else { 0 },
                effect: 0x04,
                param: 0xAF,
            })
            .collect()],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 5);
    assert_eq!(
        engine.debug_channel_period(0),
        428,
        "the sweep must not integrate"
    );
}

#[test]
fn vibrato_and_tone_portamento_keep_their_volume_slide_partners() {
    // 5xy is tone portamento plus a volume slide; 6xy is vibrato plus one.
    // Neither reads its parameter as a portamento or vibrato depth — both use
    // what 3xy or 4xy left behind.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            },
            // Set the portamento speed and target.
            Cell {
                row: 1,
                channel: 0,
                period: 214,
                effect: 0x03,
                param: 0x10,
                ..Cell::default()
            },
            // Continue it, and slide the volume down by 1 a tick.
            Cell {
                row: 2,
                channel: 0,
                effect: 0x05,
                param: 0x01,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 3);
    assert_eq!(
        engine.debug_channel_period(0),
        428 - 160,
        "5xy must keep sliding the pitch"
    );
    assert_eq!(
        engine.debug_channel_volume(0),
        64 - 5,
        "and slide the volume five times"
    );

    // 6xy: vibrato from the stored depth, plus the slide.
    let source = module(
        &[SampleSpec {
            data: square(32, 1, 100),
            volume: 64,
            repeat_start_words: 0,
            repeat_length_words: 16,
        }],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x04,
                param: 0xAF,
            },
            Cell {
                row: 1,
                channel: 0,
                effect: 0x06,
                param: 0x01,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 2);
    assert_eq!(engine.debug_channel_volume(0), 64 - 5);
    assert_ne!(
        engine.debug_channel_period(0),
        428,
        "6xy must still be wobbling the pitch"
    );
}

// ---------------------------------------------------------------------------
// Position effects: `Position`, directly
// ---------------------------------------------------------------------------

#[test]
fn a_position_jump_continues_at_the_named_order() {
    let source = module(
        &[SampleSpec::empty()],
        &[
            vec![Cell {
                row: 0,
                channel: 0,
                effect: 0x0B,
                param: 0x02,
                ..Cell::default()
            }],
            vec![],
            vec![],
        ],
        &[0, 1, 2],
        3,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 1);
    let mut one_frame = vec![0f32; 2];
    engine.render(&mut one_frame);
    let at = engine.position();
    assert_eq!(
        (at.order, at.row),
        (2, 0),
        "B02 continues at order 2, row 0"
    );
}

#[test]
fn a_pattern_break_continues_at_the_named_row_of_the_next_order() {
    // Dxy reads its parameter as decimal, because that is how a tracker showed
    // row numbers: D16 is row 16, not row 22.
    let source = module(
        &[SampleSpec::empty()],
        &[
            vec![Cell {
                row: 0,
                channel: 0,
                effect: 0x0D,
                param: 0x16,
                ..Cell::default()
            }],
            vec![],
        ],
        &[0, 1],
        2,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 1);
    let mut one_frame = vec![0f32; 2];
    engine.render(&mut one_frame);
    let at = engine.position();
    assert_eq!((at.order, at.row), (1, 16));
}

#[test]
fn set_speed_takes_ticks_below_0x20_and_tempo_at_or_above_it() {
    // The boundary is the replayer's own `cmp.b #$20 / bhs` (mt_setspeed).
    // The distilled reference says "xy <= 32", which would put $20 on the
    // speed side; the assembly puts it on the tempo side, and the assembly is
    // the behaviour.
    for (param, expected_ms) in [
        // Speed 3 at tempo 125: three 20 ms ticks.
        (0x03u8, 60.0f64),
        // Speed 31, the last value that is a speed at all.
        (0x1F, 620.0),
        // $20 is a tempo of 32: six ticks of 2500/32 = 78.125 ms.
        (0x20, 468.75),
        // Tempo 250: six ticks of 10 ms.
        (0xFA, 60.0),
    ] {
        let mut engine = Engine::new(held(0x0F, param, 64), RATE);
        let mut one_frame = vec![0f32; 2];
        let mut frames = 0usize;
        while engine.position().row == 0 && frames < RATE as usize {
            frames += engine.render(&mut one_frame);
        }
        let ms = frames as f64 * 1_000.0 / f64::from(RATE);
        assert!(
            (ms - expected_ms).abs() < 1.0,
            "F{param:02X}: row lasted {ms:.2} ms, expected {expected_ms:.2}"
        );
    }
}

// ---------------------------------------------------------------------------
// `E` is sixteen effects behind one nibble
// ---------------------------------------------------------------------------

#[test]
fn fine_portamento_moves_the_period_once_a_row() {
    // E12 subtracts 2 a row, not 2 a tick: three rows is 6, not 30.
    let mut engine = Engine::new(held(0x0E, 0x12, 8), RATE);
    render_rows(&mut engine, 3);
    assert_eq!(engine.debug_channel_period(0), 428 - 6);

    // E24 adds 4 a row.
    let mut engine = Engine::new(held(0x0E, 0x24, 8), RATE);
    render_rows(&mut engine, 3);
    assert_eq!(engine.debug_channel_period(0), 428 + 12);
}

#[test]
fn retrigger_restarts_the_sample_on_its_own_tick() {
    // A 400-byte one-shot at 8287.14 bytes/s lasts 48.3 ms — two and a bit
    // ticks. E93 restarts it on tick 3, so there is sound at 60 ms where the
    // control is silent.
    let sample = || SampleSpec {
        data: square(40, 10, 100),
        volume: 64,
        repeat_start_words: 0,
        repeat_length_words: 0,
    };
    let at_tick_three = TICK_FRAMES * 3 * 2;

    let mut engine = Engine::new(
        module(
            &[sample()],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0E,
                param: 0x93,
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    engine.render(&mut buf);
    assert!(
        (buf[at_tick_three] - level(100, 64)).abs() < 1e-6,
        "E93 must restart the sample on tick 3; got {}",
        buf[at_tick_three]
    );

    let mut engine = Engine::new(
        module(
            &[sample()],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                ..Cell::default()
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    engine.render(&mut buf);
    assert_eq!(
        buf[at_tick_three], 0.0,
        "without it the one-shot has finished by tick 3"
    );
}

#[test]
fn fine_volume_slides_move_the_volume_once_a_row() {
    // EA4 adds 4 a row: three rows is 12, not 60.
    let sample = |volume| SampleSpec {
        data: square(32, 1, 100),
        volume,
        repeat_start_words: 0,
        repeat_length_words: 16,
    };
    let rows = |effect_param: u8| {
        (0..8)
            .map(|row| Cell {
                row,
                channel: 0,
                sample: if row == 0 { 1 } else { 0 },
                period: if row == 0 { 428 } else { 0 },
                effect: 0x0E,
                param: effect_param,
            })
            .collect::<Vec<_>>()
    };

    let mut engine = Engine::new(module(&[sample(32)], &[rows(0xA4)], &[0], 1), RATE);
    render_rows(&mut engine, 3);
    assert_eq!(engine.debug_channel_volume(0), 32 + 12);

    let mut engine = Engine::new(module(&[sample(32)], &[rows(0xB4)], &[0], 1), RATE);
    render_rows(&mut engine, 3);
    assert_eq!(engine.debug_channel_volume(0), 32 - 12);
}

#[test]
fn a_note_cut_silences_the_channel_on_its_own_tick() {
    let mut engine = Engine::new(
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
                effect: 0x0E,
                param: 0xC3,
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    engine.render(&mut buf);

    let last_sounding = left(&buf)
        .enumerate()
        .filter(|(_, value)| *value != 0.0)
        .map(|(index, _)| index)
        .last()
        .expect("it must sound before the cut");
    assert!(
        last_sounding < TICK_FRAMES * 3,
        "EC3 must cut at 60 ms; last sound was at frame {last_sounding}"
    );
    assert!(
        last_sounding >= TICK_FRAMES * 3 - 4,
        "and not before it; last sound was at frame {last_sounding}"
    );
    assert_eq!(engine.debug_channel_volume(0), 0);
}

#[test]
fn a_note_delay_holds_the_note_until_its_own_tick() {
    let mut engine = Engine::new(
        module(
            &[SampleSpec {
                data: square(32, 64, 100),
                volume: 64,
                repeat_start_words: 0,
                repeat_length_words: 0,
            }],
            &[vec![Cell {
                row: 0,
                channel: 0,
                sample: 1,
                period: 428,
                effect: 0x0E,
                param: 0xD3,
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    engine.render(&mut buf);

    let first = left(&buf)
        .position(|value| value != 0.0)
        .expect("the delayed note must eventually sound");
    assert_eq!(
        first,
        TICK_FRAMES * 3,
        "ED3 must start the note on tick 3, at frame {}",
        TICK_FRAMES * 3
    );
}

#[test]
fn a_pattern_delay_replays_the_row_without_retriggering_it() {
    // EE1 plays row 0 twice: once for real, once as a delay round in which no
    // note is fetched. Only `fx_tab` runs in the extra round.
    let source = module(
        &[SampleSpec::empty()],
        &[vec![Cell {
            row: 0,
            channel: 0,
            effect: 0x0E,
            param: 0xE1,
            ..Cell::default()
        }]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    render_rows(&mut engine, 1);
    assert_eq!(engine.position().row, 0);
    render_rows(&mut engine, 1);
    assert_eq!(
        engine.position().row,
        0,
        "EE1 holds the row for a second pass"
    );
    render_rows(&mut engine, 1);
    assert_eq!(engine.position().row, 1, "and then moves on");
}

#[test]
fn a_pattern_loop_repeats_the_span_it_marked() {
    // E60 on row 0 marks the start; E61 on row 2 jumps back to it once. The
    // row order is 0, 1, 2, 0, 1, 2, 3.
    let source = module(
        &[SampleSpec::empty()],
        &[vec![
            Cell {
                row: 0,
                channel: 0,
                effect: 0x0E,
                param: 0x60,
                ..Cell::default()
            },
            Cell {
                row: 2,
                channel: 0,
                effect: 0x0E,
                param: 0x61,
                ..Cell::default()
            },
        ]],
        &[0],
        1,
    );
    let mut engine = Engine::new(source, RATE);
    let mut seen = Vec::new();
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    for _ in 0..7 {
        // Read after rendering: before the first call the sequencer has not
        // started, so its position is a row that has not played yet.
        engine.render(&mut buf);
        seen.push(engine.position().row);
    }
    assert_eq!(seen, vec![0, 1, 2, 0, 1, 2, 3]);
}

// ---------------------------------------------------------------------------
// No panic on any input
// ---------------------------------------------------------------------------

#[test]
fn no_effect_and_no_parameter_can_make_the_engine_panic() {
    // Every effect number against parameters chosen to hit each one's edges:
    // a zero target for tone portamento, an offset past the end of the sample,
    // a speed of zero, a pattern break past the end of a pattern, a position
    // jump past the end of the song.
    let mut sounded = 0usize;
    for effect in 0u8..16 {
        for param in [0x00u8, 0x01, 0x0F, 0x10, 0x1F, 0x20, 0x7F, 0x80, 0xC3, 0xFF] {
            let source = module(
                &[SampleSpec {
                    data: square(32, 1, 100),
                    volume: 64,
                    repeat_start_words: 0,
                    repeat_length_words: 16,
                }],
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
                &[0],
                1,
            );
            let mut engine = Engine::new(source, RATE);
            let mut buf = vec![0f32; RATE as usize / 4 * 2];
            assert_eq!(engine.render(&mut buf), RATE as usize / 4);
            assert_eq!(
                buf.iter()
                    .filter(|value| !value.is_finite() || value.abs() > 1.0)
                    .count(),
                0,
                "effect {effect:X}{param:02X}: every frame must be finite and \
                 inside full scale"
            );
            if buf.iter().any(|value| *value != 0.0) {
                sounded += 1;
            }
        }
    }
    // The sweep must actually reach the mixer, or it proves only that nothing
    // crashed and would keep passing against an engine that rendered silence.
    assert!(
        sounded > 100,
        "only {sounded} of 160 effect/parameter pairs made any sound"
    );
}

#[test]
fn a_sample_offset_past_the_end_leaves_one_word_rather_than_reading_past_it() {
    // 9FF asks for byte 65280 of a 32-byte sample. The replayer sets the
    // length to one word instead of clamping the start, so a very short blip
    // plays from where it already was.
    let mut engine = Engine::new(
        module(
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
                effect: 0x09,
                param: 0xFF,
            }]],
            &[0],
            1,
        ),
        RATE,
    );
    let mut buf = vec![0f32; ROW_FRAMES * 2];
    engine.render(&mut buf);
    let last = left(&buf)
        .enumerate()
        .filter(|(_, value)| *value != 0.0)
        .map(|(index, _)| index)
        .last()
        .expect("two bytes still make a sound");
    // Two bytes at 8287.14 bytes/s is 0.241 ms, which is 10.6 frames.
    assert!(
        last < 16,
        "one word is 0.24 ms; sound ran to frame {last} instead"
    );
}
