//! The one contract every player satisfies, whatever it is playing.
//!
//! These tests drive `Engine` through `&mut dyn Player` rather than through
//! its inherent methods. That is the point: the worklet on the other side of
//! the wasm boundary must be able to hold one thing and call one set of
//! methods, without knowing whether it is playing a tracker module or a tune
//! that is really a Z80 program. A test calling `Engine::render` directly
//! would pass whether or not the trait was implemented usefully.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Cell, SampleSpec, module, square};
use play198x_core::engine::Engine;
use play198x_core::player::{Player, Position};

const RATE: u32 = 48_000;

/// One channel, one note, one order — the smallest module that produces
/// sound and advances a position.
fn one_note() -> format198x_commodore_amiga_mod::Module {
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

#[test]
fn an_engine_is_a_player() {
    let mut engine = Engine::new(one_note(), RATE);
    let player: &mut dyn Player = &mut engine;

    // The whole contract: ask for n frames, get n frames.
    let mut out = vec![0.0f32; 128 * 2];
    assert_eq!(player.render(&mut out), 128);
}

#[test]
fn a_module_reports_where_it_has_got_to() {
    let mut engine = Engine::new(one_note(), RATE);
    let player: &mut dyn Player = &mut engine;

    // Before anything renders, a module sits at the top of its order table.
    match player.position() {
        Position::Module(at) => {
            assert_eq!(at.order, 0);
            assert_eq!(at.row, 0);
            assert_eq!(at.tick, 0);
        }
        other => panic!("a module should report a module position, got {other:?}"),
    }
}

#[test]
fn a_paused_player_still_fills_the_whole_request() {
    let mut engine = Engine::new(one_note(), RATE);
    let player: &mut dyn Player = &mut engine;
    player.set_playing(false);

    // Not `0`: a worklet callback handed fewer samples than it asked for is a
    // worklet callback that clicks. A paused player owes silence, in full.
    let mut out = vec![1.0f32; 128 * 2];
    assert_eq!(player.render(&mut out), 128);
    assert!(
        out.iter().all(|s| *s == 0.0),
        "a paused player must render exact zeroes, not stale buffer contents"
    );
}
