//! What `decode::module` makes of bytes `probe` called a module.
//!
//! Separate from the image tests because a module is played rather than shown,
//! and because the interesting case here is a refusal that is *correct*: three
//! of the seven magics this crate identifies name modules it cannot decode.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use play198x_core::decode::module;
use play198x_core::probe::{Confidence, Format, identify};

/// A four-channel module carrying `magic`: one square-wave sample, one
/// pattern, a C-2 on channel 0 at row 0. Parameterised on the magic so the
/// same fixture states both a module this crate plays and one it identifies
/// honestly and then declines.
fn module_bytes(magic: &[u8; 4]) -> Vec<u8> {
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
    out.extend_from_slice(magic);
    let mut pattern = vec![0u8; 64 * 4 * 4];
    let (period, sample) = (428u16, 1u8); // C-2, sample 1
    pattern[0] = (sample & 0xF0) | (period >> 8) as u8;
    pattern[1] = (period & 0xFF) as u8;
    pattern[2] = (sample & 0x0F) << 4;
    out.extend_from_slice(&pattern);
    out.extend_from_slice(&[0x40u8; 64]); // sample PCM
    out
}

#[test]
fn a_four_channel_module_decodes_to_what_the_file_says() {
    let decoded = module(&module_bytes(b"M.K.")).unwrap();

    assert_eq!(decoded.title(), "SYNTH");
    assert_eq!(decoded.channels(), 4);
    assert_eq!(decoded.orders(), &[0]);
    assert_eq!(decoded.patterns.len(), 1);
    assert_eq!(
        decoded.patterns[0][0][0].period, 428,
        "C-2 on channel 0, row 0"
    );
    assert_eq!(decoded.samples[0].name(), "square");
}

#[test]
fn a_six_or_eight_channel_module_is_identified_and_then_honestly_declined() {
    // `identify` accepts more than `decode` does, deliberately. `6CHN`, `8CHN`
    // and `FLT8` *are* modules — saying otherwise would be the lie — and the
    // decoder this crate delegates to handles four channels only. Two truthful
    // answers, not an inconsistency: a shell can say "an 8-channel module,
    // which this player cannot play yet" instead of "unrecognised file".
    //
    // Pinned so the next reader does not "fix" the probe into calling these
    // something they are not.
    for magic in [b"6CHN", b"8CHN", b"FLT8"] {
        let bytes = module_bytes(magic);
        let named = String::from_utf8_lossy(magic).into_owned();

        assert_eq!(
            identify(&bytes),
            Some((Format::ProTracker, Confidence::Certain)),
            "{named} is a module and identification must say so"
        );

        match module(&bytes) {
            Err(play198x_core::Error::Decode {
                format: Format::ProTracker,
                what,
            }) => assert!(
                what.contains("channel count") && what.contains(&named),
                "{named}: the refusal must carry the decoder's own reason: {what}"
            ),
            other => panic!("expected Decode for {named}, got {other:?}"),
        }
    }
}

#[test]
fn a_fifteen_sample_soundtracker_module_is_out_of_scope_at_both_ends() {
    // The older 15-sample Soundtracker layout has no magic at all — the field
    // at offset 1080 is sample data. Nothing identifies it, so nothing tries to
    // decode it, and this test exists to say that is the intended shape rather
    // than an oversight waiting to be patched into a length heuristic.
    let mut bytes = module_bytes(b"M.K.");
    bytes[1080..1084].copy_from_slice(&[0x11, 0x22, 0x33, 0x44]);

    assert_eq!(identify(&bytes), None, "no magic, no identification");
    match module(&bytes) {
        Err(play198x_core::Error::Decode {
            format: Format::ProTracker,
            what,
        }) => assert!(what.contains("magic"), "and the decoder agrees: {what}"),
        other => panic!("expected Decode, got {other:?}"),
    }
}

#[test]
fn decoding_a_module_never_panics_on_arbitrary_input() {
    // Truncation at every structural boundary the format has, plus lengths
    // either side of the fixed header. All must refuse in words.
    let full = module_bytes(b"M.K.");
    let mut refused = 0usize;
    for len in [0usize, 1, 1083, 1084, 1085, 2107, full.len() - 1] {
        match module(&full[..len.min(full.len())]) {
            Err(play198x_core::Error::Decode {
                format: Format::ProTracker,
                what,
            }) => {
                assert!(!what.is_empty(), "a refusal at {len} bytes must say why");
                refused += 1;
            }
            other => panic!("expected Decode at {len} bytes, got {other:?}"),
        }
    }
    assert_eq!(
        refused, 7,
        "every truncation refuses; none decodes by accident"
    );
}
