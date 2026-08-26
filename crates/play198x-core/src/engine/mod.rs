//! The ProTracker engine: sequencer, mixer, transport and timing.
//!
//! The engine never owns an audio device. A caller pulls frames from it, which
//! is what lets the same code drive a desktop audio callback, a WebAudio worklet
//! and a numerical comparison against another replayer.
//!
//! # The two numbers everything else rests on
//!
//! A tick lasts `2500 / tempo` milliseconds and a row is `speed` ticks, so at
//! ProTracker's power-on defaults of speed 6 and tempo 125 a tick is 20 ms and
//! a row is 120 ms. A sample plays at `7_093_789.2 / (2 * period)` bytes per
//! second, which puts C-2 (period 428) at 8287.14 Hz. Both are measured in
//! `tests/engine.rs` rather than asserted about the source, because either one
//! being wrong makes the whole engine wrong by a constant factor that reads as
//! plausible music.
//!
//! # Shape, and why it is this shape
//!
//! [`Engine::advance_tick`] is the single place the sequence moves. Effects
//! (the next task) hang off the two branches inside it, and computing a
//! module's duration walks it with the mixer switched off — the same code
//! path, not a second implementation that can drift from what playback does.
//!
//! Per-tick effects will therefore run on `speed - 1` ticks of each row, not
//! `speed`: the replayer's `mt_music` calls `mt_checkfx` only when the tick
//! counter has *not* wrapped, so the note tick is not one of them. See
//! `reference/by-topic/music-formats/protracker-playback-reference.md`.

mod effects;

use format198x_commodore_amiga_mod::{Module, Note, ROWS_PER_PATTERN};

/// PAL Paula's clock, in Hz. `rate = clock / (2 * period)`.
///
/// NTSC machines ran at 7_159_090.5, about 0.9% higher. PAL is the authentic
/// figure for this music: the trackers, and nearly all of the modules, are
/// European.
const PAULA_CLOCK_PAL: f64 = 7_093_789.2;

/// Ticks per row at power-on.
const DEFAULT_SPEED: u8 = 6;

/// Beats per minute at power-on. A tick is `2500 / tempo` milliseconds.
const DEFAULT_TEMPO: u16 = 125;

/// Milliseconds in the tick numerator: `tick_ms = TICK_MS_NUMERATOR / tempo`.
const TICK_MS_NUMERATOR: f64 = 2_500.0;

/// The only row shape this crate plays. `Module` is four-channel by
/// construction — the decoder refuses `6CHN`, `8CHN` and `FLT8` — so this is a
/// constant rather than something read back off each module.
const CHANNELS: usize = format198x_commodore_amiga_mod::CHANNELS;

/// Amiga panning: voices 0 and 3 to the left, 1 and 2 to the right.
///
/// Hard, as the hardware is. No stereo separation control here: a blend is a
/// taste decision belonging to whatever wraps this, and putting a default one
/// in the mixer would silently change what the differential harness compares.
const PANNING: [(f32, f32); CHANNELS] = [(1.0, 0.0), (0.0, 1.0), (0.0, 1.0), (1.0, 0.0)];

/// Per-voice gain. Hard panning means at most two voices reach either side, so
/// halving each is the exact bound that cannot clip: two full-scale voices on
/// one side sum to 1.0 and no further.
const VOICE_GAIN: f32 = 0.5;

/// ProTracker's volume ceiling. The header field is a whole byte and the
/// decoder does not clamp it, so the mixer does.
const MAX_VOLUME: u8 = 64;

/// Where playback has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// Index into the order table's played prefix — `0..song_length`.
    pub order: usize,
    /// The pattern that order names. May be past the end of the patterns the
    /// file actually holds, which is a thing real files do; such a row plays
    /// as silence and takes its normal time.
    pub pattern: usize,
    /// Row within the pattern, `0..64`.
    pub row: usize,
    /// Tick within the row, `0..speed`.
    pub tick: u8,
}

/// One of the four voices.
#[derive(Debug, Clone, Copy)]
struct Voice {
    /// Which sample slot is selected, if a note has ever chosen one.
    sample: Option<usize>,
    /// Whether it is currently sounding.
    active: bool,
    /// Amiga period. Kept even when silent: a period with no sample number
    /// replays the selected sample, and a sample number with no period sets
    /// volume without retriggering.
    period: u16,
    volume: u8,
    /// Playback position in bytes, fractional between output frames.
    position: f64,
    /// Bytes of sample data consumed per output frame.
    step: f64,
    /// Byte offset this pass ends at.
    end: usize,
    loop_start: usize,
    /// Loop length in bytes, `0` when the sample does not loop.
    loop_len: usize,
}

impl Voice {
    const fn silent() -> Self {
        Self {
            sample: None,
            active: false,
            period: 0,
            volume: 0,
            position: 0.0,
            step: 0.0,
            end: 0,
            loop_start: 0,
            loop_len: 0,
        }
    }

    /// One frame of this voice, before panning.
    ///
    /// Nearest neighbour, deliberately: it is what Paula does, and the plan
    /// settles interpolation as a later change that has to be measured rather
    /// than a default that arrives unannounced.
    fn next_sample(&mut self, data: &[u8]) -> f32 {
        if !self.active {
            return 0.0;
        }

        // `as usize` on an f64 saturates in Rust rather than being undefined,
        // and `position` is bounded by `end` regardless, which is bounded by
        // `data.len()`. The `get` is belt and braces on an FFI-facing path.
        let value = data.get(self.position as usize).map_or(0, |b| *b as i8);
        let out = f32::from(value) / 128.0 * f32::from(self.volume) / f32::from(MAX_VOLUME);

        self.position += self.step;
        if self.position >= self.end as f64 {
            if self.loop_len == 0 {
                self.active = false;
            } else {
                // Paula reloads the pointer from the loop registers, so the
                // overshoot carries across the wrap. Modulo rather than a
                // subtraction: a loop shorter than one output frame's step
                // would otherwise still be past its end after wrapping.
                let overshoot = self.position - self.end as f64;
                self.position = self.loop_start as f64 + overshoot % self.loop_len as f64;
                self.end = self.loop_start + self.loop_len;
            }
        }
        out
    }
}

/// A pull-based frame source for one module.
///
/// Construct it with [`Engine::new`] and pull frames with [`Engine::render`].
/// Nothing here allocates after construction, and nothing here panics: a
/// module with a zero-length sample, a loop pointing past the end of its data
/// or an order naming a pattern the file does not contain all render as
/// something, because this crate sits behind an FFI boundary where unwinding
/// is undefined behaviour.
pub struct Engine {
    module: Module,
    /// Output frames per second. Clamped away from zero at construction so the
    /// tick arithmetic can never divide by it.
    sample_rate: f64,

    voices: [Voice; CHANNELS],

    playing: bool,
    /// Ticks per row. A field rather than a constant because `Fxy` sets it.
    speed: u8,
    /// Beats per minute. Likewise.
    tempo: u16,
    /// Frames left before the next tick. Fractional, so a tick length that is
    /// not a whole number of frames does not accumulate drift.
    frames_to_next_tick: f64,

    order: usize,
    row: usize,
    tick: u8,
    /// Set when the sequencer must (re)start a row without stepping to the
    /// next one first: at construction, and after a seek.
    row_pending: bool,
}

impl Engine {
    /// Load `module` and start it at the top of the order table, playing.
    ///
    /// A `sample_rate` of zero is treated as 1 Hz. It is nonsense either way,
    /// but this crate does not get to panic about it.
    #[must_use]
    pub fn new(module: Module, sample_rate: u32) -> Self {
        let sample_rate = f64::from(sample_rate.max(1));
        Self {
            module,
            sample_rate,
            voices: [Voice::silent(); CHANNELS],
            playing: true,
            speed: DEFAULT_SPEED,
            tempo: DEFAULT_TEMPO,
            frames_to_next_tick: 0.0,
            order: 0,
            row: 0,
            tick: 0,
            row_pending: true,
        }
    }

    /// Fill `out` with interleaved stereo frames, returning how many it wrote.
    ///
    /// A caller with an odd-length buffer gets whole frames and the odd slot
    /// back untouched. **Allocates nothing** — `tests/engine_allocations.rs`
    /// counts, because an engine that allocates on the audio thread glitches
    /// on somebody else's machine and never on yours.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        let mut frames = 0;
        // `as_chunks_mut` rather than `chunks_exact_mut(2)`: a stereo frame
        // is a fixed pair, and the typed form drops a bounds check per sample.
        for frame in out.as_chunks_mut::<2>().0 {
            let (left, right) = if self.playing {
                if self.frames_to_next_tick <= 0.0 {
                    self.advance_tick();
                    self.frames_to_next_tick += self.frames_per_tick();
                }
                let mixed = self.mix_frame();
                self.frames_to_next_tick -= 1.0;
                mixed
            } else {
                (0.0, 0.0)
            };
            frame[0] = left;
            frame[1] = right;
            frames += 1;
        }
        frames
    }

    /// Where playback has got to.
    #[must_use]
    pub fn position(&self) -> Position {
        Position {
            order: self.order,
            pattern: self.current_pattern(),
            row: self.row,
            tick: self.tick,
        }
    }

    /// Start or pause. A paused engine renders exact zeroes and holds both its
    /// position and its clock, so resuming continues the row it stopped in.
    pub fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Jump to the top of an order, clamped to the played prefix.
    ///
    /// Cuts the sounding voices. A `Bxx` position jump inside a song does not
    /// — the notes carry over — but a listener dragging a scrub bar is asking
    /// for the music at the new place, not a note held over from the old one.
    pub fn seek_order(&mut self, order_index: usize) {
        let orders = self.module.orders().len();
        if orders == 0 {
            return;
        }
        self.order = order_index.min(orders - 1);
        self.row = 0;
        self.tick = 0;
        self.row_pending = true;
        self.frames_to_next_tick = 0.0;
        self.voices = [Voice::silent(); CHANNELS];
    }

    /// Output frames in one tick: `sample_rate * (2500 / tempo) / 1000`.
    ///
    /// Recomputed each tick rather than cached, so that the tempo effect
    /// arriving in the next task cannot leave a stale figure behind. One
    /// division per 20 ms is not a cost worth a cache-invalidation bug.
    fn frames_per_tick(&self) -> f64 {
        // `max(1)` because `Fxy` can name a tempo of zero, and dividing by it
        // would produce an infinite tick rather than a refused one.
        self.sample_rate * TICK_MS_NUMERATOR / f64::from(self.tempo.max(1)) / 1_000.0
    }

    /// The pattern the current order names, or 0 when there is no order to
    /// read — a `song_length` of zero, which is a module with nothing to play.
    fn current_pattern(&self) -> usize {
        self.module
            .orders()
            .get(self.order)
            .map_or(0, |pattern| usize::from(*pattern))
    }

    /// Move the sequence on by one tick.
    ///
    /// The one place playback advances. Task 7's effect tables hang off the
    /// two branches below, and duration measurement walks this with the mixer
    /// switched off rather than reimplementing it.
    fn advance_tick(&mut self) {
        if self.module.orders().is_empty() {
            return;
        }

        if self.row_pending {
            self.row_pending = false;
            self.tick = 0;
            self.start_row();
            return;
        }

        self.tick += 1;
        // `max(1)` because `F00` can set the speed to zero, and a row of zero
        // ticks would otherwise stall the comparison rather than the song.
        if self.tick >= self.speed.max(1) {
            self.tick = 0;
            self.step_row();
            self.start_row();
        }
        // Otherwise this is a plain tick: `fx_tab` effects run here, which is
        // why they run `speed - 1` times per row. Task 7.
    }

    /// Step to the next row, wrapping through the order table.
    ///
    /// ProTracker 2.3 restarts a finished song at order 0 and ignores the
    /// module's restart byte, which is a Noisetracker leftover that real files
    /// routinely leave at 127 (`mt_nextposition`, `song_step`).
    fn step_row(&mut self) {
        self.row += 1;
        if self.row >= ROWS_PER_PATTERN {
            self.row = 0;
            self.order += 1;
            if self.order >= self.module.orders().len() {
                self.order = 0;
            }
        }
    }

    /// Act on the row the sequencer has arrived at.
    fn start_row(&mut self) {
        let pattern = self.current_pattern();
        // An order naming a pattern the file does not hold plays as silence
        // and takes its normal time. Real files carry garbage in the unplayed
        // tail of the order table; refusing to play, or reading past the end,
        // are both worse answers than a quiet row.
        let Some(row) = self
            .module
            .patterns
            .get(pattern)
            .and_then(|pattern| pattern.get(self.row))
        else {
            return;
        };
        // Copied out so the voices can be borrowed mutably below. `Note` is
        // 6 bytes and `Copy`; this is a register shuffle, not an allocation.
        let notes: [Note; CHANNELS] = *row;

        for (voice, note) in self.voices.iter_mut().zip(notes) {
            trigger(voice, note, &self.module, self.sample_rate);
        }
    }

    /// One frame of the mix, panned.
    fn mix_frame(&mut self) -> (f32, f32) {
        let mut left = 0.0;
        let mut right = 0.0;
        for (index, voice) in self.voices.iter_mut().enumerate() {
            let value = match voice.sample {
                Some(slot) => match self.module.samples.get(slot) {
                    Some(sample) => voice.next_sample(&sample.data),
                    None => 0.0,
                },
                None => 0.0,
            };
            let (pan_left, pan_right) = PANNING[index];
            left += value * pan_left;
            right += value * pan_right;
        }
        (left * VOICE_GAIN, right * VOICE_GAIN)
    }
}

/// Apply one pattern cell to one voice.
///
/// A sample number selects the sample and takes its volume without
/// retriggering; a period retriggers whatever sample is selected. Both
/// together is the ordinary case of a note being played.
fn trigger(voice: &mut Voice, note: Note, module: &Module, sample_rate: f64) {
    if note.sample != 0 {
        let slot = usize::from(note.sample) - 1;
        // A sample number past the 31 slots selects nothing. Files do this;
        // ProTracker reads whatever is at that offset, which is not a
        // behaviour worth reproducing.
        if let Some(sample) = module.samples.get(slot) {
            voice.sample = Some(slot);
            voice.volume = sample.volume.min(MAX_VOLUME);
        }
    }

    if note.period == 0 {
        return;
    }
    voice.period = note.period;

    let Some(sample) = voice.sample.and_then(|slot| module.samples.get(slot)) else {
        return;
    };
    let len = sample.data.len();
    if len == 0 {
        voice.active = false;
        return;
    }

    // Clamped, because a header can point its loop past the end of its own
    // data and this crate does not get to panic about that. A loop clamped to
    // nothing becomes a one-shot: the sample plays through and stops.
    let loop_start = sample.loop_start().min(len);
    let loop_len = if sample.is_looped() {
        sample.loop_len().min(len - loop_start)
    } else {
        0
    };

    // The first pass is not always the loop. ProTracker programs Paula with
    // `repeat_start + repeat_length` words when the loop starts partway in,
    // and with the whole sample when it starts at zero — so a sample with a
    // short loop at offset zero still plays all the way through once before it
    // begins repeating (`mt_playvoice`, `set_len_start`).
    voice.end = if loop_len > 0 && sample.repeat_start_words != 0 {
        (loop_start + loop_len).min(len)
    } else {
        len
    };
    voice.loop_start = loop_start;
    voice.loop_len = loop_len;
    voice.position = 0.0;
    voice.step = PAULA_CLOCK_PAL / (2.0 * f64::from(note.period)) / sample_rate;
    voice.active = voice.end > 0;
}
