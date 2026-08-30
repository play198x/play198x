//! The seam where a 50Hz frame meets a 128-sample worklet quantum.
//!
//! This is the most load-bearing test file in the player: a seam that drops,
//! repeats or reorders a sample is not a subtle wrongness, it is an audible
//! click at 50Hz, and it would be blamed on the tune rather than on the pump.
//!
//! The fixture emits a **ramp** rather than a tone, so every sample is
//! distinguishable from every other. A tone would let a dropped or duplicated
//! sample hide inside a periodic waveform, which is precisely the defect
//! being hunted.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use play198x_core::player::pump::{FramePump, FrameSource};
use play198x_core::player::{Player, Position};

/// Emits `n .. n + samples_per_frame` on frame `n`, as identical left and
/// right channels, so the concatenated output of any sequence of renders is
/// checkable against a plain counter.
struct Ramp {
    next: f32,
    per_frame: usize,
    /// Frame lengths to walk through, one per `frame()` call, cycling. A
    /// single-entry list is a fixed-rate source; a longer one is a source
    /// whose rate changes between frames, which SID does.
    lengths: Vec<usize>,
    frames_run: usize,
}

impl Ramp {
    fn fixed(per_frame: usize) -> Self {
        Self {
            next: 0.0,
            per_frame,
            lengths: vec![per_frame],
            frames_run: 0,
        }
    }

    fn varying(lengths: &[usize]) -> Self {
        Self {
            next: 0.0,
            per_frame: lengths[0],
            lengths: lengths.to_vec(),
            frames_run: 0,
        }
    }
}

impl FrameSource for Ramp {
    fn frame(&mut self) {
        self.per_frame = self.lengths[self.frames_run % self.lengths.len()];
        self.frames_run += 1;
    }

    fn render_frame(&mut self, out: &mut [f32]) -> usize {
        for frame in out.as_chunks_mut::<2>().0.iter_mut().take(self.per_frame) {
            frame[0] = self.next;
            frame[1] = self.next;
            self.next += 1.0;
        }
        self.per_frame
    }

    fn samples_per_frame(&self) -> usize {
        self.per_frame
    }
}

/// Every left-channel sample the pump produced, across `calls` renders of
/// `frames` each.
fn drain(pump: &mut FramePump<Ramp>, frames: usize, calls: usize) -> Vec<f32> {
    let mut got = Vec::new();
    let mut out = vec![0.0f32; frames * 2];
    for _ in 0..calls {
        out.fill(f32::NAN);
        let written = pump.render(&mut out);
        assert_eq!(written, frames, "the pump must fill the whole request");
        got.extend(out.as_chunks::<2>().0.iter().map(|f| f[0]));
    }
    got
}

#[test]
fn the_seam_neither_drops_nor_repeats_a_sample() {
    // 960 is one frame at 48kHz; 128 is the Web Audio render quantum. Their
    // ratio is deliberately not whole — 7.5 quanta to a frame — so most
    // requests straddle a frame boundary rather than landing on one.
    let mut pump = FramePump::new(Ramp::fixed(960));
    let got = drain(&mut pump, 128, 40);

    let expected: Vec<f32> = (0..got.len()).map(|i| i as f32).collect();
    assert_eq!(
        got, expected,
        "the pump must emit the source's samples in order, once each"
    );
}

#[test]
fn a_request_larger_than_a_frame_spans_frames() {
    // The opposite ordering: one request needs several frames pumped to fill
    // it, rather than one frame serving several requests.
    let mut pump = FramePump::new(Ramp::fixed(100));
    let got = drain(&mut pump, 512, 3);

    let expected: Vec<f32> = (0..got.len()).map(|i| i as f32).collect();
    assert_eq!(got, expected);
}

#[test]
fn a_request_of_exactly_one_frame_is_not_a_special_case() {
    let mut pump = FramePump::new(Ramp::fixed(256));
    let got = drain(&mut pump, 256, 4);

    let expected: Vec<f32> = (0..got.len()).map(|i| i as f32).collect();
    assert_eq!(got, expected);
}

#[test]
fn a_source_whose_frame_length_changes_still_joins_up() {
    // A SID driven by a CIA timer does not produce a constant number of
    // samples per frame. If the pump caches the first answer, this test sees
    // it as a gap or an overlap in the ramp.
    let mut pump = FramePump::new(Ramp::varying(&[960, 800, 1_000, 799]));
    let got = drain(&mut pump, 128, 60);

    let expected: Vec<f32> = (0..got.len()).map(|i| i as f32).collect();
    assert_eq!(got, expected);
}

#[test]
fn a_paused_pump_renders_silence_in_full() {
    let mut pump = FramePump::new(Ramp::fixed(960));
    pump.set_playing(false);

    let mut out = vec![1.0f32; 128 * 2];
    assert_eq!(pump.render(&mut out), 128);
    assert!(
        out.iter().all(|s| *s == 0.0),
        "a paused pump owes exact zeroes, in full"
    );
}

#[test]
fn the_position_is_the_song_and_the_frame() {
    let mut pump = FramePump::new(Ramp::fixed(960));
    assert_eq!(pump.position(), Position::Frame { song: 0, frame: 0 });

    // Seven quanta is 896 samples, inside a single 960-sample frame. The
    // counter must read 1, not 7: it counts frames the source ran, not times
    // the worklet asked. That distinction is the whole reason it is here —
    // a counter that tracked render calls would report a tune's progress at
    // 7.5x its real speed.
    drain(&mut pump, 128, 7);
    assert_eq!(pump.position(), Position::Frame { song: 0, frame: 1 });

    // Two more crosses into the second frame, and only then does it tick.
    drain(&mut pump, 128, 2);
    assert_eq!(pump.position(), Position::Frame { song: 0, frame: 2 });
}
