//! Turning a player that thinks in frames into one that fills any request.
//!
//! # The mismatch this exists for
//!
//! A tune that is really a program produces audio a frame at a time: run one
//! interrupt, drain the sound chip, and you have 960 stereo frames at 48kHz —
//! never 128, which is what a Web Audio worklet asks for.
//!
//! Something has to reconcile those, and **where** it goes is the decision.
//! Put it in the worklet and the one piece of code that should not know which
//! format is playing learns that `.ay` arrives in lumps; every later
//! frame-driven format then has to agree with that arrangement. Put it in the
//! `.ay` player and SID, NSF and SAP each write it again — four chances to
//! get a seam wrong that is audible as a click at 50Hz when you do.
//!
//! So it goes here, once, behind [`FrameSource`]: a format says how to run one
//! frame, and inherits the reconciliation and the tests that hold it down.
//!
//! # Why not just render a frame per callback
//!
//! Because the callback's size is not ours to choose. The Web Audio render
//! quantum is fixed by the spec at 128 samples, and 960 does not divide by it
//! evenly at every sample rate — 44,100 gives 882 samples a frame, which is
//! 6.89 quanta. Any scheme that assumes a whole-number relationship is a
//! scheme that drifts.

use super::{Player, Position};

/// Something that produces audio exactly one frame at a time.
///
/// Implement this for a format whose player is driven by an interrupt rather
/// than by a clock of its own, and [`FramePump`] makes it a [`Player`].
pub trait FrameSource {
    /// Advance one frame: run the tune's interrupt routine, or apply the next
    /// register dump, or whatever this format does once per frame.
    fn frame(&mut self);

    /// Fill `out` with this frame's interleaved stereo, returning the frames
    /// written. Called once per [`Self::frame`], with a buffer at least
    /// [`Self::samples_per_frame`] frames long.
    fn render_frame(&mut self, out: &mut [f32]) -> usize;

    /// How many stereo frames the frame just run will produce.
    ///
    /// **Asked after every [`Self::frame`], never cached across frames.** A
    /// SID driven by a CIA timer does not run at a fixed rate, and a pump that
    /// remembered the first answer would be the reason that format had to be
    /// special-cased. Answering with a constant is fine; being *asked* every
    /// time is what keeps the varying case ordinary.
    fn samples_per_frame(&self) -> usize;

    /// Which subtune is playing.
    ///
    /// Defaults to 0, which is the whole answer for a format that carries one
    /// tune per file — a register dump has no subtunes to choose between.
    /// `.ay` and SID override it. It lives here rather than on the pump
    /// because the subtune is a property of what is playing, not of the
    /// buffering in front of it.
    fn song(&self) -> usize {
        0
    }
}

/// Serves any request from a [`FrameSource`], a frame at a time.
pub struct FramePump<S: FrameSource> {
    source: S,
    /// The current frame's samples, interleaved. Grown only when a frame
    /// arrives longer than any seen so far, so the steady state allocates
    /// nothing — which matters because this runs on the audio thread.
    frame: Vec<f32>,
    /// How much of `frame` holds this frame's samples. Not `frame.len()`:
    /// the buffer keeps its capacity when a shorter frame follows a longer
    /// one, so its length would overstate what is valid.
    filled: usize,
    /// Read cursor into `frame`, in stereo frames.
    cursor: usize,
    playing: bool,
    frames_run: u32,
}

impl<S: FrameSource> FramePump<S> {
    /// Wrap a source. Nothing is rendered until the first [`Player::render`].
    pub fn new(source: S) -> Self {
        Self {
            source,
            frame: Vec::new(),
            filled: 0,
            cursor: 0,
            playing: true,
            frames_run: 0,
        }
    }

    /// The source underneath, for the format-specific things a `Player` does
    /// not expose.
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Run one frame and refill the buffer from it.
    fn pump(&mut self) {
        self.source.frame();
        self.frames_run = self.frames_run.saturating_add(1);

        let wanted = self.source.samples_per_frame();
        if self.frame.len() < wanted * 2 {
            self.frame.resize(wanted * 2, 0.0);
        }
        self.filled = self.source.render_frame(&mut self.frame[..wanted * 2]);
        self.cursor = 0;
    }
}

impl<S: FrameSource> Player for FramePump<S> {
    fn render(&mut self, out: &mut [f32]) -> usize {
        let wanted = out.len() / 2;

        if !self.playing {
            // Exact zeroes for the whole request, per the trait's rule: a
            // worklet handed a short buffer clicks, and a stale one screeches.
            out[..wanted * 2].fill(0.0);
            return wanted;
        }

        let mut written = 0;
        while written < wanted {
            if self.cursor >= self.filled {
                self.pump();
                // A source that yields nothing would spin here forever.
                // Treat it as silence rather than hanging the audio thread.
                if self.filled == 0 {
                    out[written * 2..wanted * 2].fill(0.0);
                    return wanted;
                }
            }

            let take = (self.filled - self.cursor).min(wanted - written);
            let src = self.cursor * 2..(self.cursor + take) * 2;
            let dst = written * 2..(written + take) * 2;
            out[dst].copy_from_slice(&self.frame[src]);

            self.cursor += take;
            written += take;
        }
        written
    }

    fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    fn position(&self) -> Position {
        Position::Frame {
            song: self.source.song(),
            frame: self.frames_run,
        }
    }
}
