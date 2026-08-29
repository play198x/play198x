//! The `.ay` container: Z80 code plus the addresses to call, not sample data.
//!
//! Every multi-byte field is big-endian, and every pointer is **signed and
//! relative to its own position in the file** — the target is the pointer's
//! own offset plus its value. Verified against real files from the World of
//! Spectrum archive rather than from memory; an early draft of this parser
//! had `LoReg` and `HiReg` the other way round. `follow` also documents a
//! narrow, evidence-backed exception to "signed": see its own doc.

/// Why a file could not be read. Every variant is reachable from bytes a
/// stranger supplied.
#[derive(Debug, PartialEq, Eq)]
pub enum AyError {
    NotAnAyFile,
    Truncated,
    /// A pointer that resolves outside the file.
    BadPointer,
    /// A song index the file does not have.
    NoSuchSong,
    /// The tune's init routine never returned inside its cycle budget.
    InitDidNotReturn,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub address: u16,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Song {
    pub name: String,
    pub length_frames: u16,
    pub fade_frames: u16,
    pub hi_reg: u8,
    pub lo_reg: u8,
    pub stack: u16,
    pub init: u16,
    pub interrupt: u16,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AyFile {
    pub player_version: u8,
    pub author: String,
    pub misc: String,
    pub songs: Vec<Song>,
}

fn be16(bytes: &[u8], at: usize) -> Result<u16, AyError> {
    let pair = bytes.get(at..at + 2).ok_or(AyError::Truncated)?;
    Ok(u16::from_be_bytes([pair[0], pair[1]]))
}

/// Resolves a relative pointer stored at `at`: the signed reading the
/// format specifies, falling back to an unsigned reading of the same bits
/// only when the signed one fails.
///
/// # Why signed first
///
/// The AY format's own documentation (vgmrips' "AY File Format" page,
/// itself transcribing Sergey Bulba's original spec with Patrik Rak's
/// help) is explicit about the encoding: "all pointers are signed and
/// relative", and each pointer field is declared `smallint` — Pascal for
/// a signed 16-bit integer. That is what this function reads first, and
/// it is the only reading most files ever need.
///
/// # Why a fallback exists, and why it is safe
///
/// Two files out of the 696 in the corpus Task 8 swept
/// (`games/r/RobinOfTheWood.ay.zip`, `demos/s/SpecialMusicCollection.ay.zip`)
/// store a block offset whose *signed* reading resolves outside the file —
/// `RobinOfTheWood.ay`'s song 15, block 0 stores `0xC5BD` (50,621) at file
/// position 838, which signed is -14,077 relative to its own position,
/// negative and so out of range. The format's documentation says pointers
/// are signed; it does not say what a reader should do when that reading
/// fails, so this is not a question the spec answers either way.
///
/// What settles it is the corpus: read as an *unsigned* distance instead,
/// `RobinOfTheWood.ay`'s same bits resolve to file position 51,459, and
/// that position plus the block's own 2,126-byte length lands at exactly
/// 53,585 — this file's length to the byte. `SpecialMusicCollection.ay` is
/// stronger evidence again: its failing pointer (song 7, block 1, field
/// `0x9461` at position 506) reads unsigned as position 38,491, ending
/// (with its 3,775-byte length) at 42,266 — which is exactly where song
/// 8's block 1 begins under the *same* unsigned reading, and that block's
/// unsigned end lands at 46,216, this file's length, again to the byte.
/// Two independently-computed pointers, in two different songs, landing
/// contiguous with each other and flush with end-of-file is not a
/// coincidence a corrupt file produces by chance.
///
/// A pointer whose signed target already resolves inside the file is
/// never affected by this: the fallback below only runs after the signed
/// reading has already failed, so every pointer that works today —
/// including this format's real backward pointers, which the signed
/// reading exists to serve — takes exactly the path it always has and
/// lands in exactly the same place. The only pointers the fallback can
/// change are ones this function would otherwise refuse outright.
///
/// This is a heuristic, not a spec rule, and it has a real cost: a
/// genuinely corrupt file could, by chance, carry a signed-invalid
/// pointer whose unsigned reading happens to land inside the file too,
/// and this function would return that address instead of refusing the
/// file — trading a clean [`AyError::BadPointer`] for a block built from
/// the wrong bytes. Accepted because the failure mode stays contained
/// (the returned index is always in-bounds; nothing is read out of the
/// file, and a tune built from the wrong bytes is exactly the shape of
/// thing this crate's own tests and the corpus sweep's peak/audible
/// metrics are built to notice, not a silent or unsafe outcome) and
/// because the corpus gave two real files, not zero, as evidence this
/// case is a genuine forward offset outside the signed range rather than
/// corruption. See `task-8-report.md` (fix round 1) for the corpus-wide
/// check confirming every file that parsed before this change still
/// parses to the same songs afterwards.
fn follow(bytes: &[u8], at: usize) -> Result<usize, AyError> {
    let raw = be16(bytes, at)?;
    let signed_target = at as i64 + (raw as i16) as i64;
    if signed_target >= 0 && (signed_target as usize) < bytes.len() {
        return Ok(signed_target as usize);
    }
    let unsigned_target = at as u64 + raw as u64;
    if unsigned_target < bytes.len() as u64 {
        return Ok(unsigned_target as usize);
    }
    Err(AyError::BadPointer)
}

/// A NUL-terminated Latin-1 string. Amiga and Spectrum text is Latin-1, not
/// UTF-8; reading it as UTF-8 mangles accents and box-drawing bytes.
fn nt_string(bytes: &[u8], at: usize) -> String {
    bytes[at..]
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

/// Reads an `.ay` file's structure. Loads no code and runs nothing.
///
/// # Errors
///
/// When the magic is wrong, the file ends inside a structure, or a pointer
/// resolves outside it.
pub fn parse(bytes: &[u8]) -> Result<AyFile, AyError> {
    if bytes.len() < 20 || &bytes[0..4] != b"ZXAY" || &bytes[4..8] != b"EMUL" {
        return Err(AyError::NotAnAyFile);
    }

    let player_version = bytes[9];
    let author = nt_string(bytes, follow(bytes, 12)?);
    let misc = nt_string(bytes, follow(bytes, 14)?);
    let count = bytes[16] as usize + 1;
    let songs_at = follow(bytes, 18)?;

    let mut songs = Vec::with_capacity(count);
    for i in 0..count {
        let entry = songs_at + i * 4;
        let name = nt_string(bytes, follow(bytes, entry)?);
        let data = follow(bytes, entry + 2)?;

        let points = follow(bytes, data + 10)?;
        let addresses = follow(bytes, data + 12)?;

        let mut blocks = Vec::new();
        let mut at = addresses;
        loop {
            let address = be16(bytes, at)?;
            if address == 0 {
                break;
            }
            let length = be16(bytes, at + 2)? as usize;
            let start = follow(bytes, at + 4)?;
            // A block that overruns the file is truncated rather than
            // rejected: the tune may never read the tail, and refusing the
            // whole file would lose one that plays.
            let end = start.saturating_add(length).min(bytes.len());
            blocks.push(Block {
                address,
                data: bytes[start..end].to_vec(),
            });
            at += 6;
        }

        songs.push(Song {
            name,
            length_frames: be16(bytes, data + 4)?,
            fade_frames: be16(bytes, data + 6)?,
            lo_reg: *bytes.get(data + 8).ok_or(AyError::Truncated)?,
            hi_reg: *bytes.get(data + 9).ok_or(AyError::Truncated)?,
            stack: be16(bytes, points)?,
            init: be16(bytes, points + 2)?,
            interrupt: be16(bytes, points + 4)?,
            blocks,
        });
    }

    Ok(AyFile {
        player_version,
        author,
        misc,
        songs,
    })
}
