//! Every `render` in this crate must allocate nothing.
//!
//! An engine that allocates inside an audio callback glitches on somebody
//! else's machine and never on yours, so this is measured rather than argued
//! from reading the code.
//!
//! Its own test binary on purpose. A counting global allocator is
//! binary-wide state, and cargo runs a binary's tests concurrently: sharing
//! one with the rest of the engine tests would count their allocations too
//! and turn an exact assertion into a flaky one.
//!
//! That is also why this binary holds **one** `#[test]`, measuring both
//! renderers in turn rather than one test each. Two `#[test]`s here would
//! run concurrently and count each other, which is the same failure from
//! the inside.
#![allow(unsafe_code, clippy::unwrap_used)]

mod common;

use common::{Cell, SampleSpec, module, square};
use play198x_core::engine::Engine;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// The system allocator, plus a tally of every request that hands out memory.
struct Counting;

// SAFETY: every method forwards to `System` unchanged with the same layout and
// pointer it was given, and only adds a relaxed counter increment. The counter
// is not read by the allocator, so it cannot affect allocation behaviour.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Both renderers keep the contract, measured one at a time. Splitting
/// them into two `#[test]`s would let cargo run them at once against one
/// binary-wide counter — see the module doc.
#[test]
fn rendering_allocates_nothing() {
    engine_render_allocates_nothing();
    #[cfg(feature = "ay")]
    ay_render_allocates_nothing();
}

fn engine_render_allocates_nothing() {
    let source = module(
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
    );
    let mut engine = Engine::new(source, 44_100);
    let mut buf = vec![0f32; 44_100 * 2];

    // The first call is measured too. Anything lazily built on first render is
    // exactly the bug this test is for: it would allocate once, on the audio
    // thread, in the worst possible place.
    for call in 1..=3 {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        let frames = engine.render(&mut buf);
        let after = ALLOCATIONS.load(Ordering::Relaxed);
        assert_eq!(frames, 44_100);
        assert_eq!(
            after - before,
            0,
            "render call {call} made {} allocations; it must make none",
            after - before
        );
    }
}

/// `AyPlayer` keeps the same contract as `Engine`, and for the same reason:
/// both end up in an audio callback. `AyPlayer::render` returns a frame
/// count and reuses a buffer sized once at construction rather than building
/// one per call — 50 allocations a second, on the audio thread, otherwise.
///
/// `frame()` is measured alongside it. It runs the CPU rather than the
/// mixer, but it runs in the same callback, so an allocation there costs
/// exactly as much.
#[cfg(feature = "ay")]
fn ay_render_allocates_nothing() {
    use play198x_core::player::ay::AyPlayer;

    // An init that returns immediately and an interrupt that programs one
    // AY channel, so the chip has real work to render rather than silence.
    let mut code = vec![0u8; 0x80];
    code[0x00] = 0xC9; // RET
    let mut at = 0x10;
    for (reg, val) in [(0u8, 0x00u8), (1, 0x01), (7, 0x3E), (8, 0x0F)] {
        code[at..at + 12].copy_from_slice(&[
            0x01, 0xFD, 0xFF, 0x3E, reg, 0xED, 0x79, 0x01, 0xFD, 0xBF, 0x3E, val,
        ]);
        code[at + 12] = 0xED;
        code[at + 13] = 0x79;
        at += 14;
    }
    code[at] = 0xC9; // RET

    let bytes = common::build_ay(0x8000, 0x8010, 0x8000, &code);
    let mut player = AyPlayer::new(&bytes, 0, 44_100).unwrap();
    let mut buf = vec![0f32; 44_100 / 50 * 2];

    // The first call is measured too: anything built lazily on first render
    // allocates once, on the audio thread, in the worst possible place.
    for call in 1..=3 {
        let before = ALLOCATIONS.load(Ordering::Relaxed);
        player.frame();
        let frames = player.render(&mut buf);
        let after = ALLOCATIONS.load(Ordering::Relaxed);
        assert_eq!(frames, 44_100 / 50);
        assert_eq!(
            after - before,
            0,
            "ay render call {call} made {} allocations; it must make none",
            after - before
        );
    }
}
