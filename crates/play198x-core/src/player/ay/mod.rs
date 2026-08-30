pub mod format;

use crate::host::spectrum::{
    AY_SELECT_DECODE_MASK, AY_SELECT_DECODE_MATCH, SpectrumHost, UNATTACHED_BUS,
};
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
/// The return address `call` pushes, so a routine coming back is
/// detectable. Nothing is written to this address and nothing may be: it
/// sits in the 16 KB window a `$7FFD` write repoints, so what is visible
/// there depends on which RAM bank a tune last selected. `call` watches for
/// PC reaching it and `frame` stops running the CPU from that point, so the
/// byte at this address is never fetched.
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
/// How far past its budget `call` will run to finish the instruction in
/// flight, in half T-states.
///
/// A give-up, not a guarantee, and the difference matters: no finite bound
/// can cover every instruction, because this core absorbs prefix bytes into
/// the instruction that follows them. Sixteen `DD` bytes and a `NOP` are one
/// instruction to it and retire after 136 half T-states, and the chain has
/// no length limit. A bound is still wanted — without one this is a second
/// budget with no ceiling, on a routine that has already proved it does not
/// return.
///
/// 64 is set from what real tunes reach rather than from the instruction
/// set. Measured across all 1,536 playable songs: 66 of them use the tail at
/// all, over 4,666 frames of a possible 384,000, the longest is 36 half
/// T-states (Technician Ted's song 1), and the bound is hit zero times. A
/// tune that did hit it would resume mid-instruction exactly as it did
/// before this existed, which is the old behaviour rather than a new fault.
///
/// The cost is that a frame whose routine overran can run up to this many
/// half T-states long — 0.045% of a frame, on the 4,666 frames that use it.
const INSTRUCTION_TAIL: u32 = 64;

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
/// what settles that: 6 of the 553 files on the sweep's song-0 pass drive
/// the chip and the beeper together, and 13 of the 1,536 songs across the
/// whole archive do, so detuning the other 1,475 ay-only songs to serve
/// them would be paying the wrong bill. The headroom
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
    /// same whatever rate this player runs at, and floored at 0 so a
    /// nonsense rate cannot put it outside the region where this filter
    /// converges. See [`DcBlocker::new`]. A fixed literal would tie the
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
    /// `R = 1 - 2*pi*fc/fs` for a one-pole high-pass's -3dB point at `fc`,
    /// held inside the region where that filter is stable.
    ///
    /// `y[n] = x[n] - x[n-1] + R*y[n-1]` converges only while `|R| < 1`. `R`
    /// can never exceed 1 for a positive rate, so the bound that matters is
    /// the lower one: `R > -1` needs `fs > pi*fc`, about 110Hz at this
    /// cutoff. Below that the filter does not merely sound wrong, it grows
    /// without limit — at `fs = 0` a tune driving the speaker reaches an
    /// infinite peak by its eighteenth frame, and `render`'s clamp cannot
    /// help because `inf` clamps to 1.0 and takes the whole mix with it.
    ///
    /// Clamped at 0 rather than at the -1 stability edge, which is two
    /// judgements rather than one. `R` goes negative below `2*pi*fc` (about
    /// 220Hz), where the filter alternates sign each sample instead of
    /// decaying — technically stable and nothing like the intended
    /// behaviour. And a floor at the edge of stability is a floor with no
    /// margin: `R = -0.999` converges so slowly it is indistinguishable
    /// from divergence over any frame count anyone would render. `R = 0`
    /// leaves `y[n] = x[n] - x[n-1]`, which still blocks DC and cannot
    /// diverge, so a nonsense rate degrades to a plain difference rather
    /// than to infinity.
    ///
    /// This is the floor that matters, and it is not the one on
    /// `AyPlayer::new`'s `sample_rate`: that one stops a division by zero
    /// and keeps a frame's sample count non-zero, and it leaves `R` at
    /// -218.9.
    fn new(sample_rate: u32) -> Self {
        let r = 1.0 - (2.0 * std::f32::consts::PI * DC_BLOCKER_CUTOFF_HZ) / sample_rate as f32;
        Self {
            // `max` and not `clamp`: `R` has no upper bound to enforce, and
            // this also catches the `NaN` that `0.0 / 0.0` would produce,
            // because `f32::max` returns the other operand for a NaN.
            r: r.max(0.0),
            prev_x: 0.0,
            prev_y: 0.0,
        }
    }

    /// Forget everything seen so far, without changing the cutoff. Used to
    /// start playback from a filter that has seen nothing — see
    /// [`AyPlayer::discard_init_output`].
    fn reset(&mut self) {
        self.prev_x = 0.0;
        self.prev_y = 0.0;
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
    samples_per_frame: usize,
    ay_tick_accumulator: u32,
    /// Half T-states elapsed — `idle_cycle` increments this once per call,
    /// and every host cycle passes through it. Each is one Rise-or-Fall
    /// half-cycle of the Z80 core
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
    /// The largest absolute sample seen before `render`'s clamp, since
    /// construction. See [`AyPlayer::peak_before_clamp`].
    peak_before_clamp: f32,
    /// Whether this tune ever read any I/O port during playback. Every port
    /// but the AY's answers `UNATTACHED_BUS` regardless of which one it is
    /// (`SpectrumHost::step`), so this counts curiosity rather than
    /// correctness — it exists so `tests/ay_corpus.rs`'s sweep can measure
    /// how many real tunes look at an `IN` result at all, before the two
    /// fields below narrow that down to the port the chip answers. No
    /// production code reads it.
    pub any_port_read: bool,
    /// Whether this tune ever read the AY's register-read port — `$FFFD`
    /// and the addresses that decode with it
    /// (`emu198x-gi-ay-3-8910`'s doc: "Data read: IN from port $FFFD").
    /// No production code reads it.
    pub ay_read: bool,
    /// How many of this tune's AY reads returned something other than
    /// `UNATTACHED_BUS`, i.e. how often the chip had a real answer to give.
    ///
    /// The pair with `ay_read` is what makes the read path measurable
    /// rather than merely present: a tune can probe the port constantly and
    /// still only ever see 0xFF, which looks identical to a host that
    /// answers nothing. No production code reads it.
    pub ay_reads_non_ff: u32,
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
        for block in &song.blocks {
            host.mem.load(block.address, &block.data);
        }
        // The file has now said everything it has to say about the address
        // space, and said it without ever naming a RAM bank. See
        // [`Memory::mirror_window_into_the_pageable_banks`] for why that
        // means every bank starts with the same window image, and what
        // breaks in the archive if it does not.
        host.mem.mirror_window_into_the_pageable_banks();

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

        // A `sample_rate` of zero is nonsense, and this crate does not get
        // to panic about it — the same ruling `Engine::new` makes, for the
        // same reason. Two floors rather than one: the rate itself, because
        // the DC blocker divides by it and would otherwise take a pole of
        // -inf; and the frame's sample count, because a rate under 50Hz has
        // no whole sample in a 50Hz frame and the chip's downsampler is
        // handed that count as a divisor.
        let sample_rate = sample_rate.max(1);
        let samples_per_frame = (sample_rate as usize / 50).max(1);
        // Fitted to the host rather than held here, because the host is what
        // answers the bus: an `IN` from the AY's port is the machine's
        // answer to give, and a chip owned by the player would leave
        // `SpectrumHost::step` answering one thing and this player another.
        host.ay = Some(Ay3_8910::new(AY_CLOCK_HZ, sample_rate, samples_per_frame));
        let mut player = Self {
            host,
            song,
            frames_played: 0,
            samples_per_frame,
            ay_tick_accumulator: 0,
            t_states: 0,
            beeper: Vec::with_capacity(samples_per_frame),
            mono: vec![0.0; samples_per_frame],
            beeper_accumulator: 0,
            beeper_dc: DcBlocker::new(sample_rate),
            ay_dc: DcBlocker::new(sample_rate),
            peak_before_clamp: 0.0,
            any_port_read: false,
            ay_read: false,
            ay_reads_non_ff: 0,
        };
        let init = if player.song.init == 0 {
            player.song.blocks.first().map_or(0, |b| b.address)
        } else {
            player.song.init
        };
        if !player.call(init, INIT_BUDGET) {
            return Err(AyError::InitDidNotReturn);
        }
        player.discard_init_output();
        Ok(player)
    }

    /// Init runs for as long as it needs — up to four frames — and it runs
    /// the whole host, so the chip has been accumulating output and the
    /// beeper buffer has been filling all the while. None of that belongs
    /// to frame 0. Without this the first rendered frame carries init's
    /// output and drops the tail of frame 0's, with the chip and the beeper
    /// offset from each other by different amounts, because they accumulate
    /// at different rates.
    ///
    /// The chip is drained rather than reset: `end_frame` is what clears
    /// its accumulator, and the samples it produces are thrown away. The
    /// beeper's DC blocker is reset outright, so playback starts from a
    /// filter that has seen nothing rather than from init's last level.
    fn discard_init_output(&mut self) {
        let mut mono = std::mem::take(&mut self.mono);
        if let Some(ay) = &mut self.host.ay {
            ay.end_frame(&mut mono);
        }
        self.mono = mono;
        self.beeper.clear();
        self.beeper_accumulator = 0;
        self.ay_tick_accumulator = 0;
        self.beeper_dc.reset();
    }

    /// One 50Hz frame: the tune's interrupt routine, then the rest of the
    /// frame's cycles with the chip and the beeper still running and the
    /// CPU stopped.
    ///
    /// Returns whether the interrupt routine returned inside its one-frame
    /// budget. A tune whose play routine never comes back is
    /// still rendered — it has usually already written the chip registers
    /// this frame — but it is a fact about the playback, and a caller that
    /// wants to count it can. `tests/ay_corpus.rs` does.
    pub fn frame(&mut self) -> bool {
        let before = self.t_states;
        let returned = self.call(self.song.interrupt, INTERRUPT_BUDGET);
        // The routine has returned; the frame has not. The chip and the
        // beeper still need the rest of the frame's cycles — and the CPU
        // must not get them. On the machine the player is idle here,
        // waiting for the next interrupt; there is nothing left for this
        // tune to execute, and `call` has stopped it at SENTINEL, which is
        // a return address and not code.
        //
        // Nothing is parked at SENTINEL to halt the CPU with, because
        // nothing at that address can be relied on. A HALT byte written
        // there is overwritten by any block that covers it: 57 of the
        // archive's 1,536 playable tunes end a run with something else at
        // 0xFFFF, measured against the model that wrote one. Banking adds a
        // second way to lose it — the address is inside the window a
        // `$7FFD` write repoints, and banks 2 and 5 are not mirrored
        // because they carry the file's image of their own fixed addresses.
        //
        // A CPU left running from there executes the tune's own bytes as
        // code for the rest of every frame, and that does not sound like a
        // crash. Ghosts'n'Goblins is the clearest case in the archive: its
        // play routine never touches the sound chip at all, and every note
        // it appeared to make came from the runaway. Target Renegade is the
        // same fault the other way up — six of its eight subtunes rendered
        // silence while the seventh played the wreckage.
        //
        // Only reached when the routine returned early: INTERRUPT_BUDGET is
        // one frame, so a routine that overran has already consumed the
        // whole of it and this loop does not run.
        //
        // `t_states` counts half T-states (see the field doc), so a real
        // frame's worth of T_STATES_PER_FRAME T-states is twice that many
        // cycles.
        while self.t_states.wrapping_sub(before) < T_STATES_PER_FRAME * 2 {
            self.idle_cycle();
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
    pub fn render_frame(&mut self, out: &mut [f32]) -> usize {
        // Taken out of `self` so the chip and the DC blocker can be borrowed
        // mutably alongside it, and put back before returning. The `Vec`
        // itself is never reallocated, so this moves a header, not data.
        let mut mono = std::mem::take(&mut self.mono);
        match &mut self.host.ay {
            Some(ay) => ay.end_frame(&mut mono),
            // Not reachable through `AyPlayer`, which fits a chip in
            // `new()` and never removes it. It exists because the host's
            // chip is an `Option` — a `SpectrumHost` driven directly can
            // have none — and because the alternative is an `unwrap` on a
            // field a caller can reach. A host with no chip has no chip
            // audio; cleared rather than left as it is, because `mem::take`
            // hands back the previous frame's samples, which would repeat
            // instead of falling silent.
            None => mono.fill(0.0),
        }

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
        // and this crate should not be the thing that hands it one.
        //
        // It does not currently engage at all. Across all 1,536 playable
        // tunes in the 696-file corpus nothing exceeds full scale before
        // the clamp; the loudest sit exactly on 1.0, which is what the DC
        // blocker returns for a step from silence to the chip's own maximum
        // (`y = x - prev_x + R*prev_y` with `prev_y` at rest). That is
        // visible only through `peak_before_clamp` — a peak taken from
        // `out` afterwards can never exceed 1.0, so it would report the
        // clamp's existence rather than the headroom behind it.
        let mut frames = 0;
        for (slot, sample) in out.as_chunks_mut::<2>().0.iter_mut().zip(mono.iter()) {
            self.peak_before_clamp = self.peak_before_clamp.max(sample.abs());
            let clamped = sample.clamp(-1.0, 1.0);
            slot[0] = clamped;
            slot[1] = clamped;
            frames += 1;
        }

        self.mono = mono;
        frames
    }

    /// How many beeper samples are buffered and not yet rendered.
    ///
    /// No production code reads this. It exists so `tests/ay_player.rs` can
    /// pin that the init routine's output does not reach frame 0, which is
    /// otherwise visible only as a slightly wrong first frame in something
    /// nobody listens to closely.
    #[must_use]
    pub fn buffered_beeper_samples(&self) -> usize {
        self.beeper.len()
    }

    /// The largest absolute sample this player has produced *before*
    /// `render`'s clamp, since construction.
    ///
    /// A backstop nobody can measure is indistinguishable from one that
    /// never engages. `render` clamps to `[-1, 1]`, so any peak taken from
    /// its output is at most 1.0 by construction — a sweep asserting
    /// "nothing exceeded full scale" against that restates the clamp rather
    /// than testing the headroom it backs up. This is the number that tells
    /// the two apart, and `tests/ay_corpus.rs` measures it rather than the
    /// rendered output for exactly that reason.
    #[must_use]
    pub fn peak_before_clamp(&self) -> f32 {
        self.peak_before_clamp
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
        // A routine that ran a HALT of its own and then overran its budget
        // leaves the flag set: the core's `halt` flag makes every fetch a
        // phantom NOP regardless of what `regs.pc` points at, and only an
        // accepted interrupt clears it normally. This player drives PC
        // directly instead, so it must clear the flag itself or the routine
        // we are about to "call" would never actually execute.
        self.host.cpu.halt = false;
        let sp_before = self.host.cpu.regs.sp;
        self.host.cpu.regs.sp = sp_before.wrapping_sub(2);
        let sp = self.host.cpu.regs.sp;
        self.host.mem.write(sp, (SENTINEL & 0xFF) as u8);
        self.host
            .mem
            .write(sp.wrapping_add(1), (SENTINEL >> 8) as u8);

        let mut retired = self.host.cpu.instructions_retired();
        let mut at_boundary = false;
        for _ in 0..budget {
            self.step_with_chip();
            let now = self.host.cpu.instructions_retired();
            at_boundary = now != retired;
            retired = now;
            // An instruction must have *finished* on this cycle, not merely
            // PC hold the sentinel during it. PC moves while an instruction
            // runs, so a bare address match fires part-way through one
            // whose operand fetches pass through 0xFFFF — and `frame` now
            // leaves the CPU exactly where this stops it, so
            // mid-instruction is a state the next call would resume from as
            // if it were a fresh fetch.
            //
            // An edge on `instructions_retired`, not `instruction_complete`:
            // that one is the obvious-looking predicate and it is wrong. The
            // crate documents it as a *level* that stays true throughout the
            // following opcode fetch, so `LD A,n` sitting at 0xFFFE
            // satisfies it while its own operand byte is still being
            // fetched — and resuming from there executes the stale opcode
            // against the next routine's first byte.
            // `tests/ay_player.rs` pins that case, and it is not a
            // hypothetical one: Star Dragon's third subtune is stopped
            // mid-instruction by the level flag, and the corrupted resume
            // is where its beeper writes came from. On the edge it returns
            // cleanly, writes neither the chip nor the speaker, and renders
            // the silence it actually contains.
            if at_boundary && self.host.cpu.regs.pc == SENTINEL {
                return true;
            }
        }

        // The budget ran out, and it can run out anywhere — including
        // part-way through an instruction, which the next call would then
        // resume as though PC pointed at an opcode. Run on to the end of
        // whatever is in flight so the next call starts from a fetch.
        //
        // Bounded, and the bound is the machine's: the longest Z80
        // instruction is 23 T-states, so [`INSTRUCTION_TAIL`] half-cycles is
        // past all of them and this cannot become a second budget. A CPU
        // halted by the routine's own HALT retires nothing, which is why
        // this gives up rather than waiting.
        for _ in 0..INSTRUCTION_TAIL {
            if at_boundary {
                break;
            }
            self.step_with_chip();
            at_boundary = self.host.cpu.instructions_retired() != retired;
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

    /// One host cycle with the CPU running: the machine executes an
    /// instruction's worth of bus activity, then the clock, the chip and the
    /// beeper advance with it.
    ///
    /// The AY's ports are the host's to serve, both ways — a read is
    /// answered inside `SpectrumHost::step` and a write is applied there.
    /// All this adds is the read-side instrumentation, taken from the answer
    /// the host has already put on the bus.
    fn step_with_chip(&mut self) {
        self.host.step();
        if let Some(port) = self.host.io_read.take() {
            self.any_port_read = true;
            if port & AY_SELECT_DECODE_MASK == AY_SELECT_DECODE_MATCH {
                self.ay_read = true;
                if self.host.cpu.data_in != UNATTACHED_BUS {
                    self.ay_reads_non_ff += 1;
                }
            }
        }
        self.idle_cycle();
    }

    /// One host cycle with the CPU stopped: the clock, the chip and the
    /// beeper advance and nothing executes.
    ///
    /// What [`AyPlayer::frame`] fills the rest of a frame with once the
    /// tune's routine has returned, and the second half of every
    /// `step_with_chip`. The chip is free-running hardware — it keeps
    /// producing sound between the CPU's visits to it — so a frame is the
    /// same length in chip ticks and output samples whether the tune's
    /// routine took two thousand T-states or seventy thousand.
    fn idle_cycle(&mut self) {
        self.t_states = self.t_states.wrapping_add(1);
        self.ay_tick_accumulator += 1;
        if self.ay_tick_accumulator >= AY_TICK_DIVISOR {
            self.ay_tick_accumulator = 0;
            if let Some(ay) = &mut self.host.ay {
                ay.tick();
            }
        }
        self.sample_beeper();
    }

    /// Called from `idle_cycle`, and so once per host cycle (one half
    /// T-state) whether or not the CPU is running that cycle.
    /// Downsamples the speaker bit to `samples_per_frame` output samples
    /// with a Bresenham-style accumulator rather than a fixed T-state
    /// divisor: `idle_cycle` runs `T_STATES_PER_FRAME * 2` times per
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

impl crate::player::pump::FrameSource for AyPlayer {
    /// Runs one frame, discarding whether the interrupt routine returned in
    /// budget. That fact is real and worth counting — `tests/ay_corpus.rs`
    /// counts it — but it is a fact about a tune, not something a pump can
    /// act on, so it stays on the inherent [`AyPlayer::frame`].
    fn frame(&mut self) {
        let _ = AyPlayer::frame(self);
    }

    fn render_frame(&mut self, out: &mut [f32]) -> usize {
        AyPlayer::render_frame(self, out)
    }

    /// Constant for this format: `.ay` is driven by the Spectrum's 50Hz
    /// interrupt and nothing varies it. The trait asks every frame anyway,
    /// for the formats where it does vary.
    fn samples_per_frame(&self) -> usize {
        self.samples_per_frame
    }
}
