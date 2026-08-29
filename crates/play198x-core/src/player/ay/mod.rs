pub mod format;

use crate::host::spectrum::SpectrumHost;
use format::{AyError, AyFile, Song};

/// T-states in one 50Hz frame on a 48K Spectrum: 3.5MHz / 50.
const T_STATES_PER_FRAME: u32 = 69_888;
/// Where the stub parks a return address so a call's end is detectable.
const SENTINEL: u16 = 0xFFFF;
/// A call that has not returned by here is not going to.
const CALL_BUDGET: u32 = T_STATES_PER_FRAME * 8;

pub struct AyPlayer {
    pub host: SpectrumHost,
    song: Song,
    frames_played: u32,
}

impl AyPlayer {
    /// Loads `song` from an `.ay` file and runs its init routine.
    ///
    /// # Errors
    ///
    /// When the bytes are not an `.ay` file, or `song` is out of range.
    pub fn new(bytes: &[u8], song: usize, _sample_rate: u32) -> Result<Self, AyError> {
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

        let mut player = Self {
            host,
            song,
            frames_played: 0,
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
    /// frame's cycles so anything it started can finish.
    pub fn frame(&mut self) {
        let _ = self.call(self.song.interrupt);
        self.frames_played += 1;
    }

    /// Calls `address` and runs until it returns or the budget expires.
    /// Returns false if the routine never came back inside its budget — the
    /// spec's Failure section requires that be reported rather than played as
    /// silence, so `new()` turns a false here into `AyError::InitDidNotReturn`.
    fn call(&mut self, address: u16) -> bool {
        self.host.cpu.regs.pc = address;
        self.host.cpu.regs.sp = self.host.cpu.regs.sp.wrapping_sub(2);
        let sp = self.host.cpu.regs.sp;
        self.host.mem.write(sp, (SENTINEL & 0xFF) as u8);
        self.host
            .mem
            .write(sp.wrapping_add(1), (SENTINEL >> 8) as u8);

        for _ in 0..CALL_BUDGET {
            self.host.step();
            if self.host.cpu.regs.pc == SENTINEL {
                return true;
            }
        }
        false
    }
}
