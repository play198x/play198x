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
//! [`Seq::advance_tick`] is the single place the sequence moves, and the
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
use std::collections::BTreeMap;
use std::time::Duration;

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

/// Ticks [`Engine::timing`] will walk before giving up.
///
/// The visited-position set is what actually stops a looping module; this is
/// the guarantee that a hostile one stops even if that set has a hole in it,
/// so it is derived to sit well clear of anything legitimate rather than
/// picked.
///
/// The longest *legitimate* module is a 128-order song at ProTracker's slowest
/// settings. Every one of its `128 x 64 = 8192` rows can be held for `speed`
/// ticks (at most 31, since `$20` is a tempo) and replayed by an `EEx` pattern
/// delay up to 16 times:
///
/// ```text
/// 8192 rows x 31 ticks x 16 delay rounds = 4_063_232 ticks
/// ```
///
/// which at the slowest tempo of 32 — a tick of 78.125 ms — is 88 hours of
/// music that no one has ever written. `E6x` pattern loops multiply that
/// again, but a loop's repeats are bounded by its nibble and the visited set
/// counts them as distinct positions, so they end by themselves.
///
/// Ten million ticks is a little over twice the arithmetic worst case, roughly
/// forty times the longest module anybody has actually made, and costs under a
/// second to walk. A real module reaches its end or its loop in a few thousand.
const WALK_TICK_CAP: u32 = 10_000_000;

/// Positions [`Engine::timing`] will remember.
///
/// `128 orders x 64 rows x 16 pattern-loop counts` — every position a module
/// can legitimately reach and be expected to reach again. Past this the walk
/// stops recording and leans on [`WALK_TICK_CAP`] instead, which bounds what a
/// pathological file can make it allocate: the map holds 16 bytes an entry, so
/// two megabytes at the very worst and a few kilobytes for real music.
const VISIT_CAP: usize = 128 * ROWS_PER_PATTERN * 16;

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

/// How long a module plays, and whether it comes back on itself.
///
/// Produced by [`Engine::timing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// One pass: from the top of the order table to whichever comes first —
    /// the end of the song, an `F00` that stops it, or a position the song has
    /// already played.
    pub duration: Duration,
    /// Whether the song returns to a position it has already played instead of
    /// running off the end of its order table.
    pub loops: bool,
    /// How far into [`duration`](Self::duration) the repeated position was
    /// first reached — the point playback comes back to. `None` when the song
    /// does not loop.
    pub loop_start: Option<Duration>,
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
    /// Transport. The caller's business, not the song's, which is why it sits
    /// here rather than in [`State`] — a duration walk has no opinion about
    /// whether somebody has pressed pause.
    playing: bool,
    state: State,
}

/// Everything about the engine that moves as the song plays.
///
/// Split out from [`Engine`] so [`Engine::timing`] can run the sequencer over
/// a fresh copy of it: `timing` takes `&self` and the walk needs somewhere
/// mutable to move, and cloning the whole engine would copy every sample's PCM
/// to walk a song that never reads a byte of it. `Copy` because it is voices
/// and counters and nothing that owns memory.
#[derive(Debug, Clone, Copy)]
struct State {
    voices: [Voice; CHANNELS],

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

    /// `mt_SongEnd` (line 1438): the order index has run past the played
    /// prefix and the song has restarted from the top. Playback carries on —
    /// ProTracker only stops after 128 such passes — but it is where the
    /// duration walk stops counting.
    song_end: bool,
    /// `mt_Enable` cleared: `F00` calls `_mt_end` (line 2347) and stops the
    /// module dead. The replayer stores the zero speed *and then* stops, so
    /// this is a separate flag rather than a speed of zero.
    stopped: bool,
}

/// The sequencer: a module and the state it moves through, borrowed together.
///
/// Playback and duration both run *this*. A second walk written beside the
/// first is a walk that drifts from what you hear, and every position effect —
/// `Bxx`, `Dxx`, `E6x`, `EEx`, `Fxy` — changes a module's length, so the two
/// would drift on exactly the modules where the answer matters.
struct Seq<'a> {
    module: &'a Module,
    sample_rate: f64,
    state: &'a mut State,
}

impl State {
    /// A module at its power-on defaults, on the first row of the first order.
    const fn new() -> Self {
        Self {
            voices: [Voice::silent(); CHANNELS],
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
            song_end: false,
            stopped: false,
        }
    }
}

impl Engine {
    /// Load `module` and start it at the top of the order table, playing.
    ///
    /// A `sample_rate` of zero is treated as 1 Hz. It is nonsense either way,
    /// but this crate does not get to panic about it.
    #[must_use]
    pub fn new(module: Module, sample_rate: u32) -> Self {
        Self {
            module,
            sample_rate: f64::from(sample_rate.max(1)),
            playing: true,
            state: State::new(),
        }
    }

    /// Fill `out` with interleaved stereo frames, returning how many it wrote.
    ///
    /// A caller with an odd-length buffer gets whole frames and the odd slot
    /// back untouched. **Allocates nothing** — `tests/engine_allocations.rs`
    /// counts, because an engine that allocates on the audio thread glitches
    /// on somebody else's machine and never on yours.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        let playing = self.playing;
        // Borrowed once, outside the loop: `module` shared and `state` mutable
        // are disjoint fields, which the borrow checker only sees through a
        // struct literal built here rather than through a method call.
        let mut seq = Seq {
            module: &self.module,
            sample_rate: self.sample_rate,
            state: &mut self.state,
        };
        let mut frames = 0;
        // `as_chunks_mut` rather than `chunks_exact_mut(2)`: a stereo frame
        // is a fixed pair, and the typed form drops a bounds check per sample.
        for frame in out.as_chunks_mut::<2>().0 {
            let (left, right) = if playing && !seq.state.stopped {
                if seq.state.frames_to_next_tick <= 0.0 {
                    seq.advance_tick();
                    seq.state.frames_to_next_tick += seq.frames_per_tick();
                }
                let mixed = seq.mix_frame();
                seq.state.frames_to_next_tick -= 1.0;
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
            order: self.state.order,
            pattern: pattern_at(&self.module, self.state.order),
            row: self.state.row,
            tick: self.state.tick,
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
        // Speed and tempo survive a seek: they are where the song left them,
        // and the scrub bar is not a power cycle.
        let speed = self.state.speed;
        let tempo = self.state.tempo;
        self.state = State::new();
        self.state.speed = speed;
        self.state.tempo = tempo;
        self.state.order = order_index.min(orders - 1);
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
        self.state
            .voices
            .get(channel)
            .map_or(0, |voice| voice.volume)
    }

    /// The period a channel is *sounding* at — Paula's `AUDPER`, not the
    /// stored period, so a vibrato or an arpeggio is visible here.
    #[doc(hidden)]
    #[must_use]
    pub fn debug_channel_period(&self, channel: usize) -> u16 {
        self.state
            .voices
            .get(channel)
            .map_or(0, |voice| voice.play_period)
    }

    /// How long the module plays, and whether it comes back on itself.
    ///
    /// Walks the song silently from the top — the same `advance_tick` playback
    /// runs, with the mixer switched off — because `Bxx`, `Dxx`, `E6x`, `EEx`
    /// and `Fxy` all change how long a module lasts. Rows times speed times
    /// tick length is the wrong answer for any module that uses one of them,
    /// which is most of them.
    ///
    /// Takes `&self` and disturbs nothing: the walk moves its own fresh
    /// sequencer state and only reads the module, so calling it mid-playback
    /// cannot alter what you are hearing. Unlike [`Engine::render`] it *does*
    /// allocate — a map of the positions it has visited, bounded to a couple
    /// of megabytes even for a hostile file — so it belongs anywhere except
    /// the audio callback.
    #[must_use]
    pub fn timing(&self) -> Timing {
        let mut state = State::new();
        let mut seq = Seq {
            module: &self.module,
            sample_rate: self.sample_rate,
            state: &mut state,
        };

        // A `BTreeMap` rather than a `HashMap`: no random seed, no thread-local
        // hasher state, and the same answer on every machine and every run,
        // which matters for a number that ends up in a file's metadata.
        let mut visited: BTreeMap<u64, f64> = BTreeMap::new();
        let mut elapsed_ms = 0.0_f64;
        let mut loops = false;
        let mut loop_start_ms = None;

        for _ in 0..WALK_TICK_CAP {
            if seq.module.orders().is_empty() {
                break;
            }
            let fetched = seq.advance_tick();

            // The song ran off the end of its order table and restarted from
            // the top. That is where a module's length stops being measured:
            // the tick just taken belongs to the second pass, so it is not
            // counted.
            if seq.state.song_end {
                break;
            }
            // `F00` stopped the module dead (`_mt_end`). Everything from here
            // is silence, and the tick that stopped it produced silence too.
            if seq.state.stopped {
                break;
            }

            if let Some(key) = fetched {
                if let Some(first) = visited.get(&key) {
                    loops = true;
                    loop_start_ms = Some(*first);
                    break;
                }
                if visited.len() < VISIT_CAP {
                    visited.insert(key, elapsed_ms);
                }
            }

            elapsed_ms += seq.tick_ms();
        }

        Timing {
            duration: millis(elapsed_ms),
            loops,
            loop_start: loop_start_ms.map(millis),
        }
    }
}

impl Seq<'_> {
    /// One tick's length in milliseconds: `2500 / tempo`.
    ///
    /// PAL's CIA timer is loaded with `1773447 / tempo` and counts at 709379
    /// Hz, so the interrupt rate is `tempo * 0.4` and the tick is `2.5 / tempo`
    /// seconds — 20 ms at the default 125. (`mt_setspeed` line 2353 for the
    /// division, line 344 for the PAL constant.)
    fn tick_ms(&self) -> f64 {
        // `max(1)` because `Fxy` can name a tempo of zero, and dividing by it
        // would produce an infinite tick rather than a refused one.
        TICK_MS_NUMERATOR / f64::from(self.state.tempo.max(1))
    }

    /// Output frames in one tick.
    ///
    /// Recomputed each tick rather than cached, so that `Fxy` setting the
    /// tempo cannot leave a stale figure behind. One division per 20 ms is not
    /// a cost worth a cache-invalidation bug.
    fn frames_per_tick(&self) -> f64 {
        self.sample_rate * self.tick_ms() / 1_000.0
    }

    /// The pattern the current order names, or 0 when there is no order to
    /// read — a `song_length` of zero, which is a module with nothing to play.
    fn current_pattern(&self) -> usize {
        pattern_at(self.module, self.state.order)
    }

    /// The position the duration walk records as visited.
    ///
    /// Order and row, **and the four channels' `E6x` repeat counters**. A
    /// pattern loop is *supposed* to bring the song back to a row it has
    /// already played, a bounded number of times, so a bare `(order, row)`
    /// reports every legitimate `E63` as a song loop and truncates the
    /// duration to the first repeat. The counter is what makes those arrivals
    /// distinct: they run 3, 2, 1 on the way through the loop and are back at
    /// 0 by the time the song could genuinely return. `mt_jumploop` only ever
    /// stores a nibble, so each counter is `0..=15` and 45 bits hold the lot.
    ///
    /// Read **before** the row's own effects run, which is why
    /// [`Self::advance_tick`] returns it rather than the walk asking for it
    /// afterwards: `mt_posjump` writes the new order the moment `Bxx` acts, so
    /// a key taken after the fact says where the song is *going* rather than
    /// where it just was. A `B01` on order 2 read as order 0, collided with
    /// order 0's own row, and cut a three-order song a row short.
    fn visit_key(&self) -> u64 {
        let mut key = ((self.state.order as u64) & 0x7F) << 6 | (self.state.row as u64 & 0x3F);
        for voice in &self.state.voices {
            key = (key << 8) | u64::from(voice.loop_count as u8);
        }
        key
    }

    /// Move the sequence on by one tick, reporting the position it fetched.
    ///
    /// The one place playback advances, and the place the three effect tables
    /// hang off. [`Engine::timing`] walks this with the mixer switched off
    /// rather than reimplementing it, and the returned [`Self::visit_key`] is
    /// what tells it a new position has been reached: `None` for a tick that
    /// does not fetch a row, because a pattern delay's extra rounds replay a
    /// row without revisiting it.
    fn advance_tick(&mut self) -> Option<u64> {
        if self.module.orders().is_empty() {
            return None;
        }

        if self.state.row_pending {
            self.state.row_pending = false;
            self.state.tick = 0;
            return Some(self.fetch_row());
        }

        self.state.tick += 1;
        // `max(1)` because `F00` sets the speed to zero on its way to stopping
        // the song, and a row of zero ticks would otherwise stall the
        // comparison rather than the song.
        if self.state.tick < self.state.speed.max(1) {
            // An ordinary tick: `fx_tab` only. This branch is why a per-tick
            // effect runs `speed - 1` times a row.
            self.run_fx();
            return None;
        }

        self.state.tick = 0;
        // The replayer steps the pattern at the *end* of a note tick
        // (`pattern_step`, line 1387). Doing it here instead, at the start of
        // the next one, consumes the same flags in the same order and keeps
        // `position` naming the row that is sounding rather than the one
        // queued behind it.
        self.pattern_step();
        if self.state.patt_del_time2 == 0 {
            Some(self.fetch_row())
        } else {
            // A pattern delay's extra round does not re-fetch the row, so no
            // note retriggers and only `fx_tab` runs — on the note tick too.
            self.run_fx();
            None
        }
    }

    /// Note the position, then play the row that is at it.
    ///
    /// The order matters and is the whole reason this is a function: `Bxx`
    /// moves the order index while the row it sits on is being played, so the
    /// position has to be taken first or it names the destination.
    fn fetch_row(&mut self) -> u64 {
        let key = self.visit_key();
        self.start_row();
        key
    }

    /// `pattern_step` (line 1387): decide which row plays next.
    ///
    /// Pattern delay can hold the row, `E6x` can jump back inside the pattern,
    /// and `Bxy`/`Dxy` move to the next order. All three are flags the row
    /// just played left behind.
    fn pattern_step(&mut self) {
        let mut advance = true;
        let mut delay = self.state.patt_del_time2;
        if self.state.patt_del_time != 0 {
            delay = self.state.patt_del_time;
            self.state.patt_del_time = 0;
        }
        if delay != 0 {
            delay -= 1;
            if delay != 0 {
                advance = false;
            }
            self.state.patt_del_time2 = delay;
        }

        self.state.row += usize::from(advance);
        if self.state.pbreak_flag {
            self.state.pbreak_flag = false;
            self.state.row = self.state.pbreak_row;
            self.state.pbreak_row = 0;
        }
        if self.state.row >= ROWS_PER_PATTERN || self.state.posjump_flag {
            self.song_step();
        }
    }

    /// `song_step` (line 1424): move to the next order.
    ///
    /// ProTracker 2.3 restarts a finished song at order 0 and ignores the
    /// module's restart byte, which is a Noisetracker leftover that real files
    /// routinely leave at 127.
    fn song_step(&mut self) {
        self.state.row = self.state.pbreak_row;
        self.state.pbreak_row = 0;
        self.state.posjump_flag = false;
        // `and.w #$007f`: the order is a 7-bit field, so it wraps rather than
        // running off the 128-entry table.
        self.state.order = (self.state.order + 1) & 0x7F;
        if self.state.order >= self.module.orders().len() {
            self.state.order = 0;
            // `addq.b #1,mt_SongEnd` (line 1438). The replayer counts passes
            // and keeps playing; this crate keeps playing too, and only the
            // duration walk reads the flag. A `B00` that jumps to order 0
            // does *not* set it — the increment lands inside the played
            // prefix — which is what separates a song that ends from a song
            // that loops.
            self.state.song_end = true;
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
            .and_then(|pattern| pattern.get(self.state.row))
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
        for (index, voice) in self.state.voices.iter_mut().enumerate() {
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

/// The pattern an order names, or 0 when there is no such order — a
/// `song_length` of zero, which is a module with nothing to play.
fn pattern_at(module: &Module, order: usize) -> usize {
    module
        .orders()
        .get(order)
        .map_or(0, |pattern| usize::from(*pattern))
}

/// Milliseconds as a [`Duration`], without a panic path.
///
/// `Duration::from_secs_f64` panics on a negative or non-finite value.
/// Nothing here can produce one — a tick is `2500 / tempo` with the tempo
/// floored at 1, and the walk takes at most [`WALK_TICK_CAP`] of them — but
/// this crate sits behind an FFI boundary, so the arithmetic that "cannot"
/// overflow is still not allowed to unwind if it does.
fn millis(ms: f64) -> Duration {
    Duration::try_from_secs_f64(ms / 1_000.0).unwrap_or(Duration::ZERO)
}
