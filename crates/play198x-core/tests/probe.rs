//! What `identify` says about bytes, and how sure it says it.
//!
//! Every fixture is built here rather than shipped: no media enters the
//! repository. Each test asserts the exact `(Format, Confidence)` pair, because
//! the confidence is half the answer — an interface shows a `Probable`
//! identification differently from a `Certain` one, so a test that only checked
//! the format would pass while the distinction rotted away.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use format198x_commodore_amiga_ilbm::{Compression, Ilbm};
use format198x_commodore_amiga_mod::MAGIC_OFFSET;
use play198x_core::probe::{Confidence, Format, identify};

/// The eight bytes `identify` actually reads to award `Format::Ay` its
/// `Certain`: `ZXAY` at 0, `EMUL` at 4, nothing else. `tests/ay_format.rs`'s
/// `common::build_ay` (via `synthetic_ay`) also starts with these bytes, but
/// builds a whole parseable file around them — a song table, pointers, a
/// code block — none of which this identification rule reads. Restating
/// only the header here, rather than pulling that helper's full layout in,
/// is what makes this file's tests prove "the magic alone is enough": a
/// fact `an_ay_file_probes_as_certain` in the gated file, which parses a
/// complete fixture, cannot pin on its own.
fn ay_header() -> Vec<u8> {
    b"ZXAYEMUL".to_vec()
}

/// A four-channel `M.K.` module: one square-wave sample, one pattern, a C-2 on
/// channel 0 at row 0. The shape is the MOD crate's own test fixture, so what
/// `probe` accepts is what that crate can actually decode.
fn synthetic_module() -> Vec<u8> {
    let mut out = b"SYNTH".to_vec();
    out.resize(20, 0);
    for i in 0..31 {
        let mut header = vec![0u8; 30];
        if i == 0 {
            header[..6].copy_from_slice(b"square");
            header[22..24].copy_from_slice(&32u16.to_be_bytes()); // length in words
            header[25] = 64; // volume
            header[28..30].copy_from_slice(&32u16.to_be_bytes()); // loop length
        }
        out.extend_from_slice(&header);
    }
    out.push(1); // song length
    out.push(0); // restart position
    out.extend_from_slice(&[0u8; 128]); // order table
    out.extend_from_slice(b"M.K.");
    let mut pattern = vec![0u8; 64 * 4 * 4];
    let (period, sample) = (428u16, 1u8); // C-2, sample 1
    pattern[0] = (sample & 0xF0) | (period >> 8) as u8;
    pattern[1] = (period & 0xFF) as u8;
    pattern[2] = (sample & 0x0F) << 4;
    out.extend_from_slice(&pattern);
    out.extend_from_slice(&[0x40u8; 64]); // sample PCM
    out
}

/// A real `FORM ILBM` of `width × height`, four bitplanes, uncompressed.
fn synthetic_ilbm(width: u16, height: u16) -> Vec<u8> {
    let image = Ilbm {
        width,
        height,
        n_planes: 4,
        palette: (0..16).map(|i| [i * 16, i * 8, i * 4]).collect(),
        pixels: (0..usize::from(width) * usize::from(height))
            .map(|i| (i % 16) as u8)
            .collect(),
        camg: 0,
        x_aspect: 10,
        y_aspect: 11,
    };
    format198x_commodore_amiga_ilbm::encode(&image, Compression::None).unwrap()
}

/// A Koala file: the `0x6000` load address little-endian, then the payload.
fn synthetic_koala() -> Vec<u8> {
    let mut out = format198x_commodore_c64_koala::LOAD_ADDRESS
        .to_le_bytes()
        .to_vec();
    out.resize(format198x_commodore_c64_koala::FILE_LEN, 0x55);
    out
}

/// An Art Studio file at `len` bytes, which the format permits anywhere in
/// `MIN_FILE_LEN..=FILE_LEN` — the trailing pad is optional.
fn synthetic_art_studio(len: usize) -> Vec<u8> {
    let mut out = format198x_commodore_c64_art_studio::LOAD_ADDRESS
        .to_le_bytes()
        .to_vec();
    out.resize(len, 0xAA);
    out
}

#[test]
fn a_module_is_identified_by_its_magic_with_certainty() {
    assert_eq!(
        identify(&synthetic_module()),
        Some((Format::ProTracker, Confidence::Certain))
    );
}

#[test]
fn an_ilbm_is_identified_by_its_form_type_with_certainty() {
    assert_eq!(
        identify(&synthetic_ilbm(32, 16)),
        Some((Format::Ilbm, Confidence::Certain))
    );
}

#[test]
fn an_ay_file_is_identified_by_its_header_alone_with_certainty() {
    // `Format::Ay`'s rule is deliberately not behind the `ay` feature (see
    // its doc on `probe::Format`): naming an `.ay` costs eight byte
    // comparisons, not a Z80 or an AY chip. This test file carries no `ay`
    // feature gate at all, so a pass here — in whichever configuration this
    // crate happens to be built with — is what proves identification really
    // is reachable without the feature that plays the tune. It also proves
    // the header is genuinely the *whole* signal: `ay_header()` is eight
    // bytes and nothing more, not a full parseable file cut down to a
    // subset that happens to still work.
    assert_eq!(
        identify(&ay_header()),
        Some((Format::Ay, Confidence::Certain))
    );
}

/// The strengthened check holds: `identify` requires `EMUL` as well as
/// `ZXAY`, matching `player::ay::format::parse`'s own rejection (see
/// `tests/ay_format.rs`'s `rejects_bytes_that_are_not_an_ay_file`, which
/// pins the same byte string at the parser). `ZXAY` + `AMAD` is a real type
/// ID the wild `.ay` corpus carries, not a hypothetical one, so a file this
/// crate's own parser refuses must not be reported `Certain` here either.
#[test]
fn a_zxay_header_with_a_different_type_id_is_not_identified_as_an_ay() {
    assert_eq!(identify(b"ZXAYAMAD"), None);
}

/// The `.ay` counterpart to the ILBM and module collisions below: this rule
/// sits above SCR's length-only fallback specifically so a file that both
/// opens `ZXAYEMUL` and happens to run to exactly 6,912 bytes is still
/// identified as an `.ay`, not demoted to a probable screen dump. Moving the
/// `.ay` rule below SCR's would still pass every other test in this file —
/// this is the one that would catch it.
#[test]
fn an_ay_header_padded_to_6912_bytes_is_an_ay_and_not_a_screen() {
    let mut bytes = ay_header();
    assert!(bytes.len() < format198x_sinclair_zx_spectrum_scr::FILE_LEN);
    bytes.resize(format198x_sinclair_zx_spectrum_scr::FILE_LEN, 0);
    assert_eq!(identify(&bytes), Some((Format::Ay, Confidence::Certain)));
}

#[test]
fn a_koala_is_identified_by_load_address_and_length_with_certainty() {
    assert_eq!(
        identify(&synthetic_koala()),
        Some((Format::Koala, Confidence::Certain))
    );
}

#[test]
fn an_art_studio_bitmap_is_identified_with_or_without_its_trailing_pad() {
    for len in [
        format198x_commodore_c64_art_studio::MIN_FILE_LEN,
        format198x_commodore_c64_art_studio::FILE_LEN,
    ] {
        assert_eq!(
            identify(&synthetic_art_studio(len)),
            Some((Format::ArtStudio, Confidence::Probable)),
            "an Art Studio bitmap of {len} bytes"
        );
    }
}

#[test]
fn a_6912_byte_file_is_only_probably_a_screen() {
    assert_eq!(
        identify(&vec![0u8; format198x_sinclair_zx_spectrum_scr::FILE_LEN]),
        Some((Format::Scr, Confidence::Probable))
    );
}

#[test]
fn nothing_is_identified_from_an_empty_input() {
    assert_eq!(identify(&[]), None);
}

#[test]
fn nothing_is_identified_from_a_single_byte() {
    assert_eq!(identify(&[0x00]), None);
}

// --- Ordering: a weak signal must never shadow a strong one. ---

#[test]
fn a_koala_length_file_with_the_wrong_load_address_is_not_identified_at_all() {
    // Length alone is not evidence for a C64 bitmap: both C64 formats want the
    // load address *and* the length, so dropping either drops the whole
    // identification rather than demoting it to `Probable`.
    let mut bytes = synthetic_koala();
    bytes[..2].copy_from_slice(&format198x_commodore_c64_art_studio::LOAD_ADDRESS.to_le_bytes());
    assert_eq!(identify(&bytes), None);
}

#[test]
fn an_ilbm_that_happens_to_be_6912_bytes_is_an_ilbm_and_not_a_screen() {
    // The real collision. SCR is identified by length alone, so any format
    // whose file lands on 6,912 bytes trips that rule if it is tested first.
    //
    // Padded to length rather than sized to it: bytes past the FORM's declared
    // end are ignored by the ILBM decoder, so this is still a file that crate
    // reads, and the padding survives a change to what chunks `encode` writes.
    // Sizing the image itself to 6,912 would tie the fixture to today's chunk
    // arithmetic and break on the next CAMG or CMAP change, which is a fragile
    // way to state a fact about ordering.
    let mut bytes = synthetic_ilbm(32, 16);
    assert!(bytes.len() < format198x_sinclair_zx_spectrum_scr::FILE_LEN);
    bytes.resize(format198x_sinclair_zx_spectrum_scr::FILE_LEN, 0);
    assert_eq!(
        format198x_commodore_amiga_ilbm::decode(&bytes).map(|image| image.width),
        Ok(32),
        "the padded fixture must still be an ILBM, not merely start like one"
    );
    assert_eq!(identify(&bytes), Some((Format::Ilbm, Confidence::Certain)));
}

#[test]
fn a_module_that_happens_to_be_6912_bytes_is_a_module_and_not_a_screen() {
    // Same collision from the other strong signal. A module's magic sits at
    // offset 1080, so a 6,912-byte module is perfectly possible and must not
    // come back as a Spectrum screen.
    let mut bytes = synthetic_module();
    bytes.resize(format198x_sinclair_zx_spectrum_scr::FILE_LEN, 0);
    assert!(bytes.len() > MAGIC_OFFSET + 4);
    assert_eq!(
        identify(&bytes),
        Some((Format::ProTracker, Confidence::Certain))
    );
}

#[test]
fn identify_never_panics_on_arbitrary_input() {
    let mut hit = std::collections::BTreeSet::new();
    for len in [
        0usize, 1, 2, 11, 1080, 1084, 6911, 6912, 6913, 9002, 9009, 10003,
    ] {
        for fill in [0x00u8, 0xFF, 0x55, 0xAA, 0x60, 0x20] {
            if let Some((format, confidence)) = identify(&vec![fill; len]) {
                hit.insert(format!("{format:?}/{confidence:?}"));
            }
        }
    }

    // Exact, not `is_empty`. A sweep that identifies nothing proves only that
    // nothing crashed, and would keep passing if `identify` returned `None`
    // unconditionally; a sweep that asserts merely "something matched" would
    // keep passing if it started matching everything. Measured 2026-08-26 from
    // this list: uniform fills carry no magic, so ProTracker, ILBM and `.ay`
    // are unreachable here, and the only load address a uniform fill can
    // spell is 0x0000 — which is neither C64 format's. What is left is the
    // length-only rule, hit six times over at 6,912 bytes.
    let expected: std::collections::BTreeSet<String> = ["Scr/Probable".to_owned()].into();
    assert_eq!(hit, expected);
}

#[test]
fn a_uniform_sweep_reaches_every_rule_once_the_signals_are_real() {
    // The counterpart to the sweep above: the same shape of loop over fixtures
    // that do carry the signals, so all six rules are pinned to a positive
    // identification rather than only the one a uniform fill can reach.
    let cases: [(Vec<u8>, Format, Confidence); 6] = [
        (synthetic_module(), Format::ProTracker, Confidence::Certain),
        (synthetic_ilbm(32, 16), Format::Ilbm, Confidence::Certain),
        (ay_header(), Format::Ay, Confidence::Certain),
        (synthetic_koala(), Format::Koala, Confidence::Certain),
        (
            synthetic_art_studio(format198x_commodore_c64_art_studio::FILE_LEN),
            Format::ArtStudio,
            // Probable: $2000 is the commonest C64 load address and nothing
            // downstream can catch a miss -- the Art Studio decoder checks
            // exactly what identify already checked.
            Confidence::Probable,
        ),
        (
            vec![0u8; format198x_sinclair_zx_spectrum_scr::FILE_LEN],
            Format::Scr,
            Confidence::Probable,
        ),
    ];
    for (bytes, format, confidence) in cases {
        assert_eq!(identify(&bytes), Some((format, confidence)));
    }
}
