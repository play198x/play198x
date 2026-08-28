#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `DecodedImage::metadata` closes the duplication the site used to carry:
//! before this method existed, the format label, the dimensions string and
//! the palette swatches were reimplemented in JavaScript from
//! `DecodedImage`'s raw fields. These tests pin that `metadata` reports
//! exactly what those raw fields already say, so a caller can delete that
//! copy and trust this one.

use wasm_bindgen_test::wasm_bindgen_test;

/// A 6912-byte SCREEN$, as in `tests/boundary.rs`: every bitmap bit clear,
/// every attribute the same.
fn screen(attribute: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; 6912];
    bytes[6144..].fill(attribute);
    bytes
}

const PAPER_CYAN_INK_BLACK: u8 = 0x28;

#[wasm_bindgen_test]
fn metadata_reports_the_same_facts_as_the_decoded_image() {
    let decoded = play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "scr").unwrap();
    let meta = decoded.metadata("tunes/title.scr");

    assert_eq!(meta.format(), "scr");
    assert_eq!(meta.width(), decoded.width());
    assert_eq!(meta.height(), decoded.height());
    assert_eq!(meta.palette(), decoded.palette());
    assert_eq!(meta.source(), "tunes/title.scr");
}

#[wasm_bindgen_test]
fn source_is_passed_through_exactly_as_given() {
    let decoded = play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "scr").unwrap();
    // Deliberately not a sanitary path — `source` is caller-supplied and is
    // never sanitised or reinterpreted, per its own documentation.
    let meta = decoded.metadata("../weird/../name.scr");
    assert_eq!(meta.source(), "../weird/../name.scr");
}

/// A module whose metadata is known by construction: a stated title, one
/// order naming pattern 0, and that pattern stored as 64 rows of silent
/// cells. Built in code per this crate's "no media committed" rule.
///
/// Silence is fine here. These tests read what a module *says about itself*
/// — title, shape, sample names, how long the walk takes — none of which
/// depends on a sample ever sounding. `tests/boundary.rs` carries the
/// fixtures that need real PCM.
fn titled_module(title: &str) -> Vec<u8> {
    let mut bytes = vec![0u8; 20];
    let title = title.as_bytes();
    bytes[..title.len()].copy_from_slice(title);
    bytes.extend_from_slice(&[0u8; 31 * 30]); // 31 empty sample headers
    bytes.push(1); // song length: one order played
    bytes.push(0); // restart byte, unread by playback
    bytes.extend_from_slice(&[0u8; 128]); // order table; order 0 -> pattern 0
    bytes.extend_from_slice(b"M.K.");
    bytes.extend_from_slice(&[0u8; 64 * 4 * 4]); // pattern 0, all-zero cells
    bytes
}

#[wasm_bindgen_test]
fn module_meta_reports_what_the_module_says_about_itself() {
    let meta = play198x_web::module_meta(&titled_module("play198x test tune")).unwrap();

    assert_eq!(meta.title(), "play198x test tune");
    assert_eq!(meta.format_tag(), "M.K.");
    assert_eq!(meta.channels(), 4);
    assert_eq!(meta.patterns(), 1);
    assert_eq!(meta.orders(), 1);
    // Always 31, including the slots holding no sample: the empty ones are
    // where authors wrote, so a caller must be able to see them.
    assert_eq!(meta.sample_names().len(), 31);
}

/// The duration is derivable rather than observed, which is the only way this
/// assertion is worth making: at ProTracker's default tempo 125 a tick is
/// `2500 / 125` = 20ms, the default speed of 6 ticks makes a row 120ms, and
/// one pattern of 64 rows is therefore 7,680ms.
#[wasm_bindgen_test]
fn one_pattern_at_the_default_tempo_lasts_exactly_one_pattern() {
    let meta = play198x_web::module_meta(&titled_module("")).unwrap();
    assert!(
        (meta.duration_ms() - 7_680.0).abs() < 1.0,
        "expected 7680ms for 64 rows at speed 6, tempo 125; got {}",
        meta.duration_ms()
    );
}

/// A song that runs off the end of its order table does **not** loop, and the
/// distinction is the whole point of the flag: `loops` reports "came back to a
/// position it had already played", not "ProTracker would start it again".
/// Playback wrapping round is a different thing from the walk finding a
/// repeat, and an interface that conflated them would label every module a
/// loop.
#[wasm_bindgen_test]
fn a_song_that_runs_off_the_end_does_not_loop() {
    let meta = play198x_web::module_meta(&titled_module("")).unwrap();
    assert!(!meta.loops());
    assert_eq!(meta.loop_start_ms(), None);
}

/// The counterpart: `B00` on the last row jumps back to order 0, which the
/// song has already played, so the walk stops there and reports the repeat.
/// The loop starts at the top, so the point playback returns to is 0ms.
#[wasm_bindgen_test]
fn a_song_that_jumps_back_reports_the_loop_and_where_it_starts() {
    let mut bytes = titled_module("");
    // Pattern 0 begins after the 1084-byte header; row 63, channel 0 is the
    // last cell in it. Effect B (position jump), parameter 0 (order 0).
    let last_cell = 1084 + 63 * 4 * 4;
    bytes[last_cell + 2] = 0x0B;
    bytes[last_cell + 3] = 0x00;

    let meta = play198x_web::module_meta(&bytes).unwrap();
    assert!(meta.loops(), "B00 returns to an order already played");
    assert_eq!(meta.loop_start_ms(), Some(0.0));
}

#[wasm_bindgen_test]
fn bytes_that_are_not_a_module_are_rejected_with_the_core_s_message() {
    assert!(play198x_web::module_meta(&[0u8; 32]).is_err());
}
