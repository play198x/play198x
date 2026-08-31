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

// `Format::Ay` is not behind the core's `ay` feature (identifying one needs
// no Z80), and this crate does not enable `ay` — so `probe` can hand back
// `Format::Ay` in every build of this shell even though it never plays one.
// Before `format_name` learned this variant, that case crossed as
// `{format: "unknown", confidence: "certain"}`: an honest identification
// turned into a false claim of ignorance at the exact boundary this task's
// core change exists to keep honest.
//
// The fixture is `AY_MIN_LEN` bytes rather than the eight of the magic:
// `identify` reads only the magic, but it refuses anything shorter than the
// header `parse` needs, so that a file cannot identify as `Certain` and then
// fail to parse. Trimming this to eight bytes is the regression that rule
// exists to prevent, and this test is where it would show.
/// A `.ay` header long enough for `identify` to accept: the `ZXAY`/`EMUL`
/// magic, then zeroes out to the minimum length the core requires. The bytes
/// after the magic are never read here — this shell cannot play an `.ay` — so
/// they are zeroes rather than a hand-built song table.
fn ay_header() -> Vec<u8> {
    let mut bytes = b"ZXAYEMUL".to_vec();
    bytes.resize(20, 0);
    bytes
}

fn sid_header() -> Vec<u8> {
    let mut bytes = vec![0; 0x76];
    bytes[0..4].copy_from_slice(b"PSID");
    bytes
}

#[wasm_bindgen_test]
fn a_sid_file_crosses_the_probe_boundary_by_name() {
    let probed = play198x_web::probe(&sid_header()).expect("a full-length PSID header");
    assert_eq!(probed.format(), "sid");
    assert_eq!(probed.confidence(), "certain");
}

#[wasm_bindgen_test]
fn an_ay_file_probes_as_certain_even_though_this_shell_cannot_play_it() {
    let probed = play198x_web::probe(&ay_header()).expect("a full-length ZXAY/EMUL header");
    assert_eq!(probed.format(), "ay");
    assert_eq!(probed.confidence(), "certain");
}

/// `format_from_name("ay")` follows the same precedent as `"protracker"`:
/// both name a real `Format` this shell cannot turn into a picture, and both
/// are routed into `play198x_core::decode::image` anyway so its own named
/// refusal reaches the caller, rather than this shell's generic "not a
/// format this build knows" pretending the name was never recognised.
#[wasm_bindgen_test]
fn an_ay_file_is_a_named_refusal_not_an_unrecognised_format() {
    let err = play198x_web::decode_image(&ay_header(), "ay").unwrap_err();
    let message = format!("{err:?}");
    assert!(
        message.contains("not a picture"),
        "an .ay tune must be refused with the core's own reason, not treated as an unrecognised format name: {message}"
    );
}

/// A minimal valid ProTracker module: `M.K.` at offset 1080 (`MAGIC_OFFSET`
/// in `format198x-commodore-amiga-mod`), a song length of one order naming
/// pattern 0, and that one pattern stored as 1024 bytes of all-zero cells.
/// No samples are used, so the module decodes and plays but is silent
/// throughout — fine for tests that check `render`'s shape and counts, not
/// its audio content. Built in code per this crate's "no media committed"
/// rule; nothing here is a fixture file.
///
/// **Not fine for testing the left/right split.** Every rendered sample is
/// zero, so `left` and `right` are indistinguishable — a de-interleave that
/// swapped the two channels would pass every assertion built on this
/// fixture. See [`synthetic_module_panned_left`] for the fixture that
/// actually exercises that.
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

/// A module with one non-silent note, on channel 0 only — the fixture that
/// can actually catch a left/right swap in `render`'s de-interleave, which
/// [`synthetic_module`]'s all-zero content cannot: with no PCM, `left` and
/// `right` come back identical (both zero) whichever way the two channels
/// are assigned, so a swap there passes silently.
///
/// `play198x_core::engine`'s `PANNING` table hard-pans channel 0 (and 3)
/// left and channel 1 (and 2) right — ProTracker's fixed Amiga wiring, not
/// a choice this crate makes. Row 0 here plays C-2 (period 428) on channel 0
/// against a 64-byte constant-level sample at full volume; channels 1–3
/// carry no note at all, so they never retrigger and stay silent for the
/// engine's own reasons, independent of panning. The result the engine
/// itself guarantees: non-zero output on the left, exact silence on the
/// right. If `render`'s de-interleave ever swapped `left[i]`/`right[i]`,
/// this fixture's two assertions would both fail.
///
/// Sample 1's header: length 32 words (64 bytes, offset 22..24), volume 64
/// (offset 25, full scale), no loop (`repeat_start_words`/
/// `repeat_length_words` left at 0, which `Sample::is_looped` reads as
/// unlooped). At 48 kHz, C-2 advances the sample position by about 0.173
/// bytes per output frame (`PAULA_CLOCK_PAL / (2 * 428) / 48_000`), so 128
/// frames — one render quantum — consume about 22 of the sample's 64 bytes:
/// comfortably inside it, no loop needed to sustain output for the whole
/// buffer under test.
fn synthetic_module_panned_left() -> Vec<u8> {
    let mut bytes = vec![0u8; 20]; // title: left blank, not under test here
    for slot in 0..31u8 {
        let mut header = vec![0u8; 30];
        if slot == 0 {
            let sample_len_words: u16 = 32; // 64 bytes of PCM
            header[22..24].copy_from_slice(&sample_len_words.to_be_bytes());
            header[25] = 64; // volume: full scale
        }
        bytes.extend_from_slice(&header);
    }
    bytes.push(1); // song length: one order played
    bytes.push(0); // restart byte, unread by playback
    bytes.extend_from_slice(&[0u8; 128]); // order table; order 0 -> pattern 0
    bytes.extend_from_slice(b"M.K.");

    // Pattern 0: 64 rows * 4 channels * 4 bytes/cell, all zero except row 0's
    // channel 0 cell, which names sample 1 at period 428 (C-2).
    let mut pattern = vec![0u8; 64 * 4 * 4];
    let (period, sample_number) = (428u16, 1u8);
    pattern[0] = (sample_number & 0xF0) | ((period >> 8) as u8);
    pattern[1] = (period & 0xFF) as u8;
    pattern[2] = (sample_number & 0x0F) << 4;
    bytes.extend_from_slice(&pattern);

    // Sample 1's PCM: a constant, clearly non-zero level (+100 as signed
    // 8-bit) rather than anything near the noise floor, so there is no
    // ambiguity about whether the left buffer's non-zero reading is real.
    bytes.extend_from_slice(&[100u8; 64]);
    bytes
}

#[wasm_bindgen_test]
fn render_puts_a_hard_left_note_in_the_left_buffer_only() {
    let mut player = play198x_web::Player::new(&synthetic_module_panned_left(), 0, 48_000).unwrap();
    let quantum = play198x_web::Player::render_quantum();
    player.render(quantum);

    let left = player.debug_left();
    let right = player.debug_right();
    assert!(
        left.iter().any(|&s| s != 0.0),
        "channel 0's note must reach the left buffer: {left:?}"
    );
    assert!(
        right.iter().all(|&s| s == 0.0),
        "channel 0 is panned hard left; the right buffer must stay silent: {right:?}"
    );
}

#[wasm_bindgen_test]
fn a_player_renders_the_frames_it_is_asked_for() {
    let mut player = play198x_web::Player::new(&synthetic_module(), 0, 48_000).unwrap();
    let quantum = play198x_web::Player::render_quantum();
    assert_eq!(player.render(quantum), quantum);
}

#[wasm_bindgen_test]
fn render_clamps_a_request_past_the_render_quantum() {
    // The per-channel buffers are sized once, at construction, to exactly
    // `render_quantum()` frames and never grown — a request for more must
    // clip rather than reallocate, which is the property a cached
    // `Float32Array` view over them depends on.
    let mut player = play198x_web::Player::new(&synthetic_module(), 0, 48_000).unwrap();
    let quantum = play198x_web::Player::render_quantum();
    assert_eq!(player.render(quantum + 1_000), quantum);
}

#[wasm_bindgen_test]
fn a_paused_player_renders_silence_rather_than_stopping() {
    let mut player = play198x_web::Player::new(&synthetic_module(), 0, 48_000).unwrap();
    player.set_playing(false);
    let quantum = play198x_web::Player::render_quantum();
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
    let mut player = play198x_web::Player::new(&synthetic_module(), 0, 48_000).unwrap();
    let quantum = play198x_web::Player::render_quantum();
    player.render(quantum);
    let (left_before, right_before) = (player.left_ptr(), player.right_ptr());
    player.render(quantum);
    assert_eq!(player.left_ptr(), left_before);
    assert_eq!(player.right_ptr(), right_before);
}

#[wasm_bindgen_test]
fn bytes_that_are_not_a_module_are_an_error_not_a_guess() {
    assert!(play198x_web::Player::new(&screen(PAPER_CYAN_INK_BLACK), 0, 48_000).is_err());
}

#[wasm_bindgen_test]
fn a_fresh_player_starts_at_the_top_of_the_order_table() {
    let player = play198x_web::Player::new(&synthetic_module(), 0, 48_000).unwrap();
    assert_eq!(player.order(), Some(0));
    assert_eq!(player.pattern(), Some(0));
    assert_eq!(player.row(), Some(0));
    assert_eq!(player.tick(), Some(0));
}

#[wasm_bindgen_test]
fn seek_order_clamps_to_the_songs_played_prefix() {
    let mut player = play198x_web::Player::new(&synthetic_module(), 0, 48_000).unwrap();
    // The synthetic module plays exactly one order (song length 1), so any
    // order past that clamps back to the last playable one, order 0.
    player.seek_order(99);
    assert_eq!(player.order(), Some(0));
}

/// A minimal three-song `.ay` whose init and interrupt both return at once.
///
/// Built here rather than shared with the core's fixtures: the two crates
/// have separate test trees, and a `.ay` is small enough that a second
/// hand-written copy of the layout would be the larger cost. Only the parts
/// this shell reads are filled in — the song table and the strings — since
/// nothing here checks what the tune sounds like.
fn three_song_ay() -> Vec<u8> {
    let songs: [(u16, u16); 3] = [(100, 10), (250, 25), (999, 99)];
    let mut b = b"ZXAYEMUL".to_vec();
    b.push(0); // FileVersion
    b.push(3); // PlayerVersion
    b.extend_from_slice(&0i16.to_be_bytes()); // PSpecialPlayer
    let author_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes()); // PAuthor
    let misc_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes()); // PMisc
    b.push((songs.len() - 1) as u8); // NumOfSongs - 1
    b.push(0); // FirstSong
    let structure_ptr_at = b.len();
    b.extend_from_slice(&0i16.to_be_bytes()); // PPointsSongsStructure

    let rel = |from: usize, to: usize| ((to as i64) - (from as i64)) as i16;

    let author_at = b.len();
    b.extend_from_slice(b"Steve\0");
    let misc_at = b.len();
    b.extend_from_slice(b"notes\0");

    let mut name_at = Vec::new();
    for index in 0..songs.len() {
        name_at.push(b.len());
        b.extend_from_slice(format!("Song {index}\0").as_bytes());
    }

    let structure_at = b.len();
    let mut name_ptr_at = Vec::new();
    let mut data_ptr_at = Vec::new();
    for _ in 0..songs.len() {
        name_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes());
        data_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes());
    }

    let mut data_at = Vec::new();
    let mut points_ptr_at = Vec::new();
    let mut addrs_ptr_at = Vec::new();
    for &(length, fade) in &songs {
        data_at.push(b.len());
        b.extend_from_slice(&[0, 1, 2, 3]); // AChan..Noise
        b.extend_from_slice(&length.to_be_bytes()); // SongLength
        b.extend_from_slice(&fade.to_be_bytes()); // FadeLength
        b.push(0); // HiReg
        b.push(0); // LoReg
        points_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes()); // PPoints
        addrs_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes()); // PAddresses
    }

    let mut points_at = Vec::new();
    let mut addrs_at = Vec::new();
    let mut block_ptr_at = Vec::new();
    for _ in 0..songs.len() {
        points_at.push(b.len());
        b.extend_from_slice(&0xC000u16.to_be_bytes()); // Stack
        b.extend_from_slice(&0x8000u16.to_be_bytes()); // Init
        b.extend_from_slice(&0x8000u16.to_be_bytes()); // Interrupt

        addrs_at.push(b.len());
        b.extend_from_slice(&0x8000u16.to_be_bytes()); // block address
        b.extend_from_slice(&1u16.to_be_bytes()); // block length
        block_ptr_at.push(b.len());
        b.extend_from_slice(&0i16.to_be_bytes()); // block pointer
        b.extend_from_slice(&[0, 0, 0, 0, 0, 0]); // terminator
    }

    let block_at = b.len();
    b.push(0xC9); // RET

    let mut put = |at: usize, target: usize| {
        let delta = rel(at, target).to_be_bytes();
        b[at] = delta[0];
        b[at + 1] = delta[1];
    };
    put(author_ptr_at, author_at);
    put(misc_ptr_at, misc_at);
    put(structure_ptr_at, structure_at);
    for i in 0..songs.len() {
        put(name_ptr_at[i], name_at[i]);
        put(data_ptr_at[i], data_at[i]);
        put(points_ptr_at[i], points_at[i]);
        put(addrs_ptr_at[i], addrs_at[i]);
        put(block_ptr_at[i], block_at);
    }
    b
}

#[wasm_bindgen_test]
fn an_ay_builds_a_frame_driven_player() {
    let player = play198x_web::Player::new(&three_song_ay(), 0, 48_000).unwrap();

    assert_eq!(player.position_kind(), "frame");
    assert_eq!(player.song(), Some(0));
    assert_eq!(player.frame(), Some(0));
    // The module getters mean nothing here, and say so rather than lying
    // with a zero.
    assert_eq!(player.order(), None);
    assert_eq!(player.tick(), None);
}

#[wasm_bindgen_test]
fn an_ay_fills_a_whole_quantum_from_its_own_frames() {
    let mut player = play198x_web::Player::new(&three_song_ay(), 0, 48_000).unwrap();

    // The player produces 960-sample frames; the worklet asks for 128. Ten
    // calls crosses the seam, and every one must come back full.
    let quantum = play198x_web::Player::render_quantum();
    for _ in 0..10 {
        assert_eq!(player.render(quantum), quantum);
    }
    assert_eq!(player.debug_left().len(), quantum);
}

#[wasm_bindgen_test]
fn a_subtune_is_chosen_when_the_player_is_built() {
    // Each song is a separate entry point, so selecting one builds a player
    // rather than seeking an existing one.
    let player = play198x_web::Player::new(&three_song_ay(), 2, 48_000).unwrap();
    assert_eq!(player.song(), Some(2));
}

#[wasm_bindgen_test]
fn a_song_the_file_does_not_have_is_refused() {
    let err = match play198x_web::Player::new(&three_song_ay(), 3, 48_000) {
        Ok(_) => panic!("a song index past the end of the table must be refused"),
        Err(err) => err,
    };
    let message = format!("{err:?}");
    assert!(
        message.contains("NoSuchSong"),
        "the core's own reason should reach the caller: {message}"
    );
}

#[wasm_bindgen_test]
fn an_ay_cannot_be_seeked_and_says_so() {
    let mut player = play198x_web::Player::new(&three_song_ay(), 0, 48_000).unwrap();
    assert!(
        !player.seek_order(1),
        "a .ay has no seek; returning false is how a caller learns that"
    );
}

#[wasm_bindgen_test]
fn ay_metadata_reports_every_song_with_its_own_length() {
    let meta = play198x_web::ay_meta(&three_song_ay()).unwrap();

    assert_eq!(meta.author(), "Steve");
    assert_eq!(meta.misc(), "notes");
    assert_eq!(meta.song_count(), 3);
    assert_eq!(meta.song_name(1), Some("Song 1".to_string()));
    assert_eq!(meta.song_name(3), None);

    // 100, 250 and 999 frames at 50Hz. Deliberately different, so this test
    // can tell each song's own length from song 0's given three times.
    assert_eq!(meta.song_length_ms(0), Some(2_000.0));
    assert_eq!(meta.song_length_ms(1), Some(5_000.0));
    assert_eq!(meta.song_length_ms(2), Some(19_980.0));
    assert_eq!(meta.song_fade_ms(2), Some(1_980.0));
}
