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
const CALL_BUDGET: u32 = T_STATES_PER_FRAME * 8;
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
        // too; the format does not special-case the flags. Verified against
        // Project AY's own technical documentation and the vgmrips format
        // wiki, not inferred from field names.
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
    }
}
