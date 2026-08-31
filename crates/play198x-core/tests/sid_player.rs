#![cfg(feature = "sid")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use play198x_core::player::sid::{SidPlayer, format::SidError};

fn psid(init: u16, play: u16, load: u16, body: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0; 0x7c];
    bytes[0..4].copy_from_slice(b"PSID");
    bytes[4..6].copy_from_slice(&2u16.to_be_bytes());
    bytes[6..8].copy_from_slice(&0x7cu16.to_be_bytes());
    bytes[8..10].copy_from_slice(&load.to_be_bytes());
    bytes[10..12].copy_from_slice(&init.to_be_bytes());
    bytes[12..14].copy_from_slice(&play.to_be_bytes());
    bytes[14..16].copy_from_slice(&1u16.to_be_bytes());
    bytes[16..18].copy_from_slice(&1u16.to_be_bytes());
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn callable_psid_initialises_and_renders_a_frame() {
    // init: RTS. play: write max volume at $D418, then RTS.
    let bytes = psid(
        0x1000,
        0x1001,
        0x1000,
        &[0x60, 0xa9, 0x0f, 0x8d, 0x18, 0xd4, 0x60],
    );
    let mut player = SidPlayer::new(&bytes, 0, 48_000).unwrap();
    player.frame().unwrap();
}

#[test]
fn mapped_rom_read_is_a_typed_refusal_but_banked_ram_is_not() {
    // LDA $E000; RTS with the default call bank maps KERNAL.
    let bytes = psid(0x1000, 0x1001, 0x1000, &[0x60, 0xad, 0x00, 0xe0, 0x60]);
    let mut player = SidPlayer::new(&bytes, 0, 48_000).unwrap();
    assert!(matches!(player.frame(), Err(SidError::NeedsRom(_))));

    // LDA #$34; STA $01; LDA $E000; RTS banks RAM under every ROM first.
    let bytes = psid(
        0x1000,
        0x1001,
        0x1000,
        &[0x60, 0xa9, 0x34, 0x85, 0x01, 0xad, 0x00, 0xe0, 0x60],
    );
    let mut player = SidPlayer::new(&bytes, 0, 48_000).unwrap();
    player.frame().unwrap();
}

#[test]
fn rsid_self_driven_and_missing_subtune_are_named() {
    let mut rsid = psid(0x1000, 0, 0x1000, &[0x60]);
    rsid[0..4].copy_from_slice(b"RSID");
    assert!(matches!(
        SidPlayer::new(&rsid, 0, 48_000),
        Err(SidError::RsidNotSupported)
    ));
    assert!(matches!(
        SidPlayer::new(&psid(0x1000, 0, 0x1000, &[0x60]), 0, 48_000),
        Err(SidError::SelfDrivenNotSupported)
    ));
    assert!(matches!(
        SidPlayer::new(&psid(0x1000, 0x1000, 0x1000, &[0x60]), 1, 48_000),
        Err(SidError::NoSuchSong)
    ));
}
