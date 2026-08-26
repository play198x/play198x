//! The ProTracker engine: sequencer, mixer, transport and timing.
//!
//! The engine never owns an audio device. A caller pulls frames from it, which
//! is what lets the same code drive a desktop audio callback, a WebAudio worklet
//! and a numerical comparison against another replayer.

mod effects;
