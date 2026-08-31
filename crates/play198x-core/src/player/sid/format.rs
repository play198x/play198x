//! PSID/RSID container parsing. Playback policy lives one level up.

use emu198x_mos_sid_6581::SidModel;

const V1_HEADER_LEN: usize = 0x76;
const V2_HEADER_LEN: usize = 0x7c;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Psid,
    Rsid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clock {
    Unknown,
    Pal,
    Ntsc,
    Either,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speed {
    Vbi,
    Cia,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidFile {
    pub kind: Kind,
    pub version: u16,
    pub load_address: u16,
    pub init_address: u16,
    pub play_address: u16,
    pub songs: u16,
    pub start_song: u16,
    pub speed_bits: u32,
    pub title: String,
    pub author: String,
    pub released: String,
    pub clock: Clock,
    pub model: SidModel,
    pub mus_player: bool,
    pub second_sid_address: u8,
    pub third_sid_address: u8,
    pub data: Vec<u8>,
}

impl SidFile {
    #[must_use]
    pub fn speed(&self, song: usize) -> Speed {
        let bit = song.min(31);
        if self.speed_bits & (1 << bit) == 0 {
            Speed::Vbi
        } else {
            Speed::Cia
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SidError {
    NotSid,
    Truncated,
    UnsupportedVersion(u16),
    InvalidHeader(&'static str),
    RsidNotSupported,
    SelfDrivenNotSupported,
    UnsupportedFeature(&'static str),
    NoSuchSong,
    AddressOverflow,
    InitDidNotReturn,
    PlayDidNotReturn,
    NeedsRom(crate::host::c64::RomKind),
}

impl std::fmt::Display for SidError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSid => f.write_str("not a PSID or RSID file"),
            Self::Truncated => f.write_str("truncated SID header or data"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported SID version {v}"),
            Self::InvalidHeader(what) => write!(f, "invalid SID header: {what}"),
            Self::RsidNotSupported => f.write_str("RSID needs a C64 environment and is not supported by the ROM-free player"),
            Self::SelfDrivenNotSupported => f.write_str("a zero play address is self-driven and is not supported by the callable PSID player"),
            Self::UnsupportedFeature(what) => write!(f, "this SID file needs unsupported {what}"),
            Self::NoSuchSong => f.write_str("the SID file has no such subtune"),
            Self::AddressOverflow => f.write_str("the SID payload does not fit in 64 KiB of C64 memory"),
            Self::InitDidNotReturn => f.write_str("the SID init routine did not return inside its cycle budget"),
            Self::PlayDidNotReturn => f.write_str("the SID play routine did not return inside its cycle budget"),
            Self::NeedsRom(kind) => write!(f, "this tune needs the C64 {} ROM, which Play198x does not ship", kind.label()),
        }
    }
}

impl std::error::Error for SidError {}

pub fn parse(bytes: &[u8]) -> Result<SidFile, SidError> {
    if bytes.len() < V1_HEADER_LEN {
        return Err(SidError::Truncated);
    }
    let kind = match &bytes[0..4] {
        b"PSID" => Kind::Psid,
        b"RSID" => Kind::Rsid,
        _ => return Err(SidError::NotSid),
    };
    let version = be16(bytes, 4)?;
    if !(1..=4).contains(&version) || (kind == Kind::Rsid && version == 1) {
        return Err(SidError::UnsupportedVersion(version));
    }
    let expected = if version == 1 {
        V1_HEADER_LEN
    } else {
        V2_HEADER_LEN
    };
    if bytes.len() < expected {
        return Err(SidError::Truncated);
    }
    let data_offset = usize::from(be16(bytes, 6)?);
    if data_offset != expected || data_offset > bytes.len() {
        return Err(SidError::InvalidHeader(
            "data offset does not match the version",
        ));
    }
    let stored_load = be16(bytes, 8)?;
    let mut body = &bytes[data_offset..];
    let load_address = if stored_load == 0 {
        if body.len() < 2 {
            return Err(SidError::Truncated);
        }
        let address = u16::from_le_bytes([body[0], body[1]]);
        body = &body[2..];
        address
    } else {
        stored_load
    };
    let songs = be16(bytes, 0x0e)?;
    let start_song = be16(bytes, 0x10)?;
    if songs == 0 || songs > 256 {
        return Err(SidError::InvalidHeader("songs must be 1..=256"));
    }
    if start_song == 0 || start_song > songs {
        return Err(SidError::InvalidHeader(
            "start song is outside the song table",
        ));
    }
    if usize::from(load_address)
        .checked_add(body.len())
        .is_none_or(|end| end > 0x1_0000)
    {
        return Err(SidError::AddressOverflow);
    }
    let flags = if version >= 2 { be16(bytes, 0x76)? } else { 0 };
    let init = be16(bytes, 0x0a)?;
    Ok(SidFile {
        kind,
        version,
        load_address,
        init_address: if init == 0 { load_address } else { init },
        play_address: be16(bytes, 0x0c)?,
        songs,
        start_song,
        speed_bits: be32(bytes, 0x12)?,
        title: text(&bytes[0x16..0x36]),
        author: text(&bytes[0x36..0x56]),
        released: text(&bytes[0x56..0x76]),
        clock: match (flags >> 2) & 3 {
            1 => Clock::Pal,
            2 => Clock::Ntsc,
            3 => Clock::Either,
            _ => Clock::Unknown,
        },
        model: if (flags >> 4) & 3 == 2 {
            SidModel::Mos8580
        } else {
            SidModel::Mos6581
        },
        mus_player: flags & 1 != 0,
        second_sid_address: if version >= 3 { bytes[0x7a] } else { 0 },
        third_sid_address: if version >= 4 { bytes[0x7b] } else { 0 },
        data: body.to_vec(),
    })
}

fn be16(bytes: &[u8], at: usize) -> Result<u16, SidError> {
    let pair = bytes.get(at..at + 2).ok_or(SidError::Truncated)?;
    Ok(u16::from_be_bytes([pair[0], pair[1]]))
}
fn be32(bytes: &[u8], at: usize) -> Result<u32, SidError> {
    let quad = bytes.get(at..at + 4).ok_or(SidError::Truncated)?;
    Ok(u32::from_be_bytes([quad[0], quad[1], quad[2], quad[3]]))
}
fn text(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| windows_1252(b))
        .collect::<String>()
        .trim_end()
        .to_owned()
}

fn windows_1252(byte: u8) -> char {
    const C1: [char; 32] = [
        '€', '\u{0081}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{008d}', 'Ž',
        '\u{008f}', '\u{0090}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
        '\u{009d}', 'ž', 'Ÿ',
    ];
    if (0x80..=0x9f).contains(&byte) {
        C1[usize::from(byte - 0x80)]
    } else {
        char::from(byte)
    }
}
