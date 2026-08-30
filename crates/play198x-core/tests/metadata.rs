//! What an interface shows about a work.
//!
//! The file's centre is `every_sample_slot_is_listed_including_the_empty_ones`.
//! A ProTracker module's sample names are **content**: authors put greetings,
//! credits, jokes and whole paragraphs in them, routinely in slots that hold
//! no sample at all, because an empty slot's 22-byte name field is free text
//! the tracker never touches. A metadata view that lists only the slots a song
//! plays therefore drops exactly what a reader opened the module to see, and
//! does it silently. So every test here that touches names measures all 31
//! slots and the *positions* messages sit in, not just how many came back.
//!
//! The other hazard is the encoding. Amiga text is ISO-8859-1, and a UTF-8
//! reading of a name carrying an accent or a box-drawing byte comes back
//! mangled or empty — a failure that never shows up on the ASCII fixtures
//! anyone writes first. Both the title and a sample name are pinned against
//! Latin-1 bytes here for that reason.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Cell, SampleSpec, Text, module_with_text, square};
use format198x_commodore_amiga_mod::{Module, NUM_SAMPLES, ROWS_PER_PATTERN, Sample};
use play198x_core::decode;
use play198x_core::engine::Engine;
use play198x_core::metadata::{Metadata, image_meta, module_meta};
use play198x_core::probe::Format;

const RATE: u32 = 44_100;

/// One row at the default speed 6 and tempo 125.
const ROW_MS: u128 = 120;

fn square_sample() -> SampleSpec {
    SampleSpec {
        data: square(32, 1, 100),
        volume: 64,
        repeat_start_words: 0,
        repeat_length_words: 16,
    }
}

/// A blank name for every slot, for a test to write into.
fn blank_names() -> Vec<&'static [u8]> {
    vec![&[]; NUM_SAMPLES]
}

/// A one-sample, one-pattern module whose text fields say what a test needs.
fn module_with(title: &[u8], sample_names: &[&[u8]]) -> Module {
    module_with_text(
        &Text {
            title,
            sample_names,
        },
        &[square_sample()],
        &[vec![Cell {
            row: 0,
            channel: 0,
            sample: 1,
            period: 428,
            ..Cell::default()
        }]],
        &[0],
        1,
    )
}

// ---------------------------------------------------------------------------
// Sample names are content
// ---------------------------------------------------------------------------

#[test]
fn every_sample_slot_is_listed_including_the_empty_ones() {
    let mut names = blank_names();
    names[0] = b"hello";
    names[2] = b"world";
    names[30] = b"greets to everyone";
    let module = module_with(b"HIDDEN MESSAGES", &names);

    // The fixture only measures anything if slots 2 and 30 really are unused.
    // A filter on "has data" would keep them if they carried a sample, and the
    // test would pass while the behaviour it exists for was gone.
    assert!(
        module.samples[2].data.is_empty(),
        "slot 2 must hold no sample data"
    );
    assert!(
        module.samples[30].data.is_empty(),
        "slot 30 must hold no sample data"
    );

    let meta = module_meta(&module, RATE);

    assert_eq!(
        meta.sample_names.len(),
        NUM_SAMPLES,
        "all 31 slots, not just the used ones"
    );
    assert_eq!(meta.sample_names[0], "hello");
    assert_eq!(
        meta.sample_names[1], "",
        "a blank slot is blank, not absent"
    );
    assert_eq!(meta.sample_names[2], "world");
    assert_eq!(
        meta.sample_names[30], "greets to everyone",
        "the last slot is as readable as the first"
    );
}

#[test]
fn a_latin1_sample_name_survives_in_a_slot_that_holds_no_sample() {
    // Both hazards in one name: it is in an unused slot, and it is Latin-1.
    // 0xE9 is `é`; 0xB1 is `±`, which is what a box-drawing byte from a
    // tracker's own character set decodes to.
    let mut names = blank_names();
    names[5] = b"caf\xE9 crew \xB1\xB1";
    let module = module_with(b"FIXTURE", &names);

    assert!(
        module.samples[5].data.is_empty(),
        "slot 5 must hold no sample data"
    );

    let meta = module_meta(&module, RATE);
    assert_eq!(meta.sample_names[5], "café crew ±±");
}

#[test]
fn a_latin1_title_is_not_flattened_to_empty() {
    let meta = module_meta(&module_with(b"caf\xE9", &[]), RATE);
    assert_eq!(meta.title, "café");
}

// ---------------------------------------------------------------------------
// The rest of what a module says about itself
// ---------------------------------------------------------------------------

#[test]
fn module_meta_reports_the_shape_the_module_states() {
    let module = module_with_text(
        &Text {
            title: b"SHAPE",
            sample_names: &[],
        },
        &[square_sample()],
        // Two stored patterns, a three-entry order table, and a song length
        // that plays two of them: three numbers that a naive implementation
        // conflates.
        &[vec![], vec![]],
        &[0, 1, 0],
        2,
    );

    let meta = module_meta(&module, RATE);

    assert_eq!(meta.title, "SHAPE");
    assert_eq!(meta.format_tag, "M.K.");
    assert_eq!(meta.channels, 4, "from the module, not from re-read magic");
    assert_eq!(meta.patterns, 2);
    assert_eq!(
        meta.orders, 2,
        "the played prefix, not the format's fixed 128-entry table"
    );
    assert_eq!(
        meta.timing.duration.as_millis(),
        2 * ROWS_PER_PATTERN as u128 * ROW_MS,
        "two full patterns of empty rows"
    );
    assert!(!meta.timing.loops);
    assert_eq!(
        meta.timing,
        Engine::new(module.clone(), RATE).timing(),
        "the same walk an engine already holding the module would do"
    );
}

#[test]
fn a_hand_built_module_reports_rather_than_panicking() {
    // Everything a real file cannot be and a caller can still hand us: a title
    // of raw high bytes, a name that is nothing but spaces, a magic no tracker
    // wrote, and not one stored pattern for the order table to name.
    let named = |name: [u8; 22]| Sample {
        name_bytes: name,
        data: Vec::new(),
        volume: 0,
        finetune_byte: 0,
        repeat_start_words: 0,
        repeat_length_words: 0,
    };
    let mut samples: [Sample; NUM_SAMPLES] = std::array::from_fn(|_| named([0u8; 22]));
    samples[0] = named([b' '; 22]);

    let module = Module {
        title_bytes: [0xFF; 20],
        samples,
        song_length: 1,
        order_table: [0; 128],
        restart: 0,
        magic: *b"ZZZZ",
        patterns: Vec::new(),
        trailing: Vec::new(),
    };

    let meta = module_meta(&module, RATE);

    assert_eq!(
        meta.title,
        "ÿ".repeat(20),
        "0xFF is Latin-1 ÿ, not a refusal"
    );
    assert_eq!(
        meta.sample_names[0],
        " ".repeat(22),
        "a name of spaces is spaces; trimming it would edit the author's text"
    );
    assert_eq!(meta.format_tag, "ZZZZ");
    assert_eq!(
        meta.channels, 4,
        "an unrecognised magic still names a shape"
    );
    assert_eq!(meta.patterns, 0);
    assert_eq!(
        meta.timing.duration.as_millis(),
        ROWS_PER_PATTERN as u128 * ROW_MS,
        "an order naming a pattern the file does not hold still takes its rows"
    );

    // A sample rate of zero is nonsense, and still not a panic: nothing about
    // how long a module lasts depends on it.
    assert_eq!(module_meta(&module, 0).timing, meta.timing);
}

// ---------------------------------------------------------------------------
// Pictures
// ---------------------------------------------------------------------------

#[test]
fn image_meta_carries_the_palette_that_produced_the_pixels() {
    let image = decode::image(&vec![0u8; 6912], Format::Scr).unwrap();
    let meta = image_meta(&image, "music.zip/screens/title.scr");

    assert_eq!(meta.format, Format::Scr);
    assert_eq!((meta.width, meta.height), (256, 192));
    assert_eq!(meta.source, "music.zip/screens/title.scr");

    // Taken from the spec, not written down here: if the two ever disagree the
    // test must fail, and a hard-coded copy would hide that.
    let spectrum = mediaspec198x::machine("sinclair-zx-spectrum")
        .unwrap()
        .default_palette()
        .unwrap();
    let expected: Vec<(u8, u8, u8)> = spectrum.colours.iter().map(|c| (c.r, c.g, c.b)).collect();
    assert_eq!(expected.len(), 16, "8 colours, bright and not");
    assert_eq!(meta.palette, expected);
}

#[test]
fn an_ilbms_palette_is_its_own_cmap() {
    // The Amiga has no default table to look up — its colour registers are a
    // gamut, not a palette — so this file's own CMAP is the only answer, and
    // it is one nothing else could have produced.
    let picture = format198x_commodore_amiga_ilbm::Ilbm {
        width: 16,
        height: 4,
        n_planes: 4,
        palette: (0..16u8).map(|i| [i * 16, 255 - i * 16, i * 3]).collect(),
        pixels: (0..64).map(|i| (i % 16) as u8).collect(),
        camg: 0,
        x_aspect: 10,
        y_aspect: 11,
    };
    let bytes = format198x_commodore_amiga_ilbm::encode(
        &picture,
        format198x_commodore_amiga_ilbm::Compression::None,
    )
    .unwrap();

    let meta = image_meta(&decode::image(&bytes, Format::Ilbm).unwrap(), "art.iff");

    assert_eq!(meta.format, Format::Ilbm);
    assert_eq!(meta.palette.len(), 16);
    assert_eq!(meta.palette[0], (0, 255, 0));
    assert_eq!(meta.palette[1], (16, 239, 3));
    assert_eq!(meta.palette[15], (240, 15, 45));
    assert_eq!(meta.source, "art.iff");
}

#[test]
fn the_metadata_enum_holds_one_shape_or_the_other() {
    let picture = Metadata::Image(image_meta(
        &decode::image(&vec![0u8; 6912], Format::Scr).unwrap(),
        "a.scr",
    ));
    let song = Metadata::Module(module_meta(&module_with(b"TUNE", &[]), RATE));

    match picture {
        Metadata::Image(meta) => assert_eq!(meta.source, "a.scr"),
        Metadata::Module(_) => panic!("a screen is not a module"),
        Metadata::Ay(_) => panic!("a screen is not an .ay tune"),
    }
    match song {
        Metadata::Module(meta) => assert_eq!(meta.title, "TUNE"),
        Metadata::Image(_) => panic!("a module is not a picture"),
        Metadata::Ay(_) => panic!("a module is not an .ay tune"),
    }
}

// ---------------------------------------------------------------------------
// .ay tunes
// ---------------------------------------------------------------------------

/// `AyMeta` and `Metadata::Ay` are always present (see their doc comments),
/// but building one from a real file needs `ay_meta`, which is behind the
/// `ay` feature because it takes an `AyFile`. That is the one part of this
/// file that needs the feature; the exhaustive matches above do not, which
/// is the whole point of the two rulings that split the gate this way.
///
/// The single place `AyMeta`'s fields are pinned. `tests/ay_format.rs` had a
/// second test asserting the same five against the same fixture, which only
/// this one also carries through the `Metadata` wrapping.
#[cfg(feature = "ay")]
#[test]
fn ay_meta_reports_the_first_songs_name_as_the_title() {
    use play198x_core::metadata::ay_meta;
    use play198x_core::player::ay::format::parse;

    let bytes = common::build_ay(0x8000, 0x8010, 0x8000, &[0xAA, 0xBB, 0xCC, 0xDD]);
    let file = parse(&bytes).unwrap();
    let meta = ay_meta(&file);

    // `.ay` carries no file-level title — only `PAuthor` and `PMisc` — so
    // `AyMeta::title` stands in with song 0's name, per its doc comment.
    assert_eq!(meta.title, "Test Tune");
    assert_eq!(meta.author, "Steve");
    assert_eq!(meta.misc, "notes");
    assert_eq!(meta.songs.len(), 1);
    assert_eq!(meta.songs[0].name, "Test Tune");
    assert_eq!(meta.songs[0].length_frames, 500);
    assert_eq!(meta.songs[0].fade_frames, 50);

    let wrapped = Metadata::Ay(meta);
    match wrapped {
        Metadata::Ay(meta) => assert_eq!(meta.author, "Steve"),
        Metadata::Image(_) | Metadata::Module(_) => {
            panic!("an .ay tune is neither a picture nor a module")
        }
    }
}

/// Each song's own length, not the first song's.
///
/// `AyMeta` used to carry a single `length_frames` taken from song 0, which
/// is wrong the moment an interface lets a visitor choose a song. The three
/// songs here are given **different** lengths deliberately: with equal ones
/// this test could not tell a correct implementation from the bug it exists
/// to catch.
#[test]
#[cfg(feature = "ay")]
fn ay_metadata_reports_each_songs_own_length() {
    use common::{AySongSpec, build_ay_songs_with};

    let spec = |hi, lo, length_frames, fade_frames| AySongSpec {
        hi_reg: hi,
        lo_reg: lo,
        length_frames,
        fade_frames,
    };
    let bytes = build_ay_songs_with(
        &[
            spec(0x11, 0x22, 100, 10),
            spec(0x33, 0x44, 250, 25),
            spec(0x55, 0x66, 999, 99),
        ],
        0x8000,
        0x8010,
        0x8000,
        &[0xC9],
    );

    let file = play198x_core::player::ay::format::parse(&bytes).unwrap();
    let meta = play198x_core::metadata::ay_meta(&file);

    assert_eq!(meta.author, "Steve");
    assert_eq!(meta.songs.len(), 3);
    assert_eq!(
        meta.songs
            .iter()
            .map(|s| s.length_frames)
            .collect::<Vec<_>>(),
        vec![100, 250, 999]
    );
    assert_eq!(
        meta.songs.iter().map(|s| s.fade_frames).collect::<Vec<_>>(),
        vec![10, 25, 99]
    );
    assert_eq!(meta.songs[1].name, "Test Tune 1");
}
