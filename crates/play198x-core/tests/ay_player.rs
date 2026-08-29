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

/// Builds an .ay whose init immediately stores HL to memory and returns, so
/// the register-pair setup from LoReg/HiReg can be observed without
/// depending on which register the stub happens to touch first:
///   init at 0x8000: 22 02 90  LD (0x9002),HL
///                  C9        RET
fn ay_with_reg_pair_probe() -> Vec<u8> {
    let code = vec![0x22, 0x02, 0x90, 0xC9];
    build_ay(0x8000, 0x8000, 0x8000, &code)
}

/// The format hands the player one 16-bit value for every "common" register
/// pair, split as HiReg (high byte) / LoReg (low byte) for all ten of them —
/// `build_ay` fixes these at 0x22 / 0x11, so HL (and every other pair) must
/// read 0x2211 by the time init runs.
#[test]
fn register_pairs_are_set_from_loreg_and_hireg() {
    let player = AyPlayer::new(&ay_with_reg_pair_probe(), 0, 48_000).unwrap();
    assert_eq!(
        player.host.mem.read(0x9002),
        0x11,
        "L (LoReg, the low byte) did not reach HL"
    );
    assert_eq!(
        player.host.mem.read(0x9003),
        0x22,
        "H (HiReg, the high byte) did not reach HL"
    );
}

/// A tune that programs channel A and sets the volume must produce sound.
/// Silence from a tune that ran is a failure, not a result — the whole
/// reason this test exists is that a player which renders zeroes still looks
/// like it works.
///   interrupt: writes AY registers 0 (fine tune), 1 (coarse), 7 (mixer),
///              8 (volume A), then RET
#[test]
fn a_tune_that_programs_the_chip_makes_a_noise() {
    // 4 register-write sequences of 14 bytes each, starting at 0x10, need
    // 72 bytes (0x10 + 4*14 = 0x48) plus the trailing RET — a vec![0u8; 0x40]
    // (64 bytes) is too small and panics on the out-of-bounds slice copy.
    let mut code = vec![0u8; 0x80];
    // LD BC,0xFFFD / LD A,reg / OUT (C),A / LD BC,0xBFFD / LD A,val / OUT (C),A
    let mut at = 0x10;
    for (reg, val) in [(0u8, 0x00u8), (1, 0x01), (7, 0x3E), (8, 0x0F)] {
        code[at..at + 12].copy_from_slice(&[
            0x01, 0xFD, 0xFF, 0x3E, reg, 0xED, 0x79, 0x01, 0xFD, 0xBF, 0x3E, val,
        ]);
        code[at + 12] = 0xED;
        code[at + 13] = 0x79;
        at += 14;
    }
    code[at] = 0xC9; // RET

    let bytes = build_ay(0x8000, 0x8010, 0x8000, &code);
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();

    let mut peak = 0.0f32;
    let mut out = vec![0.0f32; 48_000 / 50 * 2];
    for _ in 0..25 {
        player.frame();
        player.render(&mut out);
        for sample in &out {
            peak = peak.max(sample.abs());
        }
    }
    assert!(
        peak > 0.01,
        "the chip was programmed but rendered silence (peak {peak})"
    );
}

/// A tune that toggles the speaker and nothing else must be audible.
///   interrupt: 30 iterations of: OUT (0xFE),A with bit 4 alternating,
///              separated by a delay, then RET
#[test]
fn a_beeper_only_tune_is_audible() {
    let mut code = vec![0u8; 0x40];
    code[0x10..0x20].copy_from_slice(&[
        0x06, 0x1E, // LD B,30
        0x3E, 0x10, // LD A,0x10   (speaker high)
        0xD3, 0xFE, // OUT (0xFE),A
        0x00, 0x00, // NOP NOP
        0x3E, 0x00, // LD A,0x00   (speaker low)
        0xD3, 0xFE, // OUT (0xFE),A
        0x10, 0xF4, // DJNZ back to the first LD A
        0xC9, 0x00, // RET
    ]);

    let bytes = build_ay(0x8000, 0x8010, 0x8000, &code);
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();

    let mut peak = 0.0f32;
    let mut out = vec![0.0f32; 48_000 / 50 * 2];
    for _ in 0..10 {
        player.frame();
        player.render(&mut out);
        for sample in &out {
            peak = peak.max(sample.abs());
        }
    }
    assert!(peak > 0.01, "a beeper tune rendered silence (peak {peak})");
}

/// A tune that writes the speaker port once, then never touches it again,
/// must decay to silence — not hold a constant DC offset forever. Before
/// this fix, `speaker_written` was a sticky "ever written" flag rather than
/// "written this frame": it kept mixing a fixed +-BEEPER_GAIN offset into
/// every frame after a single write. That would have corrupted Task 8's
/// peak-based audibility sweep over 696 real `.ay` files — every tune that
/// so much as touched the speaker port once would measure as permanently
/// audible regardless of what it actually played.
///
/// The write itself is real signal (a one-off click, exactly as it would be
/// on real AC-coupled hardware) and must still be audible; only the *held*
/// level afterwards must die away. This test checks both halves, because a
/// DC blocker that is too aggressive would kill the click along with the
/// offset, and one that does nothing would pass neither.
///   interrupt: on the very first call only (guarded by a flag byte at
///              0x9000), OUT (0xFE),A once with bit 4 set; every later call
///              is a no-op RET
#[test]
fn a_speaker_write_once_tune_settles_to_silence() {
    let mut code = vec![0u8; 0x40];
    code[0x10..0x20].copy_from_slice(&[
        0x3A, 0x00, 0x90, // LD A,(0x9000)   already written?
        0xB7, // OR A
        0x20, 0x09, // JR NZ,+9  -> RET
        0x3E, 0x01, // LD A,1
        0x32, 0x00, 0x90, // LD (0x9000),A
        0x3E, 0x10, // LD A,0x10   (speaker high)
        0xD3, 0xFE, // OUT (0xFE),A
        0xC9, // RET
    ]);

    let bytes = build_ay(0x8000, 0x8010, 0x8000, &code);
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();
    let mut out = vec![0.0f32; 48_000 / 50 * 2];

    // Frame 0 contains the one-time write: a real click, must be audible.
    player.frame();
    player.render(&mut out);
    let click_peak = out
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    assert!(
        click_peak > 0.01,
        "the one-time speaker write produced no audible click (peak {click_peak})"
    );

    // Frame 1: let the DC blocker settle further without asserting on it —
    // one frame in it is still partway down its decay curve, and pinning
    // the exact sample count the filter takes would make this test as
    // fragile as the bug it is guarding against.
    player.frame();
    player.render(&mut out);

    // By two frames after the click and onward, nothing has touched the
    // port again: a sticky DC offset would keep every one of these frames
    // at the same non-zero peak the click left behind, so this is exactly
    // where the old bug would have shown up.
    let mut settled_peak = 0.0f32;
    for _ in 0..8 {
        player.frame();
        player.render(&mut out);
        for sample in &out {
            settled_peak = settled_peak.max(sample.abs());
        }
    }
    assert!(
        settled_peak < 0.01,
        "a one-time speaker write left a constant DC offset instead of decaying to silence (peak {settled_peak})"
    );
}
