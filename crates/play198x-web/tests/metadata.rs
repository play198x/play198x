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
