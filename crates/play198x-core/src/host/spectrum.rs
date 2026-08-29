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
    /// Set on any write to port 0x7FFD, the 128K memory-paging port. This
    /// host models 128K *timing* (`player::ay`'s frame-length and AY-clock
    /// constants are the 128K's) but its memory is the flat 64 KB declared
    /// on this struct's doc, so a paging write has nowhere to go and this
    /// host drops it exactly as it always has.
    ///
    /// No production code reads this field — it exists as instrumentation
    /// for `tests/ay_corpus.rs`'s sweep, which needs to tell "never touches
    /// the port" from "pages RAM banks, the screen, or the ROM in and out"
    /// (running code this host does not model) before that gap can be
    /// judged safe to leave unfixed. Mirrors `ay_written` and
    /// `speaker_written` above for the same reason.
    pub paging_written: bool,
    /// Bitwise-OR of every value written to port 0x7FFD since construction.
    ///
    /// Zero after any number of writes means every one of them wrote 0x00 —
    /// RAM bank 0 at 0xC000, screen 0, ROM 0, paging unlocked, the power-on
    /// default a flat 64 KB model already behaves as — so those tunes cost
    /// nothing by the write being dropped. Any bit set means at least one
    /// write asked this host for a bank, screen, or ROM other than that
    /// default, which it cannot give it.
    pub paging_values_seen: u8,
    /// The port most recently read via an `IoRead` bus request, if any
    /// since the last drain. Drained by `AyPlayer::step_with_chip`, which
    /// is where the AY chip that could actually answer a real AY read
    /// lives — this host cannot answer one itself, see `step`'s doc.
    ///
    /// No production code reads this field — it exists as instrumentation
    /// for the read-side counters on [`crate::player::ay::AyPlayer`], which
    /// `tests/ay_corpus.rs`'s sweep reports.
    pub io_read: Option<u16>,
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
            paging_written: false,
            paging_values_seen: 0,
            io_read: None,
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
            Some(BusOp::IoRead) => {
                self.cpu.data_in = 0xFF;
                self.io_read = Some(self.cpu.addr);
            }
            Some(BusOp::IntAck) => self.cpu.data_in = 0xFF,
            None => {}
        }
    }

    /// Spectrum port decoding is partial: the address lines that matter are
    /// the only ones decoded, which is why a tune can write the AY at 0xFFFD
    /// and the beeper at any even address.
    fn io_write(&mut self, port: u16, value: u8) {
        if port == 0x7FFD {
            // Recorded, not acted on: see `paging_written`'s doc for why —
            // this host's memory is a flat 64 KB, so there is no bank to
            // switch. Matched on the exact port rather than a partial
            // decode: the question this exists to answer is specifically
            // about the 128K paging port real tunes write, not about every
            // address a looser decode would also catch.
            self.paging_written = true;
            self.paging_values_seen |= value;
        } else if port & 0xC002 == 0xC000 {
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
