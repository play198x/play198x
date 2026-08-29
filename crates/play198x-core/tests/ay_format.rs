#![cfg(feature = "ay")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use play198x_core::metadata::ay_meta;
use play198x_core::player::ay::format::{AyError, parse};
use play198x_core::probe::{Confidence, Format, identify};

mod common;

/// Builds a one-song .ay in code — no fixture files, per this repository's
/// rule. Layout per the table in the plan; every pointer is signed,
/// big-endian, and relative to its own position.
fn synthetic_ay() -> Vec<u8> {
    common::build_ay(0x8000, 0x8010, 0x8000, &[0xAA, 0xBB, 0xCC, 0xDD])
}

#[test]
fn parses_a_one_song_file() {
    let ay = parse(&synthetic_ay()).unwrap();
    assert_eq!(ay.player_version, 3);
    assert_eq!(ay.author, "Steve");
    assert_eq!(ay.misc, "notes");
    assert_eq!(ay.songs.len(), 1);

    let song = &ay.songs[0];
    assert_eq!(song.name, "Test Tune");
    assert_eq!(song.length_frames, 500);
    assert_eq!(song.fade_frames, 50);
    assert_eq!(song.hi_reg, common::FIXTURE_HI_REG);
    assert_eq!(song.lo_reg, common::FIXTURE_LO_REG);
    assert_eq!(song.stack, 0xC000);
    assert_eq!(song.init, 0x8000);
    assert_eq!(song.interrupt, 0x8010);
    assert_eq!(song.blocks.len(), 1);
    assert_eq!(song.blocks[0].address, 0x8000);
    assert_eq!(song.blocks[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
}

/// A whole `.ay` file written out byte by byte, with every offset stated —
/// the format's own layout, not this crate's builder's idea of it.
///
/// `common::build_ay` writes the same structure, and a test that only ever
/// round-trips through it can prove the builder and the parser agree while
/// both disagree with the format. This array is the independent statement of
/// what the bytes mean, so the two cannot drift together.
///
/// | Offset | Field |
/// |---|---|
/// | 0 | `ZXAYEMUL` |
/// | 8 | `FileVersion` = 0, `PlayerVersion` = 3 |
/// | 10 | `PSpecialPlayer` (unused) |
/// | 12 | `PAuthor` -> 24 |
/// | 14 | `PMisc` -> 26 |
/// | 16 | `NumOfSongs - 1` = 0, `FirstSong` = 0 |
/// | 18 | `PSongsStructure` -> 20 |
/// | 20 | song 0: `PName` -> 28, `PSongData` -> 30 |
/// | 24 | author `"A"`, misc `"M"`, name `"S"` |
/// | 30 | song data: `AChan`..`Noise`, `SongLength` 100, `FadeLength` 10 |
/// | 38 | **`HiReg` = 0xA5** (`data + 8`) |
/// | 39 | **`LoReg` = 0x3C** (`data + 9`) |
/// | 40 | `PPoints` -> 44, `PAddresses` -> 50 |
/// | 44 | `Stack` 0xC000, `Init` 0x8000, `Interrupt` 0x8003 |
/// | 50 | block: `Address` 0x8000, `Length` 2, `Offset` -> 58; terminator |
/// | 58 | the block's two bytes |
const LITERAL_AY: [u8; 60] = [
    0x5A, 0x58, 0x41, 0x59, 0x45, 0x4D, 0x55, 0x4C, // 0: ZXAYEMUL
    0x00, 0x03, // 8: FileVersion, PlayerVersion
    0x00, 0x00, // 10: PSpecialPlayer
    0x00, 0x0C, // 12: PAuthor -> 24
    0x00, 0x0C, // 14: PMisc -> 26
    0x00, 0x00, // 16: NumOfSongs - 1, FirstSong
    0x00, 0x02, // 18: PSongsStructure -> 20
    0x00, 0x08, // 20: PName -> 28
    0x00, 0x08, // 22: PSongData -> 30
    b'A', 0x00, // 24: author
    b'M', 0x00, // 26: misc
    b'S', 0x00, // 28: song name
    0x00, 0x01, 0x02, 0x03, // 30: AChan..Noise      (data + 0)
    0x00, 0x64, // 34: SongLength = 100              (data + 4)
    0x00, 0x0A, // 36: FadeLength = 10               (data + 6)
    0xA5, // 38: HiReg                               (data + 8)
    0x3C, // 39: LoReg                               (data + 9)
    0x00, 0x04, // 40: PPoints -> 44                 (data + 10)
    0x00, 0x08, // 42: PAddresses -> 50              (data + 12)
    0xC0, 0x00, // 44: Stack
    0x80, 0x00, // 46: Init
    0x80, 0x03, // 48: Interrupt
    0x80, 0x00, // 50: block Address
    0x00, 0x02, // 52: block Length
    0x00, 0x04, // 54: block Offset -> 58
    0x00, 0x00, // 56: address-block terminator
    0xC9, 0xC9, // 58: the block itself
];

/// `HiReg` is at `data + 8` and `LoReg` at `data + 9`, and the two are not
/// interchangeable: `HiReg` becomes `A`, and a multi-song file selects its
/// subtune by the number the format hands `init` in `A`, so reading these
/// two the wrong way round plays song 0 whatever was asked for.
///
/// Asserted against [`LITERAL_AY`] rather than against `common::build_ay`,
/// because a builder and a parser that share one misreading of the format
/// round-trip perfectly while both being wrong.
#[test]
fn the_register_halves_are_read_from_the_offsets_the_format_states() {
    assert_eq!(LITERAL_AY[38], 0xA5, "the fixture itself moved");
    assert_eq!(LITERAL_AY[39], 0x3C, "the fixture itself moved");

    let song = &parse(&LITERAL_AY).unwrap().songs[0];
    assert_eq!(
        song.hi_reg, 0xA5,
        "HiReg must come from data + 8, the byte that becomes A"
    );
    assert_eq!(song.lo_reg, 0x3C, "LoReg must come from data + 9");
}

/// The rest of [`LITERAL_AY`]'s fields, pinned the same independent way —
/// `HiReg`/`LoReg` are the pair easiest to get backwards, but every other
/// offset in the song data structure is reachable by the same mistake.
#[test]
fn a_literal_file_parses_field_for_field() {
    let file = parse(&LITERAL_AY).unwrap();
    assert_eq!(file.player_version, 3);
    assert_eq!(file.author, "A");
    assert_eq!(file.misc, "M");
    assert_eq!(file.songs.len(), 1);

    let song = &file.songs[0];
    assert_eq!(song.name, "S");
    assert_eq!(song.length_frames, 100);
    assert_eq!(song.fade_frames, 10);
    assert_eq!(song.stack, 0xC000);
    assert_eq!(song.init, 0x8000);
    assert_eq!(song.interrupt, 0x8003);
    assert_eq!(song.blocks.len(), 1);
    assert_eq!(song.blocks[0].address, 0x8000);
    assert_eq!(song.blocks[0].data, vec![0xC9, 0xC9]);
}

/// Each song in a multi-song file carries its own register halves, and the
/// parser must keep them with the song they belong to. A file whose songs
/// all report song 0's registers plays song 0's music whichever subtune is
/// selected, which is silent and looks like success.
#[test]
fn every_song_keeps_its_own_register_halves() {
    let bytes = common::build_ay_songs(
        &[(0x00, 0xF0), (0x01, 0xE1), (0x02, 0xD2)],
        0x8000,
        0x8000,
        0x8000,
        &[0xC9],
    );
    let file = parse(&bytes).unwrap();
    assert_eq!(file.songs.len(), 3);
    let halves: Vec<(u8, u8)> = file.songs.iter().map(|s| (s.hi_reg, s.lo_reg)).collect();
    assert_eq!(halves, vec![(0x00, 0xF0), (0x01, 0xE1), (0x02, 0xD2)]);
}

#[test]
fn rejects_bytes_that_are_not_an_ay_file() {
    assert!(matches!(parse(&[0u8; 64]), Err(AyError::NotAnAyFile)));
    assert!(matches!(parse(b"ZXAYAMAD"), Err(AyError::NotAnAyFile)));
}

/// A pointer that walks off the end must be an error, not a panic: these
/// files come from strangers.
#[test]
fn a_pointer_past_the_end_is_an_error_not_a_panic() {
    let mut bytes = synthetic_ay();
    let n = bytes.len();
    bytes[18..20].copy_from_slice(&(i16::MAX).to_be_bytes());
    assert!(
        parse(&bytes).is_err(),
        "expected an error for a file of {n} bytes"
    );
}

/// `.ay` identification needs no `ay` feature (see `Format::Ay`'s doc).
/// `tests/probe.rs`'s `an_ay_file_is_identified_by_its_header_alone_with_certainty`
/// pins that fact against a bare eight-byte magic and nothing else, in a
/// file that carries no `ay` feature gate at all. This test pins the other
/// half: a *real*, fully structured `.ay` file — the fixture every other
/// test in this file also parses — still identifies the same way. Together
/// they cover "the magic alone is enough" and "a genuine file still carries
/// it", rather than one test standing in for both.
#[test]
fn an_ay_file_probes_as_certain() {
    let (format, confidence) = identify(&synthetic_ay()).unwrap();
    assert_eq!(format, Format::Ay);
    assert_eq!(confidence, Confidence::Certain);
}

#[test]
fn ay_metadata_reports_the_song_names() {
    let file = parse(&synthetic_ay()).unwrap();
    let meta = ay_meta(&file);

    assert_eq!(meta.author, "Steve");
    assert_eq!(meta.misc, "notes");
    assert_eq!(meta.songs, vec!["Test Tune".to_string()]);
    // `.ay` has no file-level title; song 0's name and length stand in for
    // one, per `AyMeta`'s doc.
    assert_eq!(meta.title, "Test Tune");
    assert_eq!(meta.length_frames, 500);
}
