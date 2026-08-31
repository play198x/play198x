//! The small C64-shaped host callable PSID routines need.

use emu198x_mos_6502::M6502;
pub use emu198x_mos_sid_6581::{Sid6581, SidModel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RomKind {
    Basic,
    Kernal,
    Chargen,
}

impl RomKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Basic => "BASIC",
            Self::Kernal => "KERNAL",
            Self::Chargen => "character-generator",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resolved {
    Ram,
    Sid,
    Io,
    Rom(RomKind),
}

pub struct C64Host {
    pub cpu: M6502,
    pub sid: Sid6581,
    ram: Box<[u8; 65_536]>,
    io: Box<[u8; 4_096]>,
    port_ddr: u8,
    port_data: u8,
    pub needed_rom: Option<RomKind>,
}

impl C64Host {
    #[must_use]
    pub fn new(cpu_frequency: u64, sample_rate: u32, model: SidModel) -> Self {
        let mut cpu = M6502::new();
        cpu.regs.sp = 0xff;
        Self {
            cpu,
            sid: Sid6581::new_with_model(cpu_frequency, sample_rate, model),
            ram: Box::new([0; 65_536]),
            io: Box::new([0xff; 4_096]),
            port_ddr: 0x2f,
            port_data: 0x37,
            needed_rom: None,
        }
    }

    pub fn load(&mut self, address: u16, bytes: &[u8]) {
        let start = usize::from(address);
        self.ram[start..start + bytes.len()].copy_from_slice(bytes);
    }

    pub fn poke(&mut self, address: u16, value: u8) {
        self.ram[usize::from(address)] = value;
    }

    pub fn prepare_call(&mut self, address: u16) {
        self.write(0x0000, 0x2f);
        self.write(0x0001, bank_for_call(address));
    }

    /// Establish the timer-A latch the PSID environment specifies. The host
    /// drives calls itself; these bytes are the observable CIA state for
    /// routines that inspect or rewrite it.
    pub fn set_cia_timer_a(&mut self, latch: u16) {
        let [low, high] = latch.to_le_bytes();
        self.io[0x0c04] = low;
        self.io[0x0c05] = high;
    }

    pub fn step(&mut self) {
        self.cpu.tick();
        let address = self.cpu.addr;
        if self.cpu.rw {
            self.cpu.data_in = self.read(address);
        } else {
            self.write(address, self.cpu.data);
        }
        self.sid.tick();
    }

    pub fn prime_fetch(&mut self, address: u16) {
        self.cpu.regs.pc = address;
        self.cpu.data_in = self.read(address);
    }

    fn read(&mut self, address: u16) -> u8 {
        match address {
            0 => self.port_ddr,
            1 => self.effective_port(),
            _ => match self.resolve(address) {
                Resolved::Ram => self.ram[usize::from(address)],
                Resolved::Sid => self.sid.read((address & 0x1f) as u8),
                Resolved::Io => self.io[usize::from(address - 0xd000)],
                Resolved::Rom(kind) => {
                    self.needed_rom.get_or_insert(kind);
                    0
                }
            },
        }
    }

    fn write(&mut self, address: u16, value: u8) {
        self.ram[usize::from(address)] = value;
        match address {
            0 => self.port_ddr = value,
            1 => self.port_data = value,
            _ if self.resolve(address) == Resolved::Sid => {
                self.sid.write((address & 0x1f) as u8, value)
            }
            _ if self.resolve(address) == Resolved::Io => {
                self.io[usize::from(address - 0xd000)] = value;
            }
            _ => {}
        }
    }

    fn effective_port(&self) -> u8 {
        (self.port_data & self.port_ddr) | (!self.port_ddr & 0x17)
    }

    fn resolve(&self, address: u16) -> Resolved {
        let port = self.effective_port();
        let loram = port & 1 != 0;
        let hiram = port & 2 != 0;
        let charen = port & 4 != 0;
        match address {
            0xa000..=0xbfff if loram && hiram => Resolved::Rom(RomKind::Basic),
            0xd000..=0xdfff if (loram || hiram) && !charen => Resolved::Rom(RomKind::Chargen),
            0xd400..=0xd7ff if (loram || hiram) && charen => Resolved::Sid,
            0xd000..=0xdfff if (loram || hiram) && charen => Resolved::Io,
            0xe000..=0xffff if hiram => Resolved::Rom(RomKind::Kernal),
            _ => Resolved::Ram,
        }
    }
}

const fn bank_for_call(address: u16) -> u8 {
    if address < 0xa000 {
        0x37
    } else if address < 0xd000 {
        0x36
    } else if address < 0xe000 {
        0x34
    } else {
        0x35
    }
}
