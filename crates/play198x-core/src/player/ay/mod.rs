pub mod format;

use crate::host::spectrum::SpectrumHost;
use emu198x_gi_ay_3_8910::Ay3_8910;
use format::{AyError, AyFile, Song};

// `T_STATES_PER_FRAME` and `AY_CLOCK_HZ` must come from the same machine:
// the AY is 128K-only hardware (an `.ay` file targets the 128K's sound, not
// the 48K's, which has no AY at all), so both constants below are the 128K
// Spectrum's. An earlier version of this file paired the 48K's frame length
// (69,888 T-states, from its 3.5MHz clock) with the 128K's AY clock — two
// different machines' numbers in one model — which under-fed the chip's
// downsampler by 524 ticks a frame (a silent gap at the end of every frame:
// a 50Hz buzz, and playback fractionally slow). Do not "fix" `AY_CLOCK_HZ`
// downward to make the arithmetic line up instead: it sets the pitch the
// chip produces, so lowering it would detune every tune to cancel an
// unrelated error in the frame-length constant.
/// T-states in one 50Hz frame on a 128K Spectrum: 228 T-states/line x 311
/// lines = 70,908, from a 3,546,900Hz CPU clock (≈50.02Hz refresh).
const T_STATES_PER_FRAME: u32 = 70_908;
/// Where the stub parks a return address so a call's end is detectable.
const SENTINEL: u16 = 0xFFFF;
/// A call that has not returned by here is not going to.
///
/// `call()`'s loop counts iterations of `step_with_chip`, and
/// `step_with_chip` is one half T-state (see `t_states`'s field doc) —
/// the same half-T-state unit `frame()` compares against
/// `T_STATES_PER_FRAME * 2`, not `T_STATES_PER_FRAME` alone. Written as
/// `T_STATES_PER_FRAME * 2 * 4` rather than the equal-valued
/// `T_STATES_PER_FRAME * 8` so the `* 2` for the unit and the `* 4` for
/// the budget's actual size — four real 50Hz frames' worth of T-states —
/// don't collapse into one number that reads like eight frames. An
/// earlier version of this constant made exactly that misreading, in its
/// own comment: this is the same half-T-state trap Task 5 hit in
/// `frame()`'s own loop condition.
const CALL_BUDGET: u32 = T_STATES_PER_FRAME * 2 * 4;
/// The 128K AY's clock: its 3,546,900Hz CPU clock halved.
const AY_CLOCK_HZ: u32 = 1_773_400;
/// AY ticks per `step_with_chip` call, expressed as a divisor rather than a
/// float. `SpectrumHost::step()` (and so `step_with_chip`) advances the
/// `emu198x-zilog-z80` core by one half T-state per call — the crate models
/// each T-state as a Rise and a Fall half-cycle and dispatches one per
/// `tick()` (verified: four NOPs, 16 T-states, take 32 `step()` calls). The
/// AY should tick once per two real T-states, i.e. once per four
/// `step_with_chip` calls; using 2 here would clock the chip twice too fast.
const AY_TICK_DIVISOR: u32 = 4;

/// How loud the beeper is against the chip. The Spectrum's speaker is one
/// bit driven directly, and it is loud; at parity with a full-volume AY
/// channel a mixed tune clips. Halved, which is a judgement rather than a
/// measurement — revisit if a real tune sounds wrong.
///
/// The DC blocker (`sample_beeper`) makes this gain's effect
/// frequency-dependent, not a flat halving. For a square wave of amplitude
/// `A` and half-period `N` output samples, `y[n] = x[n] - x[n-1] +
/// R*y[n-1]`'s steady-state peak works out to `2A/(1+R^N)` (derived from
/// the filter's recurrence at the two edges, verified against a direct
/// simulation) — it climbs toward `2A` as `N` grows and decays toward `A`
/// as `N` shrinks, because a slow edge's decay-to-zero finishes well before
/// the next edge arrives, while a fast edge's does not. At 48kHz with this
/// file's `R ≈ 0.9954`, pre-gain: a 2.18kHz tone (`N=11`) peaks at `1.025`;
/// 1kHz (`N=24`) at `1.055`; 200Hz (`N=120`) at `1.269`. Post-`BEEPER_GAIN`,
/// that is a `1.03×`-`1.27×` boost over the unfiltered `0.5` across the
/// roughly 200Hz-4kHz range real beeper music actually occupies — a mild
/// overshoot, not a headroom cliff.
///
/// The ~0.99 peak measured against `a_beeper_only_tune_is_audible`'s
/// fixture (see `task-6-report.md`'s fix-round-2 entry) is not
/// representative of that: the fixture's toggle burst is brief and then
/// holds for most of a 20ms frame, which behaves close to a ~25Hz square
/// wave (`N≈960`) — the one point on this curve where `R^N` is small enough
/// for the peak to approach `2A`. No real beeper engine plays a ~25Hz tone;
/// it is an artifact of that fixture's shape, not of typical playback.
///
/// The AY chip's own three channels already sum to `~1.0` at full volume
/// (`~0.333` each) before the beeper is added at all, so there is no
/// headroom budget in this mix regardless of the beeper's own gain — a
/// clipping clamp is a real, open question. It is deliberately deferred to
/// the whole-branch review, once the Task 8 corpus shows how many real
/// tunes actually drive both the chip and the beeper together, and at what
/// beeper frequencies, so the worst case is measured rather than guessed.
const BEEPER_GAIN: f32 = 0.5;

/// Target -3dB cutoff for the beeper's DC-blocking high-pass (see
/// `AyPlayer::dc_r` and `sample_beeper`). Real hardware AC-couples the
/// speaker, so a held level decays to silence instead of sitting as a
/// constant offset; this is the digital equivalent. Chosen deliberately,
/// not copied from a canned `R`: a 1-bit beeper engine has no practical use
/// below roughly 50Hz (the resolution and the audibility both fall apart
/// down there), so 35Hz sits well clear of anything a tune would use as a
/// tone, while being high enough that the filter settles within about one
/// output frame (20ms) rather than leaving a real click's decay tail
/// hanging into the next frame or two.
const DC_BLOCKER_CUTOFF_HZ: f32 = 35.0;

pub struct AyPlayer {
    pub host: SpectrumHost,
    song: Song,
    frames_played: u32,
    chip: Ay3_8910,
    samples_per_frame: usize,
    ay_tick_accumulator: u32,
    /// Half T-states elapsed — `step_with_chip` increments this once per
    /// call, and each call is one Rise-or-Fall half-cycle of the Z80 core
    /// (see [`AY_TICK_DIVISOR`]). Two of these make one real T-state, so
    /// callers compare against `T_STATES_PER_FRAME * 2`.
    t_states: u32,
    /// One sample of the speaker bit per output sample, filled by
    /// `sample_beeper` as the CPU runs and drained by `render`.
    beeper: Vec<f32>,
    /// Bresenham-style accumulator for downsampling the beeper from host
    /// cycles (half T-states) to `samples_per_frame` output samples. Plain
    /// division (`T_STATES_PER_FRAME / samples_per_frame`) truncates and
    /// drifts — at 48kHz it produces ~964 samples/frame against a
    /// 960-sample buffer, dropping the tail and running the beeper
    /// fractionally fast. Adding `samples_per_frame` every host cycle and
    /// firing whenever the total reaches `T_STATES_PER_FRAME * 2` emits
    /// exactly `samples_per_frame` samples every frame with no drift — the
    /// same technique `emu198x-gi-ay-3-8910` uses internally for its own
    /// downsampling, so the two signals stay in step.
    beeper_accumulator: u32,
    /// The DC-blocking high-pass's pole, `R` in
    /// `y[n] = x[n] - x[n-1] + R*y[n-1]`, derived from the caller's
    /// `sample_rate` in `new()` so the filter's cutoff (see
    /// [`DC_BLOCKER_CUTOFF_HZ`]) — and so its decay time — stays the same
    /// regardless of what rate this player runs at. A fixed literal `R`
    /// would tie the cutoff to whichever sample rate happened to be used
    /// when it was chosen; this player takes an arbitrary caller-supplied
    /// rate (44.1kHz and 48kHz are both realistic), and the two would
    /// otherwise sound subtly different — deriving `R` keeps them
    /// identical.
    dc_r: f32,
    /// `x[n-1]`: the DC blocker's previous unfiltered beeper sample.
    dc_prev_x: f32,
    /// `y[n-1]`: the DC blocker's previous filtered output. Carried on the
    /// player (not reset per frame) so the filter is continuous across
    /// frame boundaries — resetting it every frame would reintroduce a
    /// step at every frame edge, audible as a click at 50Hz.
    dc_prev_y: f32,
}

impl AyPlayer {
    /// Loads `song` from an `.ay` file and runs its init routine.
    ///
    /// # Errors
    ///
    /// When the bytes are not an `.ay` file, or `song` is out of range.
    pub fn new(bytes: &[u8], song: usize, sample_rate: u32) -> Result<Self, AyError> {
        let file: AyFile = format::parse(bytes)?;
        let song = file.songs.get(song).cloned().ok_or(AyError::NoSuchSong)?;

        let mut host = SpectrumHost::new();
        // The stub, standing in for the ROM the format does not want: any
        // call into low memory returns instead of executing whatever happens
        // to be there.
        for address in 0x0000..0x0100u16 {
            host.mem.write(address, 0xC9);
        }
        // Once `call()` detects PC == SENTINEL it stops driving the CPU
        // itself, but `frame()` keeps clocking the whole host afterwards so
        // the chip gets a full frame's worth of ticks — so SENTINEL must be
        // somewhere safe to keep fetching from. Left at its default (0x00,
        // NOP), the fetch after it wraps 0xFFFF -> 0x0000 and lands on the
        // low-memory RET stub above, which pops whatever the (long-since
        // restored) stack now holds and sends the CPU off executing tune
        // data as code. A HALT here parks the CPU on itself instead (the
        // Z80 core re-fetches NOPs internally without advancing PC while
        // halted), so `step_with_chip` stays safe to call all the way to
        // the next frame boundary.
        host.mem.write(SENTINEL, 0x76);
        for block in &song.blocks {
            host.mem.load(block.address, &block.data);
        }

        host.cpu.regs.sp = if song.stack == 0 { 0xC000 } else { song.stack };

        // The format hands the player one 16-bit value for every "common"
        // register pair -- AF/AF', BC/BC', DE/DE', HL/HL', IX, IY -- split
        // into two halves: HiReg is the high byte (A, B, D, H, IXH, IYH),
        // LoReg is the low byte (F, C, E, L, IXL, IYL). F and F' take LoReg
        // too; the format does not special-case the flags.
        //
        // Which byte of the file is which is the parser's business, and it
        // matters more than the split does: a multi-song file selects its
        // subtune by the number the format leaves in A, so reading the two
        // halves the wrong way round makes every subtune play song 0. See
        // `format::Song`'s field docs for the offsets, and
        // `tests/ay_format.rs` for what pins them.
        let reg_pair = (u16::from(song.hi_reg) << 8) | u16::from(song.lo_reg);
        host.cpu.regs.af = reg_pair;
        host.cpu.regs.af_alt = reg_pair;
        host.cpu.regs.bc = reg_pair;
        host.cpu.regs.bc_alt = reg_pair;
        host.cpu.regs.de = reg_pair;
        host.cpu.regs.de_alt = reg_pair;
        host.cpu.regs.hl = reg_pair;
        host.cpu.regs.hl_alt = reg_pair;
        host.cpu.regs.ix = reg_pair;
        host.cpu.regs.iy = reg_pair;

        let samples_per_frame = (sample_rate / 50) as usize;
        // R = 1 - 2*pi*fc/fs for a one-pole high-pass's -3dB point at fc,
        // fs the sample rate — see `dc_r`'s field doc for why this is
        // derived per-instance rather than a fixed literal.
        let dc_r = 1.0 - (2.0 * std::f32::consts::PI * DC_BLOCKER_CUTOFF_HZ) / sample_rate as f32;
        let mut player = Self {
            host,
            song,
            frames_played: 0,
            chip: Ay3_8910::new(AY_CLOCK_HZ, sample_rate, samples_per_frame),
            samples_per_frame,
            ay_tick_accumulator: 0,
            t_states: 0,
            beeper: Vec::with_capacity(samples_per_frame),
            beeper_accumulator: 0,
            dc_r,
            dc_prev_x: 0.0,
            dc_prev_y: 0.0,
        };
        let init = if player.song.init == 0 {
            player.song.blocks.first().map_or(0, |b| b.address)
        } else {
            player.song.init
        };
        if !player.call(init) {
            return Err(AyError::InitDidNotReturn);
        }
        Ok(player)
    }

    /// One 50Hz frame: the tune's interrupt routine, then the rest of the
    /// frame's cycles so anything it started can finish, with the AY chip
    /// clocked throughout.
    pub fn frame(&mut self) {
        let before = self.t_states;
        let _ = self.call(self.song.interrupt);
        // `t_states` counts half T-states (see the field doc), so a real
        // frame's worth of T_STATES_PER_FRAME T-states is twice that many
        // `step_with_chip` calls.
        while self.t_states - before < T_STATES_PER_FRAME * 2 {
            self.step_with_chip();
        }
        self.frames_played += 1;
    }

    /// Fills one frame of interleaved stereo. Call once per `frame()`.
    pub fn render(&mut self, out: &mut [f32]) {
        let mut mono = vec![0.0f32; self.samples_per_frame];
        self.chip.end_frame(&mut mono);

        // A tune that never wrote the speaker port has no beeper signal.
        // The DC blocker in `sample_beeper` now removes a *held* level's
        // offset on its own, so this guard is no longer preventing a sticky
        // DC offset — but it still earns its place: `sample_beeper` runs
        // unconditionally from the start of playback (the filter has to see
        // every sample to stay continuous, see its comment), so a tune that
        // never writes the port is still feeding the filter a constant
        // -1.0 it was never told to expect. Against the filter's zeroed
        // initial state that constant looks exactly like an edge, and the
        // filter answers with its own start-up transient before decaying
        // away. Without this guard that transient — not a real event —
        // would leak into the very first frame of every tune with no
        // beeper output at all. Gating on `speaker_written` keeps such a
        // tune silent from sample zero, not just "silent after the filter
        // settles".
        if self.host.speaker_written {
            for (sample, beep) in mono.iter_mut().zip(self.beeper.iter()) {
                *sample += BEEPER_GAIN * beep;
            }
        }
        self.beeper.clear();

        for (i, sample) in mono.iter().enumerate() {
            if let Some(slot) = out.get_mut(i * 2..i * 2 + 2) {
                slot[0] = *sample;
                slot[1] = *sample;
            }
        }
    }

    /// Calls `address` and runs until it returns or the budget expires.
    /// Returns false if the routine never came back inside its budget — the
    /// spec's Failure section requires that be reported rather than played as
    /// silence, so `new()` turns a false here into `AyError::InitDidNotReturn`.
    fn call(&mut self, address: u16) -> bool {
        self.host.cpu.regs.pc = address;
        // A previous frame may have left the CPU halted at SENTINEL (see the
        // HALT parked there in `new()`): the core's `halt` flag makes every
        // fetch a phantom NOP regardless of what `regs.pc` points at, and
        // only an accepted interrupt clears it normally. This player drives
        // PC directly instead, so it must clear the flag itself or the
        // routine we are about to "call" would never actually execute.
        self.host.cpu.halt = false;
        self.host.cpu.regs.sp = self.host.cpu.regs.sp.wrapping_sub(2);
        let sp = self.host.cpu.regs.sp;
        self.host.mem.write(sp, (SENTINEL & 0xFF) as u8);
        self.host
            .mem
            .write(sp.wrapping_add(1), (SENTINEL >> 8) as u8);

        for _ in 0..CALL_BUDGET {
            self.step_with_chip();
            if self.host.cpu.regs.pc == SENTINEL {
                return true;
            }
        }
        false
    }

    /// One host cycle, with the chip kept in step and any AY port write
    /// applied the moment it happens — a write applied a frame late is a
    /// note in the wrong place.
    fn step_with_chip(&mut self) {
        self.host.step();
        self.t_states += 1;
        if let Some((register, value)) = self.host.ay_write.take() {
            self.chip.select_register(register);
            self.chip.write_data(value);
        }
        self.ay_tick_accumulator += 1;
        if self.ay_tick_accumulator >= AY_TICK_DIVISOR {
            self.ay_tick_accumulator = 0;
            self.chip.tick();
        }
        self.sample_beeper();
    }

    /// Called from `step_with_chip`, once per host cycle (one half T-state).
    /// Downsamples the speaker bit to `samples_per_frame` output samples
    /// with a Bresenham-style accumulator rather than a fixed T-state
    /// divisor: `step_with_chip` runs `T_STATES_PER_FRAME * 2` times per
    /// frame (see `t_states`'s doc), so adding `samples_per_frame` every
    /// call and firing whenever the running total reaches that many host
    /// cycles emits exactly `samples_per_frame` samples a frame, every
    /// frame, with no truncation and no drift.
    ///
    /// Each downsampled sample is passed through the DC-blocking high-pass
    /// (`dc_r`, `dc_prev_x`, `dc_prev_y`) before it is buffered, so a level
    /// the tune holds — rather than toggles — decays instead of sitting as
    /// a constant offset in every later frame. This runs unconditionally,
    /// not gated on `host.speaker_written`: the filter's state must be
    /// continuous from the start of playback, or the first real write would
    /// hit an untracked filter and produce the wrong transient. `render`'s
    /// gate is what keeps a tune that never touches the port silent (see
    /// its comment) — this function only shapes the signal, it does not
    /// decide whether the signal is heard.
    fn sample_beeper(&mut self) {
        self.beeper_accumulator += self.samples_per_frame as u32;
        if self.beeper_accumulator >= T_STATES_PER_FRAME * 2 {
            self.beeper_accumulator -= T_STATES_PER_FRAME * 2;
            let level = if self.host.speaker { 1.0 } else { -1.0 };
            let filtered = level - self.dc_prev_x + self.dc_r * self.dc_prev_y;
            self.dc_prev_x = level;
            self.dc_prev_y = filtered;
            if self.beeper.len() < self.samples_per_frame {
                self.beeper.push(filtered);
            }
        }
    }
}
