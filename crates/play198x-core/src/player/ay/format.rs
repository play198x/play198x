//! The `.ay` container: Z80 code plus the addresses to call, not sample data.
//!
//! Every multi-byte field is big-endian, and every pointer is **signed and
//! relative to its own position in the file** — the target is the pointer's
//! own offset plus its value. Verified against real files from the World of
//! Spectrum archive rather than from memory; an early draft of this parser
//! had `LoReg` and `HiReg` the other way round.

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

/// Resolves a signed relative pointer stored at `at`.
fn follow(bytes: &[u8], at: usize) -> Result<usize, AyError> {
    let delta = be16(bytes, at)? as i16;
    let target = at as i64 + delta as i64;
    if target < 0 || target as usize >= bytes.len() {
        return Err(AyError::BadPointer);
    }
    Ok(target as usize)
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
