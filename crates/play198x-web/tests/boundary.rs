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

/// Attribute `0x28` is `0b0_0_101_000`: FLASH off, BRIGHT off, PAPER 5 (cyan),
/// INK 0 (black). Every bitmap bit is clear, so every pixel takes PAPER.
const PAPER_CYAN_INK_BLACK: u8 = 0x28;

/// `0x68` is the same with BRIGHT set, which moves both INK and PAPER into the
/// palette's upper half together.
const BRIGHT_PAPER_CYAN: u8 = 0x68;

fn pixel(rgba: &[u8], x: usize, y: usize, width: usize) -> (u8, u8, u8, u8) {
    let i = (y * width + x) * 4;
    (rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3])
}

#[wasm_bindgen_test]
fn a_clear_screen_is_its_paper_colour_everywhere() {
    let decoded = play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "scr").unwrap();

    assert_eq!(decoded.width(), 256);
    assert_eq!(decoded.height(), 192);

    let rgba = decoded.rgba();
    assert_eq!(rgba.len(), 256 * 192 * 4);

    // mediaspec198x emu198x-v1, index 5: cyan at the normal 0xC2 level.
    assert_eq!(pixel(&rgba, 0, 0, 256), (0x00, 0xC2, 0xC2, 0xFF));
    assert_eq!(pixel(&rgba, 255, 191, 256), (0x00, 0xC2, 0xC2, 0xFF));
}

#[wasm_bindgen_test]
fn a_set_bit_takes_ink_and_its_neighbour_does_not() {
    let mut bytes = screen(PAPER_CYAN_INK_BLACK);
    // Bitmap byte 0 is the leftmost eight pixels of row 0; bit 7 is x = 0.
    bytes[0] = 0x80;

    let decoded = play198x_web::decode_image(&bytes, "scr").unwrap();
    let rgba = decoded.rgba();

    assert_eq!(
        pixel(&rgba, 0, 0, 256),
        (0x00, 0x00, 0x00, 0xFF),
        "INK 0 is black"
    );
    assert_eq!(
        pixel(&rgba, 1, 0, 256),
        (0x00, 0xC2, 0xC2, 0xFF),
        "PAPER 5 is cyan"
    );
}

#[wasm_bindgen_test]
fn bright_selects_the_upper_half_of_the_palette() {
    let decoded = play198x_web::decode_image(&screen(BRIGHT_PAPER_CYAN), "scr").unwrap();
    // Index 13: bright cyan at 0xFF, not the 0xC2 of index 5.
    assert_eq!(pixel(&decoded.rgba(), 0, 0, 256), (0x00, 0xFF, 0xFF, 0xFF));
}

#[wasm_bindgen_test]
fn the_spectrum_pixel_is_square_and_says_so() {
    let decoded = play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "scr").unwrap();
    assert_eq!(decoded.pixel_aspect_w(), 1);
    assert_eq!(decoded.pixel_aspect_h(), 1);
}

#[wasm_bindgen_test]
fn the_palette_crosses_whole_and_in_hardware_order() {
    let decoded = play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "scr").unwrap();
    let palette = decoded.palette();

    assert_eq!(palette.len(), 16 * 3, "sixteen RGB triples");
    assert_eq!(&palette[0..3], &[0x00, 0x00, 0x00], "index 0 black");
    assert_eq!(&palette[15..18], &[0x00, 0xC2, 0xC2], "index 5 cyan");
    assert_eq!(
        &palette[39..42],
        &[0x00, 0xFF, 0xFF],
        "index 13 bright cyan"
    );
}

#[wasm_bindgen_test]
fn a_wrong_format_is_an_error_carrying_the_decoders_words() {
    let err = play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "koala").unwrap_err();
    let message = format!("{err:?}");
    // Not just non-empty — a non-empty string including one the shell
    // invented would still pass that. "decoder rejected the bytes" is
    // `play198x_core::Error::Decode`'s own Display wording (`lib.rs`: "the
    // {format:?} decoder rejected the bytes: {what}"), unchanged by the shell
    // per the boundary's contract ("errors carry play198x_core::Error's own
    // message unchanged — the shell invents no wording of its own").
    assert!(
        message.contains("decoder rejected the bytes"),
        "the error must carry the core's own wording, not just any text: {message}"
    );
}

#[wasm_bindgen_test]
fn an_unknown_format_name_is_an_error_not_a_guess() {
    assert!(play198x_web::decode_image(&screen(PAPER_CYAN_INK_BLACK), "jpeg").is_err());
}

/// A minimal valid ProTracker module: `M.K.` at offset 1080 (`MAGIC_OFFSET`
/// in `format198x-commodore-amiga-mod`), a song length of one order naming
/// pattern 0, and that one pattern stored as 1024 bytes of all-zero cells.
/// No samples are used, so the module decodes and plays but is silent
/// throughout — fine for boundary tests, which check `render`'s shape and
/// counts, not its audio content. Built in code per this crate's "no media
/// committed" rule; nothing here is a fixture file.
///
/// Layout: a 20-byte title, 31 30-byte sample headers (930 bytes), the
/// song-length byte (offset 950), the restart byte (951), the 128-byte order
/// table (952..1080), then the 4-byte magic — 1084 bytes total, exactly
/// `format198x_commodore_amiga_mod`'s documented header size.
fn synthetic_module() -> Vec<u8> {
    let mut bytes = vec![0u8; 1084];
    bytes[950] = 1; // song length: one order played
    // order_table[0] is already 0, naming pattern 0 — no jump needed.
    bytes[1080..1084].copy_from_slice(b"M.K.");
    bytes.extend_from_slice(&[0u8; 1024]); // pattern 0: 64 rows, all-zero cells
    bytes
}

#[wasm_bindgen_test]
fn a_player_renders_the_frames_it_is_asked_for() {
    let mut player = play198x_web::ModulePlayer::new(&synthetic_module(), 48_000).unwrap();
    let quantum = play198x_web::ModulePlayer::render_quantum();
    assert_eq!(player.render(quantum), quantum);
}

#[wasm_bindgen_test]
fn render_clamps_a_request_past_the_render_quantum() {
    // The per-channel buffers are sized once, at construction, to exactly
    // `render_quantum()` frames and never grown — a request for more must
    // clip rather than reallocate, which is the property a cached
    // `Float32Array` view over them depends on.
    let mut player = play198x_web::ModulePlayer::new(&synthetic_module(), 48_000).unwrap();
    let quantum = play198x_web::ModulePlayer::render_quantum();
    assert_eq!(player.render(quantum + 1_000), quantum);
}

#[wasm_bindgen_test]
fn a_paused_player_renders_silence_rather_than_stopping() {
    let mut player = play198x_web::ModulePlayer::new(&synthetic_module(), 48_000).unwrap();
    player.set_playing(false);
    let quantum = play198x_web::ModulePlayer::render_quantum();
    let rendered = player.render(quantum);
    assert_eq!(rendered, quantum, "a paused player still fills its buffers");
    assert!(
        player.debug_left().iter().all(|&s| s == 0.0),
        "left channel is silent"
    );
    assert!(
        player.debug_right().iter().all(|&s| s == 0.0),
        "right channel is silent"
    );
}

#[wasm_bindgen_test]
fn render_buffers_keep_the_same_address_across_calls() {
    // `left_ptr`/`right_ptr` are only useful to a caller that built a view
    // over them once and expects it to still be good on the next call —
    // this is the Rust-side half of that guarantee (the other half, that
    // wasm memory itself can still grow and detach the view regardless, is
    // documented on `wasm_memory` and is a JS-side concern this crate
    // cannot test from here).
    let mut player = play198x_web::ModulePlayer::new(&synthetic_module(), 48_000).unwrap();
    let quantum = play198x_web::ModulePlayer::render_quantum();
    player.render(quantum);
    let (left_before, right_before) = (player.left_ptr(), player.right_ptr());
    player.render(quantum);
    assert_eq!(player.left_ptr(), left_before);
    assert_eq!(player.right_ptr(), right_before);
}

#[wasm_bindgen_test]
fn bytes_that_are_not_a_module_are_an_error_not_a_guess() {
    assert!(play198x_web::ModulePlayer::new(&screen(PAPER_CYAN_INK_BLACK), 48_000).is_err());
}

#[wasm_bindgen_test]
fn a_fresh_player_starts_at_the_top_of_the_order_table() {
    let player = play198x_web::ModulePlayer::new(&synthetic_module(), 48_000).unwrap();
    assert_eq!(player.order(), 0);
    assert_eq!(player.pattern(), 0);
    assert_eq!(player.row(), 0);
    assert_eq!(player.tick(), 0);
}

#[wasm_bindgen_test]
fn seek_order_clamps_to_the_songs_played_prefix() {
    let mut player = play198x_web::ModulePlayer::new(&synthetic_module(), 48_000).unwrap();
    // The synthetic module plays exactly one order (song length 1), so any
    // order past that clamps back to the last playable one, order 0.
    player.seek_order(99);
    assert_eq!(player.order(), 0);
}
