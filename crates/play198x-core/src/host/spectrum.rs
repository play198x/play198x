use crate::host::memory::Memory;
use emu198x_zilog_z80::{BusOp, Z80};

/// Re-exported because [`SpectrumHost::ay`] is a public field of its type:
/// without this, a caller outside this crate can see the field and cannot
/// name what is in it.
pub use emu198x_gi_ay_3_8910::Ay3_8910;

/// What an `IN` from a port nothing answers reads back.
///
/// The Z80's data bus has no keeper: with no device driving it, the pull-ups
/// win and every line reads high. This host wires up a sound chip and a
/// paging latch and nothing else, so every other port a tune probes — a
/// joystick, a disk interface, the keyboard — reads as absent, which is what
/// it is. So does the AY's own port on a host with no chip fitted.
pub(crate) const UNATTACHED_BUS: u8 = 0xFF;

/// The address lines the 128K's memory-paging latch decodes: A15 and A1,
/// both low, i.e. `(port & 0x8002) == 0x0000`
/// (`reference/by-system/sinclair-zx-spectrum/128k-memory-paging.md`,
/// § Port Decoding, for the 128K and +2).
///
/// Decoded partially because the hardware decodes partially, and the
/// difference is not academic: `$7FFD` is the address the manuals give, but
/// `$5FFD`, `$3FFD`, `$1FFD` and every other address with those two lines
/// low reach the same latch and page the same memory. A host matching the
/// exact port would run a tune that pages through any of the others while
/// showing it an address space that never moved — the failure being silent,
/// because a tune reading the wrong bank finds data-shaped bytes there.
///
/// The +2A and +3 decode differently (`(port & 0xC002) == 0x4000`, which
/// excludes `$1FFD` because that became a second paging port). This host
/// stands in for the 128K, the machine whose AY decode the writes below
/// already use.
const PAGING_DECODE_MASK: u16 = 0x8002;
const PAGING_DECODE_MATCH: u16 = 0x0000;

/// The address lines the AY's register-select port decodes: A15 and A14
/// high, A1 low, i.e. `(port & 0xC002) == 0xC000` — `$FFFD` and `$DFFD`
/// (`syntheses/zx-spectrum/128k-extras.md`, § I/O ports).
///
/// Named rather than written twice: a write here selects a register and a
/// read here reads that register back, so both paths must agree, and two
/// copies of one decode are a pair that can drift apart without anything
/// failing to compile.
///
/// The cited synthesis and this code disagree about the read, and the code
/// is right: § I/O ports there says "`$BFFD` (data): read/write the selected
/// register's value", but the AY's data lines are only driven onto the bus
/// for an `IN` from the *select* port. `emu198x-gi-ay-3-8910`'s
/// `read_data()` documents it as "IN from port $FFFD" and matches Fuse's
/// `ay_registerport_read`, and it is what the archive's read-back tunes
/// actually do. Recorded here rather than silently: a reader who checks the
/// synthesis will otherwise think this decode is wrong.
pub(crate) const AY_SELECT_DECODE_MASK: u16 = 0xC002;
pub(crate) const AY_SELECT_DECODE_MATCH: u16 = 0xC000;

/// The host an `.ay` tune runs on: a 128K Spectrum's RAM, a Z80, and the
/// ports a tune can make a noise with or page memory through. No ROM, no
/// display, no keyboard, no tape.
///
/// The machine it stands in for is the 128K Spectrum throughout. The AY is
/// 128K-only hardware — a 48K has no sound chip at all — `player::ay`'s
/// clock and frame-length constants are that machine's, and so is
/// [`Memory`]'s eight-bank map.
pub struct SpectrumHost {
    pub cpu: Z80,
    pub mem: Memory,
    /// The sound chip, if one is fitted. `None` is a host with no AY on the
    /// board: its port reads as [`UNATTACHED_BUS`] and its writes go
    /// nowhere, which is what a machine without the chip does.
    ///
    /// The chip belongs to the host and not to whatever is driving it,
    /// because [`SpectrumHost::step`] has to answer the bus. A chip owned by
    /// a caller means the caller has to patch the answer afterwards, and
    /// then two callers of the same public `step` get two different answers
    /// from the same port.
    pub ay: Option<Ay3_8910>,
    /// Register the AY has selected, via port 0xFFFD.
    pub ay_register: u8,
    /// Set on any write to an AY data register (port 0xBFFD). Mirrors
    /// `speaker_written` for the same reason: the value itself goes into the
    /// chip, so nothing survives on this struct to inspect once playback has
    /// run.
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
    /// Set on any write that reaches the 128K memory-paging latch *and is
    /// acted on* — see [`PAGING_DECODE_MASK`] for which addresses reach it,
    /// and [`Memory::page`] for when the latch stops accepting writes.
    ///
    /// A write the latch has been locked out of does not count. What this
    /// measures is how much of an archive this host's memory model has to
    /// answer for, and a discarded write is one it does not.
    ///
    /// No production code reads it — it exists as instrumentation for
    /// `tests/ay_corpus.rs`'s sweep, which reports how much of a real
    /// archive pages memory at all, and so is the only thing that can say
    /// whether a change to the paging model reached the tunes it was
    /// written for. Mirrors `ay_written` and `speaker_written` above for
    /// the same reason.
    pub paging_written: bool,
    /// Bitwise-OR of every value the paging latch accepted since
    /// construction — the same acceptance test as `paging_written` above.
    ///
    /// Zero after any number of writes means every one of them wrote 0x00 —
    /// RAM bank 0 at 0xC000, screen 0, ROM 0, paging unlocked, the same
    /// state the machine powers on in. Any bit set says which of the port's
    /// functions the tunes in an archive actually use, which is a different
    /// question from how many of them touch the port.
    pub paging_values_seen: u8,
    /// The port most recently read via an `IoRead` bus request, if any
    /// since the last drain. The read is already answered by the time this
    /// is set — see [`SpectrumHost::step`]; this only records that it
    /// happened, and which port.
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
            ay: None,
            ay_register: 0,
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
                let port = self.cpu.addr;
                self.cpu.data_in = self.io_read(port);
                self.io_read = Some(port);
            }
            Some(BusOp::IntAck) => self.cpu.data_in = 0xFF,
            None => {}
        }
    }

    /// What answers an `IN` from `port`.
    ///
    /// The AY drives the bus for a read of its select port, and only when a
    /// chip is fitted; nothing else on this host drives it at all, so every
    /// other port reads as [`UNATTACHED_BUS`].
    ///
    /// The chip's selected register is set from this host's last select
    /// write rather than assumed to be in step. It matters for a tune that
    /// selects a register and reads it straight back without writing data to
    /// it first — real hardware answers that, and a chip whose selection
    /// only moved on data writes would not.
    fn io_read(&mut self, port: u16) -> u8 {
        if port & AY_SELECT_DECODE_MASK != AY_SELECT_DECODE_MATCH {
            return UNATTACHED_BUS;
        }
        let register = self.ay_register;
        match &mut self.ay {
            Some(ay) => {
                ay.select_register(register);
                ay.read_data()
            }
            None => UNATTACHED_BUS,
        }
    }

    /// Spectrum port decoding is partial: the address lines that matter are
    /// the only ones decoded, which is why a tune can write the AY at 0xFFFD
    /// and the beeper at any even address.
    ///
    /// The paging latch is decoded on its own rather than as another arm of
    /// the chain, because partial decoding lets one write reach two devices
    /// at once and the latch is the pair most likely to collide: it answers
    /// to A15 and A1 low, the ULA answers to A0 low, and any even address
    /// below 0x8000 with A1 clear satisfies both. On the machine, both
    /// respond. An `else if` here would hand the write to whichever arm was
    /// written first and lose the other.
    fn io_write(&mut self, port: u16, value: u8) {
        if port & PAGING_DECODE_MASK == PAGING_DECODE_MATCH && self.mem.page(value) {
            self.paging_written = true;
            self.paging_values_seen |= value;
        }

        if port & AY_SELECT_DECODE_MASK == AY_SELECT_DECODE_MATCH {
            self.ay_register = value & 0x0F;
        } else if port & 0xC002 == 0x8000 {
            self.ay_written = true;
            let register = self.ay_register;
            if let Some(ay) = &mut self.ay {
                ay.select_register(register);
                ay.write_data(value);
            }
        } else if port & 0x0001 == 0 {
            self.speaker = value & 0x10 != 0;
            self.speaker_written = true;
        }
    }
}
