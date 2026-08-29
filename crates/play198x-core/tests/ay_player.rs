#![cfg(feature = "ay")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;

use common::build_ay;
use play198x_core::host::spectrum::SpectrumHost;
use play198x_core::player::ay::AyPlayer;

/// The host must execute code we hand it. This program writes 0x42 to
/// 0x9000 and halts:
///   3E 42     LD A,0x42
///   32 00 90  LD (0x9000),A
///   76        HALT
#[test]
fn the_host_executes_code_it_is_given() {
    let mut host = SpectrumHost::new();
    host.mem.load(0x8000, &[0x3E, 0x42, 0x32, 0x00, 0x90, 0x76]);
    host.cpu.regs.pc = 0x8000;

    for _ in 0..200 {
        host.step();
    }

    assert_eq!(host.mem.read(0x9000), 0x42, "the program did not run");
}

/// Builds an .ay whose init writes a marker and whose interrupt increments a
/// counter, so both halves of the stub are observable.
///   init at 0x8000:      3E 5A     LD A,0x5A
///                        32 00 90  LD (0x9000),A
///                        C9        RET
///   interrupt at 0x8010: 3A 01 90  LD A,(0x9001)
///                        3C        INC A
///                        32 01 90  LD (0x9001),A
///                        C9        RET
fn ay_with_observable_stub() -> Vec<u8> {
    let mut code = vec![0u8; 0x20];
    code[0x00..0x06].copy_from_slice(&[0x3E, 0x5A, 0x32, 0x00, 0x90, 0xC9]);
    code[0x10..0x19].copy_from_slice(&[0x3A, 0x01, 0x90, 0x3C, 0x32, 0x01, 0x90, 0xC9, 0x00]);
    build_ay(0x8000, 0x8010, 0x8000, &code)
}

#[test]
fn init_runs_once_and_the_interrupt_runs_per_frame() {
    let bytes = ay_with_observable_stub();
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();

    assert_eq!(player.host.mem.read(0x9000), 0x5A, "init did not run");
    assert_eq!(
        player.host.mem.read(0x9001),
        0,
        "interrupt ran before a frame"
    );

    for _ in 0..5 {
        player.frame();
    }
    assert_eq!(
        player.host.mem.read(0x9001),
        5,
        "interrupt did not run once per frame"
    );
}

/// Low memory must return rather than execute rubbish: the format's player
/// version 2 onward guarantees it, and tunes call into it.
#[test]
fn low_memory_is_filled_with_ret() {
    let player = AyPlayer::new(&ay_with_observable_stub(), 0, 48_000).unwrap();
    assert_eq!(player.host.mem.read(0x0000), 0xC9);
    assert_eq!(player.host.mem.read(0x00FF), 0xC9);
}
