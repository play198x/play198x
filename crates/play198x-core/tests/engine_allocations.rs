//! `render` must allocate nothing.
//!
//! An engine that allocates inside an audio callback glitches on somebody
//! else's machine and never on yours, so this is measured rather than argued
//! from reading the code.
//!
//! Its own test binary on purpose. A counting global allocator is
//! binary-wide state, and cargo runs a binary's tests concurrently: sharing
//! one with the rest of the engine tests would count their allocations too
//! and turn an exact assertion into a flaky one.
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

#[test]
fn rendering_a_second_of_audio_allocates_nothing() {
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
