//! What identified bytes turn into: RGBA an interface can upload as a texture.
//!
//! Every test asserts a **specific pixel's colour**, never the buffer's length.
//! A length assertion passes for an all-black image, which is exactly what a
//! broken palette lookup produces — so a length is the one measurement that
//! cannot tell the working case from the failing one.
//!
//! Expected colours come from `mediaspec198x`, never from a literal. The two
//! disagreeing is precisely the defect worth catching, and a hard-coded triple
//! would hide it by construction.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use play198x_core::decode::image;
use play198x_core::probe::Format;

/// The RGBA the Spectrum's pinned default interpretation gives hardware colour
/// `index`, as the crate under test must also read it.
fn spectrum_rgba(index: usize) -> [u8; 4] {
    let machine = mediaspec198x::machine("sinclair-zx-spectrum").unwrap();
    let colour = machine.default_palette().unwrap().colours[index];
    [colour.r, colour.g, colour.b, 0xFF]
}

/// A 6,912-byte SCR whose top-left cell is INK 7 on PAPER 0, with the very
/// first pixel set and the rest of the cell clear. Built with the SCR crate's
/// own `encode`, so the file's bitmap interleave is the real one.
fn scr_with_one_set_pixel() -> Vec<u8> {
    let mut screen = format198x_sinclair_zx_spectrum_scr::Screen::blank();
    screen.bitmap[0] = 0b1000_0000;
    screen.attributes[0] = 0b0000_0111; // FLASH 0, BRIGHT 0, PAPER 0, INK 7
    format198x_sinclair_zx_spectrum_scr::encode(&screen).unwrap()
}

#[test]
fn a_spectrum_screen_decodes_to_its_palette_colours() {
    let img = image(&scr_with_one_set_pixel(), Format::Scr).unwrap();

    assert_eq!((img.width, img.height), (256, 192));
    assert_eq!(&img.rgba[0..4], spectrum_rgba(7), "set pixel takes ink 7");
    assert_eq!(
        &img.rgba[4..8],
        spectrum_rgba(0),
        "clear pixel takes paper 0"
    );
}

/// The RGBA the C64's pinned default interpretation gives hardware colour
/// `index`.
fn c64_rgba(index: usize) -> [u8; 4] {
    let machine = mediaspec198x::machine("commodore-c64").unwrap();
    let colour = machine.default_palette().unwrap().colours[index];
    [colour.r, colour.g, colour.b, 0xFF]
}

/// The mode-relative pixel shape the spec gives one of `machine`'s modes.
fn spec_aspect(machine: &str, mode: &str) -> (u32, u32) {
    let ratio = mediaspec198x::machine(machine)
        .unwrap()
        .mode(mode)
        .unwrap()
        .pixel_aspect;
    (u32::from(ratio.horizontal), u32::from(ratio.vertical))
}

/// A Koala file whose first cell spells out all four bit pairs in order, each
/// resolving through a different source: screen RAM's two nybbles, colour RAM,
/// and the shared background byte.
fn koala_with_one_cell() -> Vec<u8> {
    let mut picture = format198x_commodore_c64_koala::Koala::blank();
    picture.bitmap[0] = 0b01_10_11_00;
    picture.screen_ram[0] = 0x35; // %01 -> 3, %10 -> 5
    picture.color_ram[0] = 0x07; // %11 -> 7
    picture.background = 0x0B; // %00 -> 11
    format198x_commodore_c64_koala::encode(&picture).unwrap()
}

/// An Art Studio file whose first cell is colour 1 on colour 14, with only the
/// leftmost pixel set.
fn art_studio_with_one_cell() -> Vec<u8> {
    let mut picture = format198x_commodore_c64_art_studio::ArtStudio::blank();
    picture.bitmap[0] = 0b1000_0000;
    picture.screen_ram[0] = 0x1E; // set -> 1, clear -> 14
    format198x_commodore_c64_art_studio::encode(&picture).unwrap()
}

#[test]
fn a_koala_bitmap_resolves_every_bit_pair_through_its_own_source() {
    let img = image(&koala_with_one_cell(), Format::Koala).unwrap();

    assert_eq!((img.width, img.height), (160, 200));
    assert_eq!(
        &img.rgba[0..4],
        c64_rgba(3),
        "%01 takes screen RAM's high nybble"
    );
    assert_eq!(
        &img.rgba[4..8],
        c64_rgba(5),
        "%10 takes screen RAM's low nybble"
    );
    assert_eq!(&img.rgba[8..12], c64_rgba(7), "%11 takes colour RAM");
    assert_eq!(
        &img.rgba[12..16],
        c64_rgba(11),
        "%00 takes the shared background"
    );
}

#[test]
fn a_koala_pixel_is_double_wide_because_the_spec_says_so() {
    // The measurement a shell depends on: 160 double-wide pixels must be drawn
    // 320 across, or a Koala picture renders at half its real width. The
    // expected ratio comes from the spec, so a change there fails here rather
    // than silently changing what every shell draws.
    let img = image(&koala_with_one_cell(), Format::Koala).unwrap();
    assert_eq!(
        img.pixel_aspect,
        spec_aspect("commodore-c64", "multicolour-bitmap")
    );
    assert_eq!(img.pixel_aspect, (2, 1), "and the spec's answer is 2:1");
}

#[test]
fn an_art_studio_bitmap_takes_its_two_colours_from_the_cell() {
    let img = image(&art_studio_with_one_cell(), Format::ArtStudio).unwrap();

    assert_eq!((img.width, img.height), (320, 200));
    assert_eq!(
        &img.rgba[0..4],
        c64_rgba(1),
        "a set pixel takes the high nybble"
    );
    assert_eq!(
        &img.rgba[4..8],
        c64_rgba(14),
        "a clear pixel takes the low nybble"
    );
    assert_eq!(
        img.pixel_aspect,
        spec_aspect("commodore-c64", "hires-bitmap")
    );
}

#[test]
fn a_bright_cell_reaches_the_upper_half_of_the_spectrum_palette() {
    // BRIGHT is not a separate channel: it selects which half of the sixteen
    // palette entries both INK and PAPER come from. Ink 7 bright is index 15,
    // and the two entries differ, so getting the shift wrong cannot pass.
    let mut screen = format198x_sinclair_zx_spectrum_scr::Screen::blank();
    screen.bitmap[0] = 0b1000_0000;
    screen.attributes[0] = 0b0100_0111; // BRIGHT 1, PAPER 0, INK 7
    let bytes = format198x_sinclair_zx_spectrum_scr::encode(&screen).unwrap();

    let img = image(&bytes, Format::Scr).unwrap();

    assert_ne!(
        spectrum_rgba(15),
        spectrum_rgba(7),
        "the fixture must be able to tell them apart"
    );
    assert_eq!(
        &img.rgba[0..4],
        spectrum_rgba(15),
        "bright ink 7 is index 15"
    );
    assert_eq!(
        &img.rgba[4..8],
        spectrum_rgba(8),
        "bright paper 0 is index 8"
    );
}

// --- The Amiga, which brings its own colours ---

/// A four-plane ILBM with a palette nothing else could have produced, and a
/// diagonal of index values so a transposed walk cannot pass.
fn synthetic_ilbm(
    width: u16,
    height: u16,
    camg: u32,
    compression: format198x_commodore_amiga_ilbm::Compression,
) -> Vec<u8> {
    let picture = format198x_commodore_amiga_ilbm::Ilbm {
        width,
        height,
        n_planes: 4,
        palette: (0..16u8).map(|i| [i * 16, 255 - i * 16, i * 3]).collect(),
        pixels: (0..usize::from(width) * usize::from(height))
            .map(|i| (i % 16) as u8)
            .collect(),
        camg,
        x_aspect: 10,
        y_aspect: 11,
    };
    format198x_commodore_amiga_ilbm::encode(&picture, compression).unwrap()
}

#[test]
fn an_ilbm_takes_its_colours_from_its_own_cmap() {
    // No `mediaspec198x` lookup happens on this path and none should: the
    // Amiga's colour registers are a parametric gamut, so the machine has no
    // default table, and the file states the twelve-bit values it used.
    for compression in [
        format198x_commodore_amiga_ilbm::Compression::None,
        format198x_commodore_amiga_ilbm::Compression::ByteRun1,
    ] {
        let bytes = synthetic_ilbm(32, 16, 0, compression);
        let img = image(&bytes, Format::Ilbm).unwrap();

        assert_eq!((img.width, img.height), (32, 16), "{compression:?}");
        // Pixel n has index n % 16, and CMAP entry i is [i*16, 255-i*16, i*3].
        assert_eq!(
            &img.rgba[0..4],
            [0, 255, 0, 255],
            "{compression:?}: index 0"
        );
        assert_eq!(
            &img.rgba[4..8],
            [16, 239, 3, 255],
            "{compression:?}: index 1"
        );
        assert_eq!(
            &img.rgba[60..64],
            [240, 15, 45, 255],
            "{compression:?}: index 15"
        );
    }
}

#[test]
fn an_ilbm_pixel_shape_follows_its_camg_hires_bit() {
    // The BMHD's own xAspect/yAspect say 10:11 in both fixtures below, so a
    // decoder reading those would report one answer for two different pixel
    // shapes. The hires bit is what actually decides it.
    let lores = image(
        &synthetic_ilbm(
            32,
            16,
            0,
            format198x_commodore_amiga_ilbm::Compression::None,
        ),
        Format::Ilbm,
    )
    .unwrap();
    let hires = image(
        &synthetic_ilbm(
            32,
            16,
            format198x_commodore_amiga_ilbm::CAMG_HIRES,
            format198x_commodore_amiga_ilbm::Compression::None,
        ),
        Format::Ilbm,
    )
    .unwrap();

    assert_eq!(
        lores.pixel_aspect,
        spec_aspect("commodore-amiga-ocs", "lores-pal")
    );
    assert_eq!(
        hires.pixel_aspect,
        spec_aspect("commodore-amiga-ocs", "hires-pal")
    );
    assert_ne!(
        lores.pixel_aspect, hires.pixel_aspect,
        "a hires pixel is not a lores pixel"
    );
}

#[test]
fn the_amiga_has_no_default_palette_and_that_is_the_answer() {
    // Pinned because the shape of this fact invites a "fix". A `None` here is
    // the spec saying the machine's palette is a parametric gamut, not the
    // spec missing a table — and the ILBM path never asks, because the file
    // carries its own colours. Routing around this `None` with a hard-coded
    // table would put a palette constant in `decode`, which is the one thing
    // the ruling behind this task forbids.
    let amiga = mediaspec198x::machine("commodore-amiga-ocs").unwrap();
    assert_eq!(amiga.default_interpretation, None);
    assert!(amiga.default_palette().is_none());
    assert_eq!(
        amiga.palette.gamut_size(),
        Some(4096),
        "a gamut, four bits per gun, not a fixed table"
    );

    // And the fixed-palette machines do pin one, which is what makes the
    // Amiga's `None` a statement rather than an omission.
    for id in ["sinclair-zx-spectrum", "commodore-c64"] {
        assert_eq!(
            mediaspec198x::machine(id).unwrap().default_interpretation,
            Some("emu198x-v1"),
            "{id}"
        );
    }
}

// --- Refusals: typed, named, and never a blank picture ---

/// The `what` of a `Decode` error for `format`, or a panic naming what came
/// back instead. Every refusal must arrive as this one variant.
fn decode_refusal(bytes: &[u8], format: Format) -> String {
    match image(bytes, format) {
        Err(play198x_core::Error::Decode { format: got, what }) => {
            assert_eq!(got, format, "the error must name the format that refused");
            assert!(!what.is_empty(), "the decoder's own message must survive");
            what
        }
        other => panic!("expected Decode for {format:?}, got {other:?}"),
    }
}

#[test]
fn a_truncated_screen_is_a_typed_error_naming_the_format() {
    let what = decode_refusal(&[0u8; 100], Format::Scr);
    assert!(
        what.contains("6912"),
        "the decoder's own message says what it wanted: {what}"
    );
}

#[test]
fn every_image_format_refuses_bytes_it_cannot_read_by_naming_itself() {
    // One sweep rather than four near-identical tests. The point is that the
    // error names the format that refused — a caller shows "this is not a
    // Koala", and a blank picture or a bare `Unrecognised` would tell them
    // nothing about which decoder gave up.
    for format in [Format::Scr, Format::Koala, Format::ArtStudio, Format::Ilbm] {
        let what = decode_refusal(b"not a picture", format);
        assert!(
            !what.contains("panic"),
            "{format:?} must refuse in words, not survive one: {what}"
        );
    }
}

#[test]
fn art_studio_names_itself_when_the_load_address_is_wrong() {
    // Art Studio's is the identification most likely to be wrong in the wild:
    // $2000 is the commonest C64 load address there is. When the bytes turn
    // out not to be one, the message has to say which decoder refused.
    let mut bytes = art_studio_with_one_cell();
    bytes[..2].copy_from_slice(&format198x_commodore_c64_koala::LOAD_ADDRESS.to_le_bytes());
    let what = decode_refusal(&bytes, Format::ArtStudio);
    assert!(
        what.contains("$2000"),
        "the message names the address it wanted: {what}"
    );
}

#[test]
fn an_art_studio_sized_file_at_2000_always_decodes_even_when_it_is_not_one() {
    // Pinned because it is a limit worth knowing, not a defect to fix here.
    // Art Studio's decoder checks exactly what `probe` already checked — the
    // $2000 load address and the length range — so nothing a probe accepts can
    // fail this decode. A misidentified $2000 file therefore surfaces as a
    // picture that looks wrong, never as an error, and the guard against that
    // is the identification, not the decode.
    let mut bytes = format198x_commodore_c64_art_studio::LOAD_ADDRESS
        .to_le_bytes()
        .to_vec();
    bytes.resize(format198x_commodore_c64_art_studio::FILE_LEN, 0x5A);

    let img = image(&bytes, Format::ArtStudio).unwrap();
    assert_eq!((img.width, img.height), (320, 200));
    // 0x5A everywhere: bitmap %01011010, screen RAM high nybble 5 / low nybble 10.
    assert_eq!(
        &img.rgba[0..4],
        c64_rgba(10),
        "bit 7 clear: the low nybble, colour 10"
    );
    assert_eq!(
        &img.rgba[4..8],
        c64_rgba(5),
        "bit 6 set: the high nybble, colour 5"
    );
}

#[test]
fn an_ilbm_whose_cmap_is_too_short_says_so_rather_than_painting_black() {
    // There is nothing to fall back on: this machine has no fixed table, and
    // inventing one would be exactly the palette constant `decode` must not
    // hold. So a pixel index past the CMAP is an error that names the index.
    let picture = format198x_commodore_amiga_ilbm::Ilbm {
        width: 4,
        height: 1,
        n_planes: 4,
        palette: vec![[1, 2, 3], [4, 5, 6]], // two entries for a four-plane image
        pixels: vec![0, 1, 9, 0],
        camg: 0,
        x_aspect: 10,
        y_aspect: 11,
    };
    let bytes = format198x_commodore_amiga_ilbm::encode(
        &picture,
        format198x_commodore_amiga_ilbm::Compression::None,
    )
    .unwrap();

    let what = decode_refusal(&bytes, Format::Ilbm);
    assert!(
        what.contains("index 9"),
        "the message names the index: {what}"
    );
}

#[test]
fn a_module_is_not_an_image_and_the_refusal_says_where_to_go_instead() {
    let what = decode_refusal(b"anything at all", Format::ProTracker);
    assert!(
        what.contains("decode::module"),
        "the refusal points at the right call: {what}"
    );
}

#[test]
fn decoding_never_panics_on_arbitrary_input() {
    // The FFI promise. A decoder rejecting bytes must arrive as `Decode`;
    // unwinding past this crate's boundary is undefined behaviour, not a crash.
    let mut refused = 0usize;
    let mut decoded = 0usize;
    for len in [0usize, 1, 2, 11, 6911, 6912, 9002, 9009, 10003] {
        for fill in [0x00u8, 0xFF, 0x55, 0x60, 0x20] {
            for format in [Format::Scr, Format::Koala, Format::ArtStudio, Format::Ilbm] {
                match image(&vec![fill; len], format) {
                    Ok(img) => {
                        decoded += 1;
                        assert_eq!(
                            img.rgba.len() as u64,
                            u64::from(img.width) * u64::from(img.height) * 4,
                            "a decoded picture's buffer must match its own dimensions"
                        );
                    }
                    Err(play198x_core::Error::Decode { .. }) => refused += 1,
                    other => panic!("expected Ok or Decode, got {other:?}"),
                }
            }
        }
    }

    // Exact, not "nothing crashed". Measured 2026-08-26: of the 180 calls,
    // exactly 5 decode — the five 6,912-byte fills read as SCR, which is the
    // one format with no header to contradict. Neither C64 format can be
    // reached by a uniform fill at all, because both want a two-byte load
    // address whose halves differ ($00 $60, $00 $20), which no single repeated
    // byte spells. A sweep asserting only that nothing panicked would keep
    // passing if every call started returning an error.
    assert_eq!((decoded, refused), (5, 175));
}
