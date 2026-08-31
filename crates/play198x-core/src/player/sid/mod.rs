//! ROM-free playback for callable PSID tunes.

pub mod format;

use crate::host::c64::C64Host;
use crate::player::pump::FrameSource;
use format::{Clock, Kind, SidError, SidFile, Speed};

const PAL_CLOCK: u64 = 985_248;
const NTSC_CLOCK: u64 = 1_022_727;
// Kept in always-RAM low memory. RTS performs a dummy read at return_address
// before incrementing PC; putting the sentinel under KERNAL would make that
// CPU bus cycle look like a tune's ROM dependency.
const SENTINEL: u16 = 0x02ff;
const INIT_BUDGET: u32 = 4 * 20_000;
const PLAY_BUDGET: u32 = 20_000;

pub struct SidPlayer {
    pub file: SidFile,
    host: C64Host,
    song: usize,
    cycles_per_frame: u32,
    samples: Vec<f32>,
    frame_samples: usize,
    last_error: Option<SidError>,
}

impl SidPlayer {
    pub fn new(bytes: &[u8], song: usize, sample_rate: u32) -> Result<Self, SidError> {
        let file = format::parse(bytes)?;
        if file.kind == Kind::Rsid {
            return Err(SidError::RsidNotSupported);
        }
        if file.play_address == 0 {
            return Err(SidError::SelfDrivenNotSupported);
        }
        if file.mus_player {
            return Err(SidError::UnsupportedFeature(
                "Compute!'s Sidplayer MUS data",
            ));
        }
        if file.second_sid_address != 0 || file.third_sid_address != 0 {
            return Err(SidError::UnsupportedFeature("multi-SID hardware"));
        }
        if song >= usize::from(file.songs) {
            return Err(SidError::NoSuchSong);
        }
        let clock = if file.clock == Clock::Ntsc {
            NTSC_CLOCK
        } else {
            PAL_CLOCK
        };
        let hz = match file.speed(song) {
            Speed::Cia => 60,
            Speed::Vbi if file.clock == Clock::Ntsc => 60,
            Speed::Vbi => 50,
        };
        let mut host = C64Host::new(clock, sample_rate.max(1), file.model);
        host.set_cia_timer_a(if file.clock == Clock::Ntsc {
            0x4295
        } else {
            0x4025
        });
        host.load(file.load_address, &file.data);
        // The PSID environment byte: zero NTSC, one PAL.
        host.load(0x02a6, &[u8::from(file.clock != Clock::Ntsc)]);
        let mut player = Self {
            file,
            host,
            song,
            cycles_per_frame: (clock / hz) as u32,
            samples: Vec::with_capacity(sample_rate.div_ceil(hz as u32) as usize),
            frame_samples: 0,
            last_error: None,
        };
        let init = player.file.init_address;
        player.host.cpu.regs.a = song as u8;
        if !player.call(init, INIT_BUDGET) {
            return Err(player.failure(SidError::InitDidNotReturn));
        }
        player.host.sid.drain_buffer_into(&mut player.samples);
        player.samples.clear();
        Ok(player)
    }

    pub fn frame(&mut self) -> Result<(), SidError> {
        if let Some(error) = self.last_error.clone() {
            return Err(error);
        }
        let start = self.host.cpu.total_cycles;
        let address = self.file.play_address;
        if !self.call(address, PLAY_BUDGET) {
            return Err(self.failure(SidError::PlayDidNotReturn));
        }
        while self.host.cpu.total_cycles.wrapping_sub(start) < u64::from(self.cycles_per_frame) {
            self.host.sid.tick();
            self.host.cpu.total_cycles = self.host.cpu.total_cycles.wrapping_add(1);
        }
        self.host.sid.drain_buffer_into(&mut self.samples);
        self.frame_samples = self.samples.len();
        Ok(())
    }

    #[must_use]
    pub fn last_error(&self) -> Option<&SidError> {
        self.last_error.as_ref()
    }

    fn failure(&self, fallback: SidError) -> SidError {
        self.host.needed_rom.map_or(fallback, SidError::NeedsRom)
    }

    fn call(&mut self, address: u16, budget: u32) -> bool {
        self.host.prepare_call(address);
        self.host.cpu.halted = false;
        let saved_sp = self.host.cpu.regs.sp;
        let pushed_sp = saved_sp.wrapping_sub(2);
        self.host.cpu.regs.sp = pushed_sp;
        let return_address = SENTINEL.wrapping_sub(1).to_le_bytes();
        self.host.poke(
            0x0100 | u16::from(pushed_sp.wrapping_add(1)),
            return_address[0],
        );
        self.host.poke(
            0x0100 | u16::from(pushed_sp.wrapping_add(2)),
            return_address[1],
        );
        self.host.prime_fetch(address);
        for _ in 0..budget {
            let rom_before = self.host.needed_rom;
            self.host.step();
            if self.host.cpu.instruction_complete() && self.host.cpu.regs.pc == SENTINEL {
                // The core has already presented the following opcode fetch.
                // That is our host-only return sentinel, not tune activity.
                self.host.needed_rom = rom_before;
                return true;
            }
            if self.host.needed_rom.is_some() {
                self.host.cpu.regs.sp = saved_sp;
                return false;
            }
        }
        self.host.cpu.regs.sp = saved_sp;
        false
    }
}

impl FrameSource for SidPlayer {
    fn frame(&mut self) {
        if let Err(error) = SidPlayer::frame(self) {
            self.last_error = Some(error);
        }
    }
    fn render_frame(&mut self, out: &mut [f32]) -> usize {
        let frames = self.frame_samples.min(out.len() / 2);
        for (slot, &sample) in out
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(self.samples.iter())
            .take(frames)
        {
            let sample = sample.clamp(-1.0, 1.0);
            slot[0] = sample;
            slot[1] = sample;
        }
        frames
    }
    fn samples_per_frame(&self) -> usize {
        self.frame_samples.max(1)
    }
    fn song(&self) -> usize {
        self.song
    }
}
