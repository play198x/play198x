//! The one contract every player satisfies, and the players themselves.
//!
//! # Why a trait rather than one type per format
//!
//! A player's consumer — a worklet, a desktop transport, a command-line
//! renderer — wants to hold one thing and call one set of methods. What it
//! must not have to know is which format it is holding: that knowledge would
//! have to be repeated at every call site, and every new format would have to
//! visit all of them.
//!
//! The contract is small on purpose. [`Player::render`] is nearly all of it,
//! and its rule is **ask for `n` frames, get `n` frames** — see that method
//! for why "get fewer" is not an option a caller can be asked to handle.
//!
//! # The two shapes underneath
//!
//! A tracker engine keeps its own clock and can fill any request. A tune that
//! is really a program cannot: it runs one interrupt, produces one frame's
//! worth of samples, and must be asked again. Those are reconciled by
//! [`pump::FramePump`], not by the caller and not by this trait — see that
//! module for why the seam belongs there rather than in either.

pub mod pump;

#[cfg(feature = "ay")]
pub mod ay;

use crate::engine::ModulePosition;

/// Anything that can be played.
///
/// Deliberately narrow. Seeking is absent because not every format has it —
/// an `.ay` song is an entry point with its own register state, not an offset
/// into a stream — and a trait method that half its implementations have to
/// refuse is worse than no method.
pub trait Player {
    /// Fill `out` with interleaved stereo, returning the frames written.
    ///
    /// **Ask for `n` frames, get `n` frames.** `out.len() / 2` is the request;
    /// the return value is a convenience, not a licence to write less. A
    /// caller that is handed fewer samples than it asked for has no good
    /// option: a worklet must emit a full quantum or the output clicks, so it
    /// would have to zero the tail itself, and then every caller carries that
    /// same fix.
    ///
    /// A paused player renders exact zeroes for the whole request rather than
    /// returning early, for the same reason.
    fn render(&mut self, out: &mut [f32]) -> usize;

    /// Start or stop the transport. A stopped player still renders silence.
    fn set_playing(&mut self, playing: bool);

    /// Where playback has got to, in whichever terms the format has.
    fn position(&self) -> Position;
}

/// Where a player has got to.
///
/// One variant per *shape* of position rather than per format: a tracker
/// module has an order table and rows, and everything driven by a 50Hz-ish
/// interrupt has a song index and a frame counter. Which format is playing
/// is already answered by [`crate::probe::identify`] and by the metadata, and
/// stating it a second time here would be a second place for it to drift.
///
/// Not `#[non_exhaustive]`: a genuinely new *shape* of position is a decision
/// every interface showing one should be made to take, which is the same
/// argument [`crate::metadata::Metadata`] makes for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Position {
    /// A tracker module, walking an order table.
    Module(ModulePosition),
    /// A tune driven one frame at a time — `.ay` today, SID and the rest as
    /// they arrive. `frame` counts frames played since the song started.
    Frame { song: usize, frame: u32 },
}
