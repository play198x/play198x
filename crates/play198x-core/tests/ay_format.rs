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
    assert_eq!(song.lo_reg, 0x11);
    assert_eq!(song.hi_reg, 0x22);
    assert_eq!(song.stack, 0xC000);
    assert_eq!(song.init, 0x8000);
    assert_eq!(song.interrupt, 0x8010);
    assert_eq!(song.blocks.len(), 1);
    assert_eq!(song.blocks[0].address, 0x8000);
    assert_eq!(song.blocks[0].data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
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

/// `.ay` identification needs no `ay` feature (see `Format::Ay`'s doc), so
/// `tests/probe.rs` pins the same fact against a bare eight-byte magic with
/// no feature enabled at all. This test pins it against a *real*, fully
/// structured `.ay` file — the fixture every other test in this file also
/// parses — so the two together cover both "the magic alone is enough" and
/// "a genuine file still carries it".
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
