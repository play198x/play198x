#![cfg(feature = "ay")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
use play198x_core::player::ay::format::{
    AyError, MAX_BLOCK_BYTES, MAX_BLOCKS, MAX_STRING_BYTES, MAX_STRING_LEN, parse,
};
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

/// Builds a deliberately hostile `.ay`: every song points at one shared data
/// structure whose address list repeats one block record, and every one of
/// those records copies the whole file.
///
/// Every string pointer is aimed at one shared region too, `text_len`
/// non-NUL bytes long, so the same file can amplify through the text path
/// with no blocks at all.
///
/// This is the amplifying shape a stranger's file can take, written out so
/// the caps have something real to refuse. `songs` is capped at 256 by the
/// format's one-byte count; `blocks`, `block_len` and `text_len` are what a
/// hostile file varies to multiply a few kilobytes into gigabytes.
fn amplifying_ay(
    songs: usize,
    blocks: usize,
    block_len: u16,
    padding: usize,
    text_len: usize,
) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"ZXAYEMUL");
    b.push(0); // FileVersion
    b.push(3); // PlayerVersion
    b.extend_from_slice(&0i16.to_be_bytes()); // PSpecialPlayer
    let author_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes()); // PAuthor
    let misc_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes()); // PMisc
    b.push((songs - 1) as u8);
    b.push(0); // FirstSong
    let songs_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes()); // PSongsStructure

    // The whole song table: two pointers per song, every pair aimed at the
    // same name and the same data structure.
    let songs_at = b.len();
    let mut entry_ptr_at = Vec::new();
    for _ in 0..songs {
        entry_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes()); // PName
        entry_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes()); // PSongData
    }

    // One NUL-terminated string, shared by author, misc and every song name.
    let text_at = b.len();
    b.resize(b.len() + text_len, b'x');
    b.push(0);

    let data_at = b.len();
    b.extend_from_slice(&[0, 1, 2, 3]);
    b.extend_from_slice(&500u16.to_be_bytes());
    b.extend_from_slice(&50u16.to_be_bytes());
    b.push(0x22); // HiReg
    b.push(0x11); // LoReg
    let points_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes());
    let addrs_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes());

    let points_at = b.len();
    b.extend_from_slice(&0xC000u16.to_be_bytes());
    b.extend_from_slice(&0x8000u16.to_be_bytes());
    b.extend_from_slice(&0x8000u16.to_be_bytes());

    let addrs_at = b.len();
    let mut block_ptr_at = Vec::new();
    for _ in 0..blocks {
        b.extend_from_slice(&0x8000u16.to_be_bytes()); // Address
        b.extend_from_slice(&block_len.to_be_bytes()); // Length
        block_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes()); // Offset, patched to 0
    }
    b.extend_from_slice(&0u16.to_be_bytes()); // terminator

    // Padding is the file's own body: every block points at offset 0 and
    // copies `block_len` bytes from there, so the file's length is what each
    // block actually costs.
    b.resize(b.len() + padding, 0);

    let patch = |b: &mut Vec<u8>, at: usize, target: usize| {
        let delta = (target as i32 - at as i32) as i16;
        b[at..at + 2].copy_from_slice(&delta.to_be_bytes());
    };
    patch(&mut b, author_ptr_at, text_at);
    patch(&mut b, misc_ptr_at, text_at);
    patch(&mut b, songs_ptr_at, songs_at);
    for (index, at) in entry_ptr_at.iter().enumerate() {
        patch(&mut b, *at, if index % 2 == 0 { text_at } else { data_at });
    }
    patch(&mut b, points_ptr_at, points_at);
    patch(&mut b, addrs_ptr_at, addrs_at);
    for at in block_ptr_at {
        patch(&mut b, at, 0);
    }
    b
}

/// A small file must not turn into a large allocation.
///
/// `.ay` files come from strangers — the public site is a page you drop one
/// onto — and the block loop copies a caller-declared length once per block
/// record per song. Both multipliers are attacker-chosen, so the growth is
/// quadratic in file length: this shape reached 3.87 GB from 10,066 bytes
/// before [`MAX_BLOCK_BYTES`] existed, a 384,000x amplification in 0.64
/// seconds. Every other untrusted path in this crate is capped (see
/// `container.rs`'s `MAX_ENTRY_LEN`, `MAX_ARCHIVE_LEN` and
/// `MAX_DISK_ENTRIES`); this is that rule applied here.
#[test]
fn a_file_that_amplifies_past_the_byte_cap_is_refused() {
    // 256 songs x 8 block records, each copying the whole ~3 KB file:
    // 5.2 MB asked for, against a 4 MiB cap. The block count stays under
    // MAX_BLOCKS on purpose, so this test can only be passing because the
    // byte budget refused it.
    let hostile = amplifying_ay(256, 8, u16::MAX, 3_000, 0);
    assert!(
        hostile.len() < 8_192,
        "the hostile file itself must be small"
    );
    const {
        assert!(
            256 * 8 < MAX_BLOCKS,
            "this case must not trip the block cap"
        )
    };

    let started = std::time::Instant::now();
    assert_eq!(parse(&hostile), Err(AyError::TooLarge));
    // Refusing has to be cheap as well as bounded: a cap that still copies
    // everything before deciding is not a cap.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "refusing a hostile file took {:?}",
        started.elapsed()
    );
}

/// The same defence from the other side: blocks that copy nothing at all
/// still cost a `Block` each, so the count is capped as well as the bytes.
#[test]
fn a_file_that_declares_more_blocks_than_the_cap_is_refused() {
    // 256 songs x 64 zero-length block records: 16,384 blocks, twice
    // MAX_BLOCKS, and not one byte of block data — so only the count cap
    // can refuse this.
    let hostile = amplifying_ay(256, 64, 0, 0, 0);
    const { assert!(256 * 64 > MAX_BLOCKS) };
    assert_eq!(parse(&hostile), Err(AyError::TooLarge));
}

/// The caps must sit above what real files need, not merely below what a
/// hostile one asks for. The largest file in the 696-file World of Spectrum
/// archive builds 40 blocks and expands to 255,365 bytes, so a file of that
/// shape has to pass without help.
#[test]
fn a_file_the_size_of_the_largest_real_one_still_parses() {
    let realistic = amplifying_ay(20, 2, u16::MAX, 12_800, 75);
    let file = parse(&realistic).unwrap();
    assert_eq!(file.songs.len(), 20);
    assert_eq!(file.songs[0].blocks.len(), 2);
    let total: usize = file
        .songs
        .iter()
        .flat_map(|song| &song.blocks)
        .map(|block| block.data.len())
        .sum();
    assert!(
        total > 255_365,
        "this fixture must exceed the largest real file's expansion, not sit under it: {total}"
    );
    assert!(total < MAX_BLOCK_BYTES);
}

/// Builds the string-amplifying shape: a valid 256-song file with no
/// blocks, whose author, misc and every song name point at a tail
/// containing no NUL, so each of the 258 strings reads from its pointer to
/// end of file.
///
/// 0xFF fill because Latin-1 above 0x7F costs two UTF-8 bytes in the
/// `String` this produces, which is the second half of the multiplier.
fn string_amplifying_ay(total_len: usize) -> Vec<u8> {
    let mut hostile = amplifying_ay(256, 0, 0, 0, 0);
    let text_at = hostile.len();
    hostile.resize(total_len, 0xFF);
    // Author (12), misc (14) and all 256 name pointers (20, 24, ...). Every
    // target is a couple of kilobytes away, so the deltas stay inside the
    // signed range the format specifies — the reach of a pointer is not
    // what is unbounded here, the read from it is.
    for at in [12usize, 14]
        .into_iter()
        .chain((0..256).map(|i| 20 + i * 4))
    {
        let delta = (text_at as i32 - at as i32) as i16;
        hostile[at..at + 2].copy_from_slice(&delta.to_be_bytes());
    }
    hostile
}

/// A file with no blocks at all must not amplify through its strings.
///
/// The block caps are consulted only inside the block loop, so a file
/// declaring zero blocks never reaches either of them. It can still aim all
/// 258 of its string pointers at one region with no NUL in it, and the
/// format gives a string no length — only a pointer and a NUL to stop at —
/// so each one reads to end of file.
///
/// Measured on this machine with the caps lifted: 2,001,067 bytes expanded
/// to 1,032,000,000 bytes of `String` in 671 ms, a 515x amplification
/// through a path consulting neither block cap.
#[test]
fn a_file_that_amplifies_through_its_strings_is_refused() {
    // A quarter of a megabyte is enough to prove the shape; the mechanism
    // does not change with size, and the fixture is the test's own cost.
    let hostile = string_amplifying_ay(256 * 1024);

    let started = std::time::Instant::now();
    assert_eq!(parse(&hostile), Err(AyError::TooLarge));
    // Refusing has to be cheap as well as bounded. `nt_string` stops
    // looking one byte past MAX_STRING_LEN, so this costs a kilobyte of
    // scanning rather than a pass over the file.
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "refusing a hostile file took {:?}",
        started.elapsed()
    );
}

/// One string longer than [`MAX_STRING_LEN`] is refused on its own, before
/// the whole-file budget is anywhere near spent — the two caps bound
/// different shapes, and a single unbounded string is the one the budget
/// alone would let through on a one-song file.
#[test]
fn a_single_over_long_string_is_refused() {
    let hostile = amplifying_ay(1, 0, 0, 0, MAX_STRING_LEN + 1);
    const { assert!(MAX_STRING_LEN + 1 < MAX_STRING_BYTES) };
    assert_eq!(parse(&hostile), Err(AyError::TooLarge));

    // And one byte under the cap still parses, so the boundary is where it
    // is claimed to be rather than somewhere convenient.
    let allowed = amplifying_ay(1, 0, 0, 0, MAX_STRING_LEN);
    let file = parse(&allowed).unwrap();
    assert_eq!(file.author.chars().count(), MAX_STRING_LEN);
}

/// The string caps must sit above what real files need. The longest string
/// in the 696-file archive is 75 bytes and the largest total is 749, in a
/// file carrying 18 of them.
#[test]
fn the_string_lengths_real_files_use_still_parse() {
    let realistic = amplifying_ay(20, 1, 256, 0, 75);
    let file = parse(&realistic).unwrap();
    assert_eq!(file.songs.len(), 20);
    assert_eq!(file.author.chars().count(), 75);
    let total = file.author.chars().count()
        + file.misc.chars().count()
        + file
            .songs
            .iter()
            .map(|song| song.name.chars().count())
            .sum::<usize>();
    assert!(
        total > 749,
        "this fixture must exceed the largest real file's text, not sit under it: {total}"
    );
    assert!(total < MAX_STRING_BYTES);
}

/// `parse` must answer rather than panic on whatever bytes it is handed.
/// `probe.rs`'s `identify_never_panics_on_arbitrary_input` covers naming a
/// format; this covers reading one, which is the path that follows pointers
/// and allocates.
#[test]
fn parse_never_panics_on_arbitrary_input() {
    let mut errors = std::collections::BTreeSet::new();
    for len in [0usize, 1, 8, 19, 20, 21, 64, 1_024] {
        for fill in [0x00u8, 0xFF, 0x55, 0xAA] {
            let mut bytes = vec![fill; len];
            errors.insert(format!("{:?}", parse(&bytes).map(|f| f.songs.len())));
            // The same lengths again with the magic in place, so the sweep
            // reaches the structure walk rather than stopping at the header.
            if len >= 8 {
                bytes[0..8].copy_from_slice(b"ZXAYEMUL");
                errors.insert(format!("{:?}", parse(&bytes).map(|f| f.songs.len())));
            }
        }
    }
    // Exact, not `is_empty`: a sweep asserting only "nothing crashed" would
    // keep passing if `parse` started refusing everything, and one asserting
    // "something was refused" would keep passing if it refused everything.
    // Measured 2026-08-29. `Ok(1)` is the all-zero fill with the magic
    // pasted on: a zero pointer is self-relative, so every field resolves to
    // its own position, the song count byte reads as one song, and the
    // address list terminates on its first entry. A structurally valid file
    // that describes nothing is a correct answer, not a near-miss.
    let expected: std::collections::BTreeSet<String> = [
        "Err(NotAnAyFile)".to_owned(),
        "Err(BadPointer)".to_owned(),
        "Err(Truncated)".to_owned(),
        "Ok(1)".to_owned(),
    ]
    .into();
    assert_eq!(errors, expected);
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
/// pins that fact against the magic and a header's worth of length and
/// nothing else, in a file that carries no `ay` feature gate at all. This
/// test pins the other half: a *real*, fully structured `.ay` file — the
/// fixture every other test in this file also parses — still identifies the
/// same way. Together they cover "the header is enough" and "a genuine file
/// still carries it", rather than one test standing in for both.
#[test]
fn an_ay_file_probes_as_certain() {
    let (format, confidence) = identify(&synthetic_ay()).unwrap();
    assert_eq!(format, Format::Ay);
    assert_eq!(confidence, Confidence::Certain);
}

// `ay_meta`'s output is pinned in `tests/metadata.rs`, by
// `ay_meta_reports_the_first_songs_name_as_the_title`. It asserted the same
// five fields against the same fixture as this file did, and only that one
// also covers the `Metadata::Ay` wrapping, so the pair collapsed into it.
