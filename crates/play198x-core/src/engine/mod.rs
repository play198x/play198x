//! The ProTracker engine: sequencer, mixer, effects, transport and timing.
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
//! [`Engine::advance_tick`] is the single place the sequence moves, and the
//! three effect dispatch tables in [`effects`] hang off its two branches:
//! `fx_tab` on the tick that does not fetch a row, `prefx_tab` and
//! `morefx_tab` on the one that does. Computing a module's duration walks the
//! same function with the mixer switched off — the same code path, not a
//! second implementation that can drift from what playback does.
//!
//! Per-tick effects therefore run on `speed - 1` ticks of each row, not
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
///
/// The split between a *stored* value and a *sounding* one is the replayer's
/// own and matters: vibrato and arpeggio write Paula's period register without
/// writing the channel's stored period back (`mt_vibrato_nc`, line 2153). A
/// player that stores it integrates the sweep and the note sails away instead
/// of wobbling.
#[derive(Debug, Clone, Copy)]
struct Voice {
    /// Which sample slot is selected, if a note has ever chosen one.
    sample: Option<usize>,
    /// That sample's length in bytes, so the voice can be restarted without
    /// reaching back into the module.
    data_len: usize,
    /// Whether it is currently sounding.
    active: bool,

    /// Amiga period, `n_period`: what a slide or a new note leaves behind.
    /// Kept even when silent — a period with no sample number replays the
    /// selected sample, and a sample number with no period sets volume
    /// without retriggering.
    period: u16,
    /// The period Paula is actually playing at, `AUDPER`.
    play_period: u16,
    /// Channel volume, `n_volume`.
    volume: u8,
    /// The volume Paula is actually playing at, `AUDVOL`. Only tremolo makes
    /// it differ from `volume`.
    play_volume: u8,

    /// Playback position in bytes, fractional between output frames.
    position: f64,
    /// Bytes of sample data consumed per output frame.
    step: f64,
    /// Byte offset this pass ends at.
    end: usize,

    /// Byte offset the next trigger starts at, `n_start`. `9xx` moves it.
    start: usize,
    /// Bytes the first pass plays, `n_length`. `9xx` shortens it.
    length: usize,
    loop_start: usize,
    /// Loop length in bytes, `0` when the sample does not loop.
    loop_len: usize,

    /// The row's period, `0` when the row carries no note. `E9x` and `EDx`
    /// both branch on whether this row had one.
    note_period: u16,
    /// The row's effect number and parameter, `n_cmd`. They persist for the
    /// whole row, including through a pattern delay's extra rounds.
    effect: u8,
    param: u8,
    /// Whether the row's cell was entirely zero, which is how the replayer
    /// decides to re-assert the period at the next note tick.
    cell_empty: bool,

    /// Index into [`effects::PERIOD_TABLE`], `0..16`.
    finetune: usize,
    /// The note the current period came from, `0..36`. Arpeggio and glissando
    /// step from it.
    note_index: usize,
    /// Tone portamento's target, `0` when there is nothing to slide to.
    wanted_period: u16,
    tone_speed: u8,
    glissando: bool,
    vib_pos: u8,
    vib_speed: u8,
    vib_amp: u8,
    vib_ctrl: u8,
    trem_pos: u8,
    trem_speed: u8,
    trem_amp: u8,
    trem_ctrl: u8,
    /// The offset `900` reuses.
    sample_offset: u8,
    /// Ticks left before `E9x` retriggers.
    retrig_count: u8,
    /// The row `E60` marked as a loop start.
    loop_row: usize,
    /// Repeats left for `E6x`. Negative means "not started".
    loop_count: i8,
}

impl Voice {
    const fn silent() -> Self {
        Self {
            sample: None,
            data_len: 0,
            active: false,
            period: 0,
            play_period: 0,
            volume: 0,
            play_volume: 0,
            position: 0.0,
            step: 0.0,
            end: 0,
            start: 0,
            length: 0,
            loop_start: 0,
            loop_len: 0,
            note_period: 0,
            effect: 0,
            param: 0,
            cell_empty: true,
            finetune: 0,
            note_index: 0,
            wanted_period: 0,
            tone_speed: 0,
            glissando: false,
            vib_pos: 0,
            vib_speed: 0,
            vib_amp: 0,
            vib_ctrl: 0,
            trem_pos: 0,
            trem_speed: 0,
            trem_amp: 0,
            trem_ctrl: 0,
            sample_offset: 0,
            retrig_count: 0,
            loop_row: 0,
            loop_count: 0,
        }
    }

    /// Write Paula's period register: the one place playback pitch changes.
    ///
    /// A period of zero is a module that never gave this voice a note, and
    /// dividing by it would produce an infinite rate rather than a refused
    /// one, so it is floored at one.
    fn set_audper(&mut self, period: u16, sample_rate: f64) {
        self.play_period = period;
        self.step = PAULA_CLOCK_PAL / (2.0 * f64::from(period.max(1))) / sample_rate;
    }

    /// Start the sample from `start` — a new note, `E9x` retrigger, or `EDx`
    /// note delay reaching its tick (`do_retrigger`, line 2528).
    fn retrigger(&mut self) {
        let start = self.start.min(self.data_len);
        self.position = start as f64;
        self.end = (start + self.length).min(self.data_len);
        self.active = self.end > start;
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
        let out = f32::from(value) / 128.0 * f32::from(self.play_volume) / f32::from(MAX_VOLUME);

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
/// module with a zero-length sample, a loop pointing past the end of its data,
/// an order naming a pattern the file does not contain, a portamento target of
/// zero or a sample offset past the end of its sample all render as something,
/// because this crate sits behind an FFI boundary where unwinding is undefined
/// behaviour.
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

    /// The row the next step jumps to, `mt_PBreakPos`. `Dxy` and `E6x` set it.
    pbreak_row: usize,
    /// `mt_PBreakFlag`: an `E6x` pattern loop, which stays inside the pattern.
    pbreak_flag: bool,
    /// `mt_PosJumpFlag`: a `Bxy` or `Dxy`, which move to the next order.
    posjump_flag: bool,
    /// `mt_PattDelTime`: the delay `EEx` asked for, before it takes effect.
    patt_del_time: u8,
    /// `mt_PattDelTime2`: extra rounds of the current row still to play.
    patt_del_time2: u8,
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
            pbreak_row: 0,
            pbreak_flag: false,
            posjump_flag: false,
            patt_del_time: 0,
            patt_del_time2: 0,
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
        self.pbreak_row = 0;
        self.pbreak_flag = false;
        self.posjump_flag = false;
        self.patt_del_time = 0;
        self.patt_del_time2 = 0;
    }

    /// A channel's stored volume, `0..=64`.
    ///
    /// Test-facing, and hidden from the documented surface: an effect is only
    /// observable through `render`'s output, but asserting a volume slide's
    /// step count against an envelope measures the mixer as much as the
    /// effect. This exposes the one number without making the mixer public.
    #[doc(hidden)]
    #[must_use]
    pub fn debug_channel_volume(&self, channel: usize) -> u8 {
        self.voices.get(channel).map_or(0, |voice| voice.volume)
    }

    /// The period a channel is *sounding* at — Paula's `AUDPER`, not the
    /// stored period, so a vibrato or an arpeggio is visible here.
    #[doc(hidden)]
    #[must_use]
    pub fn debug_channel_period(&self, channel: usize) -> u16 {
        self.voices
            .get(channel)
            .map_or(0, |voice| voice.play_period)
    }

    /// Output frames in one tick: `sample_rate * (2500 / tempo) / 1000`.
    ///
    /// Recomputed each tick rather than cached, so that `Fxy` setting the
    /// tempo cannot leave a stale figure behind. One division per 20 ms is not
    /// a cost worth a cache-invalidation bug.
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
    /// The one place playback advances, and the place the three effect tables
    /// hang off. Duration measurement walks this with the mixer switched off
    /// rather than reimplementing it.
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
        if self.tick < self.speed.max(1) {
            // An ordinary tick: `fx_tab` only. This branch is why a per-tick
            // effect runs `speed - 1` times a row.
            self.run_fx();
            return;
        }

        self.tick = 0;
        // The replayer steps the pattern at the *end* of a note tick
        // (`pattern_step`, line 1387). Doing it here instead, at the start of
        // the next one, consumes the same flags in the same order and keeps
        // `position` naming the row that is sounding rather than the one
        // queued behind it.
        self.pattern_step();
        if self.patt_del_time2 == 0 {
            self.start_row();
        } else {
            // A pattern delay's extra round does not re-fetch the row, so no
            // note retriggers and only `fx_tab` runs — on the note tick too.
            self.run_fx();
        }
    }

    /// `pattern_step` (line 1387): decide which row plays next.
    ///
    /// Pattern delay can hold the row, `E6x` can jump back inside the pattern,
    /// and `Bxy`/`Dxy` move to the next order. All three are flags the row
    /// just played left behind.
    fn pattern_step(&mut self) {
        let mut advance = true;
        let mut delay = self.patt_del_time2;
        if self.patt_del_time != 0 {
            delay = self.patt_del_time;
            self.patt_del_time = 0;
        }
        if delay != 0 {
            delay -= 1;
            if delay != 0 {
                advance = false;
            }
            self.patt_del_time2 = delay;
        }

        self.row += usize::from(advance);
        if self.pbreak_flag {
            self.pbreak_flag = false;
            self.row = self.pbreak_row;
            self.pbreak_row = 0;
        }
        if self.row >= ROWS_PER_PATTERN || self.posjump_flag {
            self.song_step();
        }
    }

    /// `song_step` (line 1424): move to the next order.
    ///
    /// ProTracker 2.3 restarts a finished song at order 0 and ignores the
    /// module's restart byte, which is a Noisetracker leftover that real files
    /// routinely leave at 127.
    fn song_step(&mut self) {
        self.row = self.pbreak_row;
        self.pbreak_row = 0;
        self.posjump_flag = false;
        // `and.w #$007f`: the order is a 7-bit field, so it wraps rather than
        // running off the 128-entry table.
        self.order = (self.order + 1) & 0x7F;
        if self.order >= self.module.orders().len() {
            self.order = 0;
        }
    }

    /// Act on the row the sequencer has arrived at: `prefx_tab`, the period,
    /// then `morefx_tab`, per channel.
    fn start_row(&mut self) {
        let pattern = self.current_pattern();
        // An order naming a pattern the file does not hold plays as an empty
        // row and takes its normal time. Real files carry garbage in the
        // unplayed tail of the order table; refusing to play, or reading past
        // the end, are both worse answers than a quiet row.
        //
        // Copied out so the voices can be borrowed mutably below. `Note` is
        // 6 bytes and `Copy`; this is a register shuffle, not an allocation.
        let notes: [Note; CHANNELS] = self
            .module
            .patterns
            .get(pattern)
            .and_then(|pattern| pattern.get(self.row))
            .copied()
            .unwrap_or_default();

        for (channel, note) in notes.into_iter().enumerate() {
            self.play_voice(channel, note);
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
