/// The 128K Spectrum's memory, as far as an `.ay` tune can tell: eight
/// 16 KB RAM banks paged through one 64 KB address space, minus the ROM.
///
/// The map is the 128K's own — `$4000-$7FFF` is always bank 5, `$8000-$BFFF`
/// always bank 2, and `$C000-$FFFF` whichever bank port `$7FFD` last
/// selected (`reference/by-system/sinclair-zx-spectrum/128k-memory-paging.md`,
/// § Memory Map). Banks 5 and 2 are therefore reachable at two addresses at
/// once, which is why they cannot be separate arrays from the paged ones: a
/// write to `$C000` with bank 5 selected has to show up at `$4000`.
///
/// `$0000-$3FFF` is where a real 128K pages one of its ROMs. This host ships
/// no ROM — the `.ay` format does not want one, and its player fills
/// `$0000-$00FF` with `RET` instead — so the region is ordinary RAM here and
/// stays that way whatever the ROM-select bit says. See [`Memory::page`].
pub struct Memory {
    /// Nine 16 KB regions in one allocation: `$0000-$3FFF` first, then RAM
    /// banks 0 through 7. One allocation rather than nine so that
    /// [`Memory::offset`] can turn an address into an index with
    /// arithmetic, and `Box<[u8]>` rather than `Box<[u8; N]>` so
    /// construction never builds 144 KB on the stack on its way to the
    /// heap.
    cells: Box<[u8]>,
    /// Which RAM bank `$C000-$FFFF` currently reads and writes. Always
    /// 0-7: [`Memory::page`] masks it, so [`Memory::offset`] cannot compute
    /// an index outside `cells`.
    paged: usize,
    /// Set by bit 5 of a `$7FFD` write, after which no further write moves
    /// the mapping. See [`Memory::page`].
    paging_locked: bool,
}

/// One page of the Z80's address space, and one RAM bank: 16 KB.
const BANK_LEN: usize = 0x4000;

/// RAM banks in a 128K Spectrum.
const BANK_COUNT: usize = 8;

/// Region index of `$0000-$3FFF` within `cells`. The RAM banks follow it,
/// so bank `n` is region `LOW_REGION + 1 + n`.
const LOW_REGION: usize = 0;

/// Fixed at `$4000-$7FFF` on every 128K machine; also holds the screen.
const BANK_AT_4000: usize = 5;

/// Fixed at `$8000-$BFFF` on every 128K machine.
const BANK_AT_8000: usize = 2;

/// The banks that can only ever be seen through the window at
/// `$C000-$FFFF` — every bank but the two with fixed addresses of their own.
/// See [`Memory::mirror_window_into_the_pageable_banks`].
const PAGEABLE_ONLY_BANKS: [usize; 6] = [0, 1, 3, 4, 6, 7];

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}

impl Memory {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cells: vec![0u8; BANK_LEN * (BANK_COUNT + 1)].into_boxed_slice(),
            // The power-on mapping, and the one a 48K-era tune assumes:
            // bank 0 at $C000, paging unlocked.
            paged: 0,
            paging_locked: false,
        }
    }

    /// Copies `data` in at `address` through the mapping currently in force,
    /// stopping at the top of memory rather than wrapping: a block that
    /// overruns is the file's problem, and wrapping would corrupt the bottom
    /// of RAM invisibly.
    ///
    /// Through the mapping, not into a bank: an `.ay` block carries an
    /// address and nothing else, so the file cannot say which bank it means.
    /// The tune's blocks therefore land wherever the machine was pointing
    /// when it was switched on — which is what a real 128K loading the same
    /// file would do too.
    pub fn load(&mut self, address: u16, data: &[u8]) {
        for (step, byte) in data.iter().enumerate() {
            let Ok(step) = u16::try_from(step) else {
                return;
            };
            let Some(target) = address.checked_add(step) else {
                return;
            };
            self.write(target, *byte);
        }
    }

    #[must_use]
    pub fn read(&self, address: u16) -> u8 {
        self.cells[self.offset(address)]
    }

    pub fn write(&mut self, address: u16, value: u8) {
        let offset = self.offset(address);
        self.cells[offset] = value;
    }

    /// Applies a write to port `$7FFD`, the 128K paging port.
    ///
    /// Bit layout, from
    /// `reference/by-system/sinclair-zx-spectrum/128k-memory-paging.md`
    /// § Port 0x7FFD: bits 0-2 select the RAM bank at `$C000-$FFFF`, bit 3
    /// selects which bank the display is drawn from, bit 4 is the ROM
    /// select, bit 5 disables further paging, and bits 6-7 are unused.
    ///
    /// Two of those do nothing here, and both silences are deliberate:
    ///
    /// - **Bit 3, the display bank.** This host has no display. Nothing
    ///   reads bank 5 or bank 7 as pixels, so which one a tune nominates
    ///   changes nothing it can observe.
    /// - **Bit 4, the ROM select.** This host has no ROM. `$0000-$3FFF` is
    ///   RAM holding the `RET` stub the `.ay` format's player is required to
    ///   supply, and swapping a ROM in over it would take that stub away —
    ///   every tune that calls into low memory would then execute whatever
    ///   the file happened to leave there. So the region is left alone. This
    ///   is not a shortcut: the bit is written by tunes that mean "give me
    ///   the 48K BASIC ROM", and the honest answer from a machine with no
    ///   ROMs at all is the RAM that is already there.
    ///
    /// Bit 5 is honoured. On real hardware the lock holds until a hard
    /// reset, with no software route back; here the equivalent of a reset is
    /// constructing a new [`Memory`], which `AyPlayer::new` does once per
    /// song. So a locked-out tune stays locked out for exactly as long as it
    /// would on the machine — the whole of the run it locked during — and
    /// the next song starts from a cold machine, which is the only thing
    /// "until reset" can mean where nothing is ever reset.
    pub fn page(&mut self, value: u8) {
        if self.paging_locked {
            return;
        }
        self.paged = (value & 0x07) as usize;
        self.paging_locked = value & 0x20 != 0;
    }

    /// Copies whatever the window at `$C000-$FFFF` was loaded with into
    /// every other bank that can appear there. Call once, after an `.ay`
    /// file's blocks are in and before its code runs.
    ///
    /// An `.ay` block carries an address and a length, never a bank. So the
    /// file's statement about `$C000-$FFFF` is a statement about the
    /// *window*, and a host that put those bytes in bank 0 alone would be
    /// adding a claim the file never made — that every other bank is empty.
    /// Giving each bank the same starting image adds nothing: whichever one
    /// a tune selects, it finds the bytes the file put at those addresses,
    /// exactly as it would on the flat 64 KB machine the format was written
    /// for. What the banks then do differently is diverge, because a write
    /// lands in the selected bank and stays there.
    ///
    /// The corpus says this is not a hypothetical. Of the 696 files in the
    /// World of Spectrum AY archive, one — Wizball, across all 17 of its
    /// subtunes — pages the window, selecting bank 1; and its single code
    /// block runs from `$BA91` to `$D970`, six kilobytes of which sit in the
    /// window. Loading that into bank 0 alone would have the tune page its
    /// own data out on its first `OUT`.
    ///
    /// Banks 2 and 5 are left alone: they have fixed addresses of their own
    /// (`$8000` and `$4000`), which the file addresses separately, and
    /// overwriting either with the window's image would throw away what the
    /// file put there.
    pub fn mirror_window_into_the_pageable_banks(&mut self) {
        let source = (LOW_REGION + 1 + self.paged) * BANK_LEN;
        for bank in PAGEABLE_ONLY_BANKS {
            if bank == self.paged {
                continue;
            }
            let target = (LOW_REGION + 1 + bank) * BANK_LEN;
            self.cells.copy_within(source..source + BANK_LEN, target);
        }
    }

    /// Which RAM bank is currently mapped at `$C000-$FFFF`.
    #[must_use]
    pub fn paged_bank(&self) -> u8 {
        // Always 0-7 by construction; see the field's doc.
        self.paged as u8
    }

    /// Whether a `$7FFD` write has locked the mapping. See [`Memory::page`].
    #[must_use]
    pub fn paging_locked(&self) -> bool {
        self.paging_locked
    }

    /// Where `address` lands in `cells` under the current mapping.
    ///
    /// The result is always in bounds: the region index is at most
    /// `BANK_COUNT`, the offset within a region at most `BANK_LEN - 1`, and
    /// `cells` is `BANK_LEN * (BANK_COUNT + 1)` bytes long. That matters
    /// because every byte of an `.ay` file can steer this, and an index
    /// computed from a stranger's bytes must not be able to panic.
    fn offset(&self, address: u16) -> usize {
        let region = match address >> 14 {
            0 => LOW_REGION,
            1 => LOW_REGION + 1 + BANK_AT_4000,
            2 => LOW_REGION + 1 + BANK_AT_8000,
            _ => LOW_REGION + 1 + self.paged,
        };
        region * BANK_LEN + (address as usize & (BANK_LEN - 1))
    }
}
