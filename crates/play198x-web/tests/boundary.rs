#![allow(clippy::unwrap_used, clippy::expect_used)]

use wasm_bindgen_test::wasm_bindgen_test;

/// A 6912-byte SCREEN$: every bitmap bit clear, every attribute the same.
/// `attribute` is `FBPPPIII` — see the plan's reference values.
fn screen(attribute: u8) -> Vec<u8> {
    let mut bytes = vec![0u8; 6912];
    bytes[6144..].fill(attribute);
    bytes
}

// `play198x_core::probe::identify` documents SCR as `Probable` and only
// `Probable` — a SCREEN$ has no magic and no structure, so its length is the
// entire signal (see `probe.rs`'s module docs). "certain" would be a false
// claim about what was measured, so this asserts what the core actually
// returns rather than the brief's original name and expectation.
#[wasm_bindgen_test]
fn a_screen_dollar_is_identified_probably() {
    let probed = play198x_web::probe(&screen(0x28)).expect("6912 bytes is a SCREEN$");
    assert_eq!(probed.format(), "scr");
    assert_eq!(probed.confidence(), "probable");
}

#[wasm_bindgen_test]
fn nothing_at_all_is_not_a_format() {
    assert!(play198x_web::probe(&[]).is_none());
    assert!(play198x_web::probe(&[0u8; 3]).is_none());
}
