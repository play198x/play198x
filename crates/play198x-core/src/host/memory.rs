/// 64KB of RAM and nothing else. No ROM is mapped, because none is shipped
/// and the `.ay` format does not want one — its player is a stub.
pub struct Memory {
    cells: Box<[u8; 0x10000]>,
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cells: Box::new([0u8; 0x10000]),
        }
    }

    /// Copies `data` in at `address`, stopping at the top of memory rather
    /// than wrapping: a block that overruns is the file's problem, and
    /// wrapping would corrupt the bottom of RAM invisibly.
    pub fn load(&mut self, address: u16, data: &[u8]) {
        let start = address as usize;
        let end = (start + data.len()).min(0x10000);
        self.cells[start..end].copy_from_slice(&data[..end - start]);
    }

    #[must_use]
    pub fn read(&self, address: u16) -> u8 {
        self.cells[address as usize]
    }

    pub fn write(&mut self, address: u16, value: u8) {
        self.cells[address as usize] = value;
    }
}
