pub mod format;

use crate::host::spectrum::SpectrumHost;
use emu198x_gi_ay_3_8910::Ay3_8910;
use format::{AyError, AyFile, Song};

// `T_STATES_PER_FRAME` and `AY_CLOCK_HZ` must come from the same machine:
// the AY is 128K-only hardware (an `.ay` file targets the 128K's sound, not
// the 48K's, which has no AY at all), so both constants below are the 128K
// Spectrum's. Pairing the 48K's frame length (69,888 T-states, from its
// 3.5MHz clock) with the 128K's AY clock puts two machines' numbers in one
// model and under-feeds the chip's downsampler by 524 ticks a frame: a
// silent gap at the end of every frame, heard as a 50Hz buzz over playback
// that is fractionally slow. `AY_CLOCK_HZ` is not the constant to move to
// make the arithmetic close, either — it sets the pitch the chip produces,
// so lowering it detunes every tune to cancel an error in the frame length.
/// T-states in one 50Hz frame on a 128K Spectrum: 228 T-states/line x 311
/// lines = 70,908, from a 3,546,900Hz CPU clock (≈50.02Hz refresh).
const T_STATES_PER_FRAME: u32 = 70_908;
/// Where the stub parks a return address so a call's end is detectable.
const SENTINEL: u16 = 0xFFFF;
/// How long `new()` gives a tune's init routine to return.
///
/// Init runs once and may do real work — unpacking, building tables — so it
/// gets four 50Hz frames rather than one. Overrunning it costs nothing but
/// the file, which is refused as [`AyError::InitDidNotReturn`].
///
/// `call()`'s loop counts iterations of `step_with_chip`, and
/// `step_with_chip` is one half T-state (see `t_states`'s field doc) — the
/// same half-T-state unit `frame()` compares against `T_STATES_PER_FRAME *
/// 2`, not `T_STATES_PER_FRAME` alone. Written as `T_STATES_PER_FRAME * 2 *
/// 4` rather than the equal-valued `T_STATES_PER_FRAME * 8` so the `* 2` for
/// the unit and the `* 4` for the budget's size don't collapse into one
/// number that reads like eight frames.
const INIT_BUDGET: u32 = T_STATES_PER_FRAME * 2 * 4;

/// How long `frame()` gives a tune's interrupt routine to return: exactly
/// one frame, in the same half-T-state unit.
///
/// A play routine called 50 times a second has one frame to finish in on
/// real hardware, so that is what it gets here. A larger budget does not
/// rescue a routine that overruns; it makes `frame()` consume several
/// frames' worth of CPU for one frame of audio, which plays the tune fast
/// and hands the chip more samples than `end_frame` has room for — they are
/// dropped, silently.
const INTERRUPT_BUDGET: u32 = T_STATES_PER_FRAME * 2;
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
/// `a_beeper_only_tune_is_audible`'s fixture measures ~0.99, which is not
/// representative of that range: its toggle burst is brief and then holds
/// for most of a 20ms frame, behaving close to a ~25Hz square wave
/// (`N≈960`) — the one point on this curve where `R^N` is small enough for
/// the peak to approach `2A`. No real beeper engine plays a ~25Hz tone, so
/// that figure describes the fixture's shape, not typical playback.
///
/// Left at 0.5 rather than lowered to buy the mix headroom. The corpus is
/// what settles that: 7 of 553 files drive the chip and the beeper
/// together, 10 at ten times the frame budget, so detuning the other 522
/// ay-only tunes to serve them would be paying the wrong bill. The headroom
/// itself comes from AC-coupling the chip's output instead — see
/// [`AyPlayer::render`], where the chip's unipolar sum is the thing actually
/// eating the budget.
const BEEPER_GAIN: f32 = 0.5;

/// Target -3dB cutoff for the DC-blocking high-pass both signal paths run
/// through (see [`DcBlocker`]). Real hardware AC-couples the amplifier, so a
/// held level decays to silence instead of sitting as a constant offset;
/// this is the digital equivalent. Chosen deliberately,
/// not copied from a canned `R`: a 1-bit beeper engine has no practical use
/// below roughly 50Hz (the resolution and the audibility both fall apart
/// down there), so 35Hz sits well clear of anything a tune would use as a
/// tone, while being high enough that the filter settles within about one
/// output frame (20ms) rather than leaving a real click's decay tail
/// hanging into the next frame or two.
const DC_BLOCKER_CUTOFF_HZ: f32 = 35.0;

/// The one-pole DC-blocking high-pass both the beeper and the AY chip run
/// through: `y[n] = x[n] - x[n-1] + R*y[n-1]`.
///
/// One implementation, two instances. The two signals need the same filter
/// and must not share its state — a filter is a memory of what it has
/// already seen, so mixing two sources through one instance would make each
/// one's transients appear in the other's output.
struct DcBlocker {
    /// The pole, `R`, derived from the caller's `sample_rate` so the cutoff
    /// (see [`DC_BLOCKER_CUTOFF_HZ`]) — and so the decay time — stays the
    /// same whatever rate this player runs at. A fixed literal would tie the
    /// cutoff to whichever rate happened to be used when it was chosen, and
    /// this player takes an arbitrary caller-supplied one (44.1kHz and 48kHz
    /// are both realistic), so the two would otherwise sound subtly
    /// different.
    r: f32,
    /// `x[n-1]`: the previous input sample.
    prev_x: f32,
    /// `y[n-1]`: the previous output. Carried across frame boundaries, never
    /// reset per frame — resetting it would reintroduce a step at every
    /// frame edge, audible as a click at 50Hz.
    prev_y: f32,
}

impl DcBlocker {
    /// `R = 1 - 2*pi*fc/fs` for a one-pole high-pass's -3dB point at `fc`.
    fn new(sample_rate: u32) -> Self {
        Self {
            r: 1.0 - (2.0 * std::f32::consts::PI * DC_BLOCKER_CUTOFF_HZ) / sample_rate as f32,
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    fn filter(&mut self, x: f32) -> f32 {
        let y = x - self.prev_x + self.r * self.prev_y;
        self.prev_x = x;
        self.prev_y = y;
        y
    }
}

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
    ///
    /// Counted with wrapping arithmetic, and read only as a difference. At
    /// 7,090,800 half T-states a second a `u32` runs out after 10 minutes 6
    /// seconds, which is an ordinary length for a tune; `+=` would panic
    /// there under the overflow checks the default dev profile turns on, and
    /// a panic reachable from the audio path is against the posture the
    /// workspace lints spell out. Wrapping is not a workaround here but the
    /// right reading: `frame()` compares `t_states - before` across a few
    /// thousand ticks, and modular subtraction gives the true difference
    /// whether or not the counter wrapped in between.
    t_states: u32,
    /// One sample of the speaker bit per output sample, filled by
    /// `sample_beeper` as the CPU runs and drained by `render`.
    beeper: Vec<f32>,
    /// The frame's mixed mono signal, before it is written out as stereo.
    /// Owned by the player rather than built per call because `render` must
    /// allocate nothing (see its doc), and its size is fixed at
    /// construction: `samples_per_frame`, every frame.
    mono: Vec<f32>,
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
    /// AC-couples the speaker bit, so a level the tune holds decays instead
    /// of sitting as a constant offset in every later frame.
    beeper_dc: DcBlocker,
    /// AC-couples the chip's output, for the same reason and against a
    /// larger offset — see [`AyPlayer::render`].
    ay_dc: DcBlocker,
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
        let mut player = Self {
            host,
            song,
            frames_played: 0,
            chip: Ay3_8910::new(AY_CLOCK_HZ, sample_rate, samples_per_frame),
            samples_per_frame,
            ay_tick_accumulator: 0,
            t_states: 0,
            beeper: Vec::with_capacity(samples_per_frame),
            mono: vec![0.0; samples_per_frame],
            beeper_accumulator: 0,
            beeper_dc: DcBlocker::new(sample_rate),
            ay_dc: DcBlocker::new(sample_rate),
        };
        let init = if player.song.init == 0 {
            player.song.blocks.first().map_or(0, |b| b.address)
        } else {
            player.song.init
        };
        if !player.call(init, INIT_BUDGET) {
            return Err(AyError::InitDidNotReturn);
        }
        Ok(player)
    }

    /// One 50Hz frame: the tune's interrupt routine, then the rest of the
    /// frame's cycles so anything it started can finish, with the AY chip
    /// clocked throughout.
    ///
    /// Returns whether the interrupt routine returned inside its one-frame
    /// budget. A tune whose play routine never comes back is
    /// still rendered — it has usually already written the chip registers
    /// this frame — but it is a fact about the playback, and a caller that
    /// wants to count it can. `tests/ay_corpus.rs` does.
    pub fn frame(&mut self) -> bool {
        let before = self.t_states;
        let returned = self.call(self.song.interrupt, INTERRUPT_BUDGET);
        // `t_states` counts half T-states (see the field doc), so a real
        // frame's worth of T_STATES_PER_FRAME T-states is twice that many
        // `step_with_chip` calls.
        while self.t_states.wrapping_sub(before) < T_STATES_PER_FRAME * 2 {
            self.step_with_chip();
        }
        self.frames_played += 1;
        returned
    }

    /// Fill `out` with interleaved stereo frames, returning how many it
    /// wrote. Call once per [`AyPlayer::frame`].
    ///
    /// One `frame()` produces `sample_rate / 50` frames of audio, so a buffer
    /// shorter than that gets as much as it holds and the rest is dropped;
    /// a longer one keeps its tail untouched. **Allocates nothing** —
    /// `tests/engine_allocations.rs` counts, because a player that allocates
    /// on the audio thread glitches on somebody else's machine and never on
    /// yours. The same contract [`crate::engine::Engine::render`] keeps.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        // Taken out of `self` so the chip and the DC blocker can be borrowed
        // mutably alongside it, and put back before returning. The `Vec`
        // itself is never reallocated, so this moves a header, not data.
        let mut mono = std::mem::take(&mut self.mono);
        self.chip.end_frame(&mut mono);

        // AC-couple the chip before anything is mixed into it.
        // `emu198x-gi-ay-3-8910`'s `compute_output` sums three channels from
        // a *unipolar* volume table (0.0 to 1.0) and divides by three, so
        // its output never goes negative: a square wave arrives riding on a
        // DC offset about half its own peak, and roughly half the headroom
        // is spent on a component that is not audio. Real hardware couples
        // the chip and the speaker through the same amplifier, so this is
        // the physically right model as well as the one that buys the room —
        // and it is host-side mixing, not chip emulation, so it stays inside
        // the thin-consumer rule.
        //
        // It also has to happen before the beeper is added, and through its
        // own filter state. `sample_beeper` already AC-couples the speaker;
        // adding an un-coupled chip signal to an already-coupled beeper
        // would mix two signals sitting on different reference levels, and
        // sharing one filter instance between them would put each one's
        // transients into the other's output.
        for sample in &mut mono {
            *sample = self.ay_dc.filter(*sample);
        }

        // A tune that never wrote the speaker port has no beeper signal,
        // and this gate is what keeps it silent from sample zero rather
        // than "silent once the filter settles". `sample_beeper` runs
        // unconditionally from the start of playback — the filter has to
        // see every sample to stay continuous, see its comment — so a tune
        // that never touches the port is still feeding it a constant -1.0.
        // Against the filter's zeroed initial state that constant looks
        // exactly like an edge, and the filter answers with a start-up
        // transient that corresponds to no real event. Without this gate
        // the transient would land in the first frame of every tune that
        // makes no beeper sound at all.
        if self.host.speaker_written {
            for (sample, beep) in mono.iter_mut().zip(self.beeper.iter()) {
                *sample += BEEPER_GAIN * beep;
            }
        }
        self.beeper.clear();

        // A backstop, not the mechanism. AC-coupling above is what keeps the
        // mix inside range; this only states the range so a consumer can
        // rely on it — an AudioWorklet handed a value above 1.0 hard-clips,
        // and this crate should not be the thing that hands it one. With the
        // coupling in place nothing in the 696-file corpus reaches it.
        let mut frames = 0;
        for (slot, sample) in out.as_chunks_mut::<2>().0.iter_mut().zip(mono.iter()) {
            let clamped = sample.clamp(-1.0, 1.0);
            slot[0] = clamped;
            slot[1] = clamped;
            frames += 1;
        }

        self.mono = mono;
        frames
    }

    /// Calls `address` and runs until it returns or `budget` half T-states
    /// expire.
    ///
    /// Returns false if the routine never came back inside its budget — the
    /// spec's Failure section requires that be reported rather than played as
    /// silence, so `new()` turns a false here into `AyError::InitDidNotReturn`
    /// and `frame()` hands it back to its caller.
    fn call(&mut self, address: u16, budget: u32) -> bool {
        self.host.cpu.regs.pc = address;
        // A previous frame may have left the CPU halted at SENTINEL (see the
        // HALT parked there in `new()`): the core's `halt` flag makes every
        // fetch a phantom NOP regardless of what `regs.pc` points at, and
        // only an accepted interrupt clears it normally. This player drives
        // PC directly instead, so it must clear the flag itself or the
        // routine we are about to "call" would never actually execute.
        self.host.cpu.halt = false;
        let sp_before = self.host.cpu.regs.sp;
        self.host.cpu.regs.sp = sp_before.wrapping_sub(2);
        let sp = self.host.cpu.regs.sp;
        self.host.mem.write(sp, (SENTINEL & 0xFF) as u8);
        self.host
            .mem
            .write(sp.wrapping_add(1), (SENTINEL >> 8) as u8);

        for _ in 0..budget {
            self.step_with_chip();
            if self.host.cpu.regs.pc == SENTINEL {
                return true;
            }
        }

        // Nothing popped the sentinel this call pushed, so put the stack
        // back where it was found. Without this, an interrupt routine that
        // never returns costs two bytes of stack every frame — 100 bytes a
        // second, 30 KB over five minutes — and the Spectrum's stack grows
        // downward through the tune's own code and data. That is silent
        // memory corruption of the program still playing, and it looks like
        // a tune that gradually falls apart rather than like a bug here.
        self.host.cpu.regs.sp = sp_before;
        false
    }

    /// One host cycle, with the chip kept in step and any AY port write
    /// applied the moment it happens — a write applied a frame late is a
    /// note in the wrong place.
    fn step_with_chip(&mut self) {
        self.host.step();
        self.t_states = self.t_states.wrapping_add(1);
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
    /// Each downsampled sample is passed through this player's beeper
    /// [`DcBlocker`] before it is buffered, so a level
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
            let filtered = self.beeper_dc.filter(level);
            if self.beeper.len() < self.samples_per_frame {
                self.beeper.push(filtered);
            }
        }
    }
}
