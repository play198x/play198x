use crate::host::memory::Memory;
use emu198x_zilog_z80::{BusOp, Z80};

/// The host an `.ay` tune runs on: 64 KB of RAM, a Z80, and the two ports a
/// tune can make a noise with. No ROM, no display, no keyboard, no tape.
///
/// The machine it stands in for is the 128K Spectrum. The AY is 128K-only
/// hardware — a 48K has no sound chip at all — and `player::ay`'s clock and
/// frame-length constants are that machine's.
pub struct SpectrumHost {
    pub cpu: Z80,
    pub mem: Memory,
    /// Register the AY has selected, via port 0xFFFD.
    pub ay_register: u8,
    /// Latest value written to an AY register, and which one — drained by the
    /// player, which owns the chip.
    pub ay_write: Option<(u8, u8)>,
    /// Set on any write to an AY data register (port 0xBFFD). Mirrors
    /// `speaker_written` for the same reason: `ay_write` above is drained
    /// every host cycle by the player that owns the chip, so nothing
    /// survives in it to inspect once playback has run.
    ///
    /// No production code reads this field — it exists as instrumentation
    /// for `tests/ay_corpus.rs`'s sweep, which needs to tell a tune that
    /// drives the chip apart from one that only ever selects a register, or
    /// never touches the AY at all, to measure how many real tunes drive
    /// the beeper and the chip together (the case the mix has no headroom
    /// budget for; see that test's module doc). One branch in code that
    /// must run regardless, so the cost of carrying it is one bool.
    pub ay_written: bool,
    /// Bit 4 of the last write to port 0xFE: the speaker. This is the whole
    /// of the beeper.
    pub speaker: bool,
    /// Set on any write to port 0xFE, so a tune that only uses the beeper can
    /// be told from one that makes no sound at all.
    pub speaker_written: bool,
}

impl Default for SpectrumHost {
    fn default() -> Self {
        Self::new()
    }
}

impl SpectrumHost {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cpu: Z80::new(),
            mem: Memory::new(),
            ay_register: 0,
            ay_write: None,
            ay_written: false,
            speaker: false,
            speaker_written: false,
        }
    }

    /// One CPU cycle, with this host answering the bus.
    ///
    /// `bus_request()` collapses the Z80's held strobes into one transaction
    /// per M-cycle, which is what the crate documents for "ordinary host
    /// dispatchers" — driving the raw pins directly would double-serve every
    /// read.
    pub fn step(&mut self) {
        self.cpu.tick();
        match self.cpu.bus_request() {
            Some(BusOp::MemRead) => self.cpu.data_in = self.mem.read(self.cpu.addr),
            Some(BusOp::MemWrite) => self.mem.write(self.cpu.addr, self.cpu.data),
            Some(BusOp::IoWrite) => self.io_write(self.cpu.addr, self.cpu.data),
            Some(BusOp::IoRead) => self.cpu.data_in = 0xFF,
            Some(BusOp::IntAck) => self.cpu.data_in = 0xFF,
            None => {}
        }
    }

    /// Spectrum port decoding is partial: the address lines that matter are
    /// the only ones decoded, which is why a tune can write the AY at 0xFFFD
    /// and the beeper at any even address.
    fn io_write(&mut self, port: u16, value: u8) {
        if port & 0xC002 == 0xC000 {
            self.ay_register = value & 0x0F;
        } else if port & 0xC002 == 0x8000 {
            self.ay_write = Some((self.ay_register, value));
            self.ay_written = true;
        } else if port & 0x0001 == 0 {
            self.speaker = value & 0x10 != 0;
            self.speaker_written = true;
        }
    }
}
