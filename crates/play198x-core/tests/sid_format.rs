#![cfg(feature = "sid")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use play198x_core::player::sid::format::{self, Clock, Kind, SidError, Speed};
use play198x_core::probe::{Confidence, Format, identify};

fn psid(play: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x7c];
    bytes[0..4].copy_from_slice(b"PSID");
    bytes[4..6].copy_from_slice(&2u16.to_be_bytes());
    bytes[6..8].copy_from_slice(&0x7cu16.to_be_bytes());
    bytes[8..10].copy_from_slice(&0x1000u16.to_be_bytes());
    bytes[10..12].copy_from_slice(&0x1000u16.to_be_bytes());
    bytes[12..14].copy_from_slice(&play.to_be_bytes());
    bytes[14..16].copy_from_slice(&2u16.to_be_bytes());
    bytes[16..18].copy_from_slice(&2u16.to_be_bytes());
    bytes[18..22].copy_from_slice(&2u32.to_be_bytes());
    bytes[0x16..0x1a].copy_from_slice(b"Tune");
    bytes[0x36..0x3c].copy_from_slice(b"Author");
    bytes[0x56..0x5a].copy_from_slice(b"1987");
    bytes[0x76..0x78].copy_from_slice(&(1u16 << 2).to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

fn rsid(body: &[u8]) -> Vec<u8> {
    let mut bytes = psid(0, body);
    bytes[0..4].copy_from_slice(b"RSID");
    bytes[8..10].fill(0);
    bytes[10..14].fill(0);
    bytes[18..22].fill(0);
    bytes[0x76..0x78].fill(0);
    bytes
}

#[test]
fn parses_header_metadata_timing_and_subtune_speed() {
    let bytes = psid(0x1001, &[0x60, 0x60]);
    assert_eq!(identify(&bytes), Some((Format::Sid, Confidence::Certain)));
    let file = format::parse(&bytes).unwrap();
    assert_eq!(file.kind, Kind::Psid);
    assert_eq!(file.clock, Clock::Pal);
    assert_eq!(file.title, "Tune");
    assert_eq!(file.start_song, 2);
    assert_eq!(file.speed(0), Speed::Vbi);
    assert_eq!(file.speed(1), Speed::Cia);
}

#[test]
fn zero_load_address_comes_from_little_endian_payload_prefix() {
    let mut bytes = psid(0x1001, &[0x34, 0x12, 0x60]);
    bytes[8..10].fill(0);
    let file = format::parse(&bytes).unwrap();
    assert_eq!(file.load_address, 0x1234);
    assert_eq!(file.data, [0x60]);
}

#[test]
fn rejects_truncation_bad_counts_and_overflow_without_panicking() {
    for len in 0..0x7c {
        assert!(format::parse(&psid(0x1001, &[])[..len]).is_err());
    }
    let mut no_songs = psid(0x1001, &[0x60]);
    no_songs[14..16].fill(0);
    assert!(matches!(
        format::parse(&no_songs),
        Err(SidError::InvalidHeader(_))
    ));
    let mut overflow = psid(0x1001, &[0; 2]);
    overflow[8..10].copy_from_slice(&0xffffu16.to_be_bytes());
    assert_eq!(format::parse(&overflow), Err(SidError::AddressOverflow));
}

#[test]
fn identifies_but_parser_names_rsid_and_self_driven_policy_separately() {
    assert_eq!(
        format::parse(&rsid(&[0x00, 0x10, 0x60])).unwrap().kind,
        Kind::Rsid
    );
    assert_eq!(format::parse(&psid(0, &[0x60])).unwrap().play_address, 0);
}

#[test]
fn rejects_rsid_fields_that_cannot_describe_a_real_c64_tune() {
    let valid = rsid(&[0x00, 0x10, 0x60]);
    for (range, value) in [
        (8..10, 0x1000u16.to_be_bytes()),
        (12..14, 0x1000u16.to_be_bytes()),
    ] {
        let mut malformed = valid.clone();
        malformed[range].copy_from_slice(&value);
        assert!(matches!(
            format::parse(&malformed),
            Err(SidError::InvalidHeader(_))
        ));
    }

    let mut speed = valid.clone();
    speed[18..22].copy_from_slice(&1u32.to_be_bytes());
    assert!(matches!(
        format::parse(&speed),
        Err(SidError::InvalidHeader(_))
    ));

    let mut low_load = valid.clone();
    low_load[0x7c..0x7e].copy_from_slice(&0x07e7u16.to_le_bytes());
    assert!(matches!(
        format::parse(&low_load),
        Err(SidError::InvalidHeader(_))
    ));

    let mut rom_init = valid.clone();
    rom_init[10..12].copy_from_slice(&0xa000u16.to_be_bytes());
    assert!(matches!(
        format::parse(&rom_init),
        Err(SidError::InvalidHeader(_))
    ));

    let mut basic_with_init = valid;
    basic_with_init[10..12].copy_from_slice(&0x1000u16.to_be_bytes());
    basic_with_init[0x76..0x78].copy_from_slice(&2u16.to_be_bytes());
    assert!(matches!(
        format::parse(&basic_with_init),
        Err(SidError::InvalidHeader(_))
    ));
}
