#![cfg(feature = "ay")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod common;

use common::{FIXTURE_HI_REG, FIXTURE_LO_REG, build_ay, build_ay_songs};
use play198x_core::host::memory::Memory;
use play198x_core::host::spectrum::{Ay3_8910, SpectrumHost};
use play198x_core::player::ay::AyPlayer;
use play198x_core::player::ay::format::AyError;

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
/// pair, split as HiReg (the high byte) / LoReg (the low byte) for all ten of
/// them, so HL — and every other pair — must read
/// `FIXTURE_HI_REG << 8 | FIXTURE_LO_REG` by the time init runs.
#[test]
fn register_pairs_are_set_from_loreg_and_hireg() {
    let player = AyPlayer::new(&ay_with_reg_pair_probe(), 0, 48_000).unwrap();
    assert_eq!(
        player.host.mem.read(0x9002),
        FIXTURE_LO_REG,
        "L (LoReg, the low byte) did not reach HL"
    );
    assert_eq!(
        player.host.mem.read(0x9003),
        FIXTURE_HI_REG,
        "H (HiReg, the high byte) did not reach HL"
    );
}

/// The register halves a song's init routine sees are that song's own.
///
/// This is what a multi-song `.ay` uses to select a subtune: the format
/// hands `init` the subtune number in `A`, the high half of `AF`, so `A`
/// must carry song *N*'s `HiReg` when song *N* is asked for. A player that
/// reads the two halves the wrong way round puts the index in `F` instead
/// and hands `init` a constant — every subtune plays song 0's music, and
/// nothing about the output looks like a failure.
///
///   init at 0x8000: 32 02 90  LD (0x9002),A
///                   22 04 90  LD (0x9004),HL
///                   C9        RET
#[test]
fn each_song_starts_with_its_own_register_state() {
    let halves = [(0x00u8, 0xF0u8), (0x01, 0xE1), (0x02, 0xD2)];
    let code = vec![0x32, 0x02, 0x90, 0x22, 0x04, 0x90, 0xC9];
    let bytes = build_ay_songs(&halves, 0x8000, 0x8000, 0x8000, &code);

    for (index, &(hi_reg, lo_reg)) in halves.iter().enumerate() {
        let player = AyPlayer::new(&bytes, index, 48_000).unwrap();
        assert_eq!(
            player.host.mem.read(0x9002),
            hi_reg,
            "song {index}: init must be handed this song's HiReg in A"
        );
        assert_eq!(
            player.host.mem.read(0x9004),
            lo_reg,
            "song {index}: L must carry this song's LoReg"
        );
        assert_eq!(
            player.host.mem.read(0x9005),
            hi_reg,
            "song {index}: H must carry this song's HiReg"
        );
    }
}

/// Builds an `.ay` whose interrupt routine programs channel A and sets its
/// volume, so the chip has something to render.
///   interrupt: writes AY registers 0 (fine tune), 1 (coarse), 7 (mixer),
///              8 (volume A), then RET
fn ay_that_programs_channel_a() -> Vec<u8> {
    // 4 register-write sequences of 14 bytes each start at 0x10, so the
    // block runs to 0x48 plus a trailing RET. 0x80 bytes, with room to
    // spare.
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
    build_ay(0x8000, 0x8010, 0x8000, &code)
}

/// A tune that programs channel A and sets the volume must produce sound.
/// Silence from a tune that ran is a failure, not a result — the whole
/// reason this test exists is that a player which renders zeroes still looks
/// like it works.
#[test]
fn a_tune_that_programs_the_chip_makes_a_noise() {
    let bytes = ay_that_programs_channel_a();
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

/// The chip's output must swing about zero, not about half its own peak.
///
/// `emu198x-gi-ay-3-8910` sums three channels from a unipolar volume table,
/// so its square wave arrives on a DC offset roughly half its amplitude.
/// Left in, that offset is a click on every start and stop, spends about
/// half the mix's headroom on something nobody can hear, and puts the chip
/// and the already-AC-coupled beeper on two different reference levels. Peak
/// alone cannot see any of that — a DC offset reads as loudness — so this
/// measures the mean, and checks the signal is still there while doing it.
#[test]
fn the_chip_output_is_ac_coupled() {
    let bytes = ay_that_programs_channel_a();
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();
    let mut out = vec![0.0f32; 48_000 / 50 * 2];

    // Past the filter's own start-up transient: at 35Hz it settles inside
    // about one frame, so ten is well clear.
    for _ in 0..10 {
        player.frame();
        player.render(&mut out);
    }

    let mut sum = 0.0f64;
    let mut count = 0u32;
    let mut peak = 0.0f32;
    for _ in 0..25 {
        player.frame();
        player.render(&mut out);
        for sample in &out {
            sum += f64::from(*sample);
            count += 1;
            peak = peak.max(sample.abs());
        }
    }
    let mean = sum / f64::from(count);

    assert!(
        peak > 0.01,
        "nothing was rendered, so the mean below proves nothing (peak {peak})"
    );
    assert!(
        mean.abs() < 0.05,
        "the chip's output sits on a DC offset of {mean} against a peak of {peak}"
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
/// must decay to silence rather than hold a constant DC offset forever.
///
/// A held level that keeps mixing a fixed +-BEEPER_GAIN into every later
/// frame is inaudible as sound and fatal as measurement: `tests/ay_corpus.rs`
/// judges a tune audible by its peak, so every tune that so much as touched
/// the port once would measure as permanently audible whatever it actually
/// played.
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
    // at the same non-zero peak the click left behind, so this is where it
    // would show.
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

/// An init routine that never returns must be reported, not silently
/// discarded into a player that renders nothing — the corpus sweep in
/// `tests/ay_corpus.rs` found 143 of 696 real archive files land here,
/// and until now nothing exercised the branch that decides that.
///   init at 0x8000: 18 FE  JR $  (an unconditional jump to itself: the
///                                 CPU spins here forever and never RETs)
#[test]
fn an_init_that_never_returns_is_reported_not_silently_played() {
    let code = vec![0x18, 0xFE];
    let bytes = build_ay(0x8000, 0x8000, 0x8000, &code);
    // `AyPlayer` isn't `Debug` (it holds a `Z80` core), so `unwrap_err()`
    // doesn't compile here — matched explicitly instead.
    match AyPlayer::new(&bytes, 0, 48_000) {
        Ok(_) => panic!("a spinning init routine should be reported, not swallowed"),
        Err(err) => assert_eq!(err, AyError::InitDidNotReturn),
    }
}

/// An interrupt routine that never returns must not eat the stack.
///
/// `call` pushes a two-byte sentinel return address so it can tell when a
/// routine has come back. A routine that never comes back never pops it, and
/// `frame()` runs 50 times a second — 100 bytes of stack a second, 30 KB
/// over five minutes, descending through the tune's own code and data on a
/// machine whose stack grows downward. The tune corrupts itself while
/// playing, which reads as a tune that falls apart rather than as a fault
/// here.
///
///   init at 0x8000:      C9     RET
///   interrupt at 0x8010: 18 FE  JR $ (spins forever)
#[test]
fn an_interrupt_that_never_returns_leaves_the_stack_where_it_found_it() {
    let mut code = vec![0u8; 0x20];
    code[0x00] = 0xC9;
    code[0x10..0x12].copy_from_slice(&[0x18, 0xFE]);

    let bytes = build_ay(0x8000, 0x8010, 0x8000, &code);
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();
    let sp_after_init = player.host.cpu.regs.sp;

    for frame in 1..=100 {
        assert!(
            !player.frame(),
            "frame {frame}: a spinning interrupt routine must be reported as not returning"
        );
        assert_eq!(
            player.host.cpu.regs.sp,
            sp_after_init,
            "frame {frame}: the stack moved by {} bytes",
            sp_after_init.wrapping_sub(player.host.cpu.regs.sp)
        );
    }
}

/// The counterpart: a routine that does return leaves the stack balanced
/// too, so the restore above cannot be hiding a routine's own imbalance.
#[test]
fn an_interrupt_that_returns_leaves_the_stack_where_it_found_it() {
    let mut player = AyPlayer::new(&ay_with_observable_stub(), 0, 48_000).unwrap();
    let sp_after_init = player.host.cpu.regs.sp;

    for frame in 1..=100 {
        assert!(
            player.frame(),
            "frame {frame}: the interrupt routine returned"
        );
        assert_eq!(player.host.cpu.regs.sp, sp_after_init, "frame {frame}");
    }
}

/// Builds a one-song `.ay` whose init routine is `code` at 0x8000 and whose
/// interrupt routine is the same address — every test below runs init only.
fn ay_with_init(code: &[u8]) -> Vec<u8> {
    build_ay(0x8000, 0x8000, 0x8000, code)
}

/// 0xC000-0xFFFF is a window onto one of eight RAM banks, and port 0x7FFD
/// says which. A write to the window must land in the selected bank and stay
/// there while another is paged over it.
///
/// This is what separates a 128K from a 64 KB machine, and a host that
/// ignored the port would pass every other test in this file: the tune runs,
/// the chip sounds, and the only symptom is that data written to one bank
/// turns up in another.
///
///   3E AA        LD A,0xAA
///   32 00 C0     LD (0xC000),A     ; into bank 0, the power-on selection
///   01 FD 7F     LD BC,0x7FFD
///   3E 01        LD A,1
///   ED 79        OUT (C),A         ; bank 1 at 0xC000
///   3E BB        LD A,0xBB
///   32 00 C0     LD (0xC000),A     ; into bank 1
///   3A 00 C0     LD A,(0xC000)
///   32 00 90     LD (0x9000),A
///   AF           XOR A
///   ED 79        OUT (C),A         ; bank 0 back
///   3A 00 C0     LD A,(0xC000)
///   32 01 90     LD (0x9001),A
///   C9           RET
#[test]
fn the_paged_window_switches_banks_and_each_bank_keeps_its_own_bytes() {
    let code = [
        0x3E, 0xAA, 0x32, 0x00, 0xC0, 0x01, 0xFD, 0x7F, 0x3E, 0x01, 0xED, 0x79, 0x3E, 0xBB, 0x32,
        0x00, 0xC0, 0x3A, 0x00, 0xC0, 0x32, 0x00, 0x90, 0xAF, 0xED, 0x79, 0x3A, 0x00, 0xC0, 0x32,
        0x01, 0x90, 0xC9,
    ];
    let player = AyPlayer::new(&ay_with_init(&code), 0, 48_000).unwrap();

    assert_eq!(
        player.host.mem.read(0x9000),
        0xBB,
        "bank 1 did not read back the byte written to it"
    );
    assert_eq!(
        player.host.mem.read(0x9001),
        0xAA,
        "bank 0's byte was overwritten by a write made while bank 1 was paged in"
    );
    assert_eq!(player.host.mem.paged_bank(), 0);
}

/// Banks 5 and 2 sit at fixed addresses *and* can be paged into the window,
/// so the same byte is reachable at two addresses at once. A model that gave
/// the window its own eight arrays would pass the test above and fail this
/// one.
///
///   3E 5A        LD A,0x5A
///   32 00 40     LD (0x4000),A     ; bank 5, at its fixed address
///   01 FD 7F     LD BC,0x7FFD
///   3E 05        LD A,5
///   ED 79        OUT (C),A         ; bank 5 into the window as well
///   3A 00 C0     LD A,(0xC000)
///   32 00 90     LD (0x9000),A
///   C9           RET
#[test]
fn bank_five_is_the_same_memory_at_4000_and_at_c000() {
    let code = [
        0x3E, 0x5A, 0x32, 0x00, 0x40, 0x01, 0xFD, 0x7F, 0x3E, 0x05, 0xED, 0x79, 0x3A, 0x00, 0xC0,
        0x32, 0x00, 0x90, 0xC9,
    ];
    let player = AyPlayer::new(&ay_with_init(&code), 0, 48_000).unwrap();

    assert_eq!(
        player.host.mem.read(0x9000),
        0x5A,
        "bank 5 paged into the window did not show what was written at 0x4000"
    );
}

/// The paging latch decodes A15 and A1 only, so 0x5FFD reaches it exactly as
/// 0x7FFD does. A host matching the documented port number alone would run
/// this tune against an address space that never moved.
///
///   01 FD 5F     LD BC,0x5FFD
///   3E 03        LD A,3
///   ED 79        OUT (C),A
///   C9           RET
#[test]
fn the_paging_latch_answers_every_address_it_decodes_not_just_7ffd() {
    let code = [0x01, 0xFD, 0x5F, 0x3E, 0x03, 0xED, 0x79, 0xC9];
    let player = AyPlayer::new(&ay_with_init(&code), 0, 48_000).unwrap();

    assert!(player.host.paging_written);
    assert_eq!(
        player.host.mem.paged_bank(),
        3,
        "a write to 0x5FFD must page memory: the latch does not decode A14-A2"
    );
}

/// Bit 4 asks for a different ROM. This host has none, and the RAM at
/// 0x0000-0x00FF is the `RET` stub the `.ay` format's player is required to
/// supply — so the bit must change nothing. If it swapped anything in over
/// that stub, every tune that calls into low memory would start executing
/// whatever the file happened to leave there.
///
///   01 FD 7F     LD BC,0x7FFD
///   3E 10        LD A,0x10        ; ROM select
///   ED 79        OUT (C),A
///   3A 00 00     LD A,(0x0000)
///   32 00 90     LD (0x9000),A
///   3A FF 00     LD A,(0x00FF)
///   32 01 90     LD (0x9001),A
///   C9           RET
#[test]
fn the_rom_select_bit_leaves_the_ret_stub_in_place() {
    let code = [
        0x01, 0xFD, 0x7F, 0x3E, 0x10, 0xED, 0x79, 0x3A, 0x00, 0x00, 0x32, 0x00, 0x90, 0x3A, 0xFF,
        0x00, 0x32, 0x01, 0x90, 0xC9,
    ];
    let player = AyPlayer::new(&ay_with_init(&code), 0, 48_000).unwrap();

    assert_eq!(player.host.mem.read(0x9000), 0xC9);
    assert_eq!(player.host.mem.read(0x9001), 0xC9);
}

/// Bit 5 locks the latch until the machine is reset, and nothing this player
/// does counts as a reset — a new song builds a new host, which is the only
/// power-cycle there is. So a tune that locks stays locked for the whole of
/// its own run, and the next song starts from a cold machine.
///
///   3E 11        LD A,0x11
///   32 00 C0     LD (0xC000),A    ; bank 0 marker
///   01 FD 7F     LD BC,0x7FFD
///   3E 01        LD A,1
///   ED 79        OUT (C),A        ; bank 1
///   3E 22        LD A,0x22
///   32 00 C0     LD (0xC000),A    ; bank 1 marker
///   3E 21        LD A,0x21        ; bank 1, and lock the latch
///   ED 79        OUT (C),A
///   AF           XOR A
///   ED 79        OUT (C),A        ; asks for bank 0; must be ignored
///   3A 00 C0     LD A,(0xC000)
///   32 00 90     LD (0x9000),A
///   C9           RET
#[test]
fn the_paging_disable_bit_holds_for_the_rest_of_the_song_and_no_longer() {
    let code = [
        0x3E, 0x11, 0x32, 0x00, 0xC0, 0x01, 0xFD, 0x7F, 0x3E, 0x01, 0xED, 0x79, 0x3E, 0x22, 0x32,
        0x00, 0xC0, 0x3E, 0x21, 0xED, 0x79, 0xAF, 0xED, 0x79, 0x3A, 0x00, 0xC0, 0x32, 0x00, 0x90,
        0xC9,
    ];
    let bytes = ay_with_init(&code);
    let player = AyPlayer::new(&bytes, 0, 48_000).unwrap();

    assert!(player.host.mem.paging_locked());
    assert_eq!(
        player.host.mem.paged_bank(),
        1,
        "a write after the lock still moved the bank"
    );
    assert_eq!(player.host.mem.read(0x9000), 0x22);

    let fresh = AyPlayer::new(&bytes, 0, 48_000).unwrap();
    assert!(
        !SpectrumHost::new().mem.paging_locked(),
        "a new host must start with paging unlocked"
    );
    assert_eq!(
        fresh.host.mem.read(0x9000),
        0x22,
        "the second run must reach the same state as the first, from a cold machine"
    );
}

/// `IN` from the AY's register port returns the selected register, not the
/// unattached bus. Ocean's 128K loaders and several tracker players read the
/// chip back, and a host answering 0xFF to that tells them there is no chip.
///
///   01 FD FF     LD BC,0xFFFD
///   3E 08        LD A,8
///   ED 79        OUT (C),A        ; select R8, channel A volume
///   01 FD BF     LD BC,0xBFFD
///   3E 0D        LD A,0x0D
///   ED 79        OUT (C),A        ; R8 = 0x0D
///   01 FD FF     LD BC,0xFFFD
///   ED 78        IN A,(C)         ; read R8 back
///   32 00 90     LD (0x9000),A
///   C9           RET
#[test]
fn reading_the_ay_port_returns_the_selected_register() {
    let code = [
        0x01, 0xFD, 0xFF, 0x3E, 0x08, 0xED, 0x79, 0x01, 0xFD, 0xBF, 0x3E, 0x0D, 0xED, 0x79, 0x01,
        0xFD, 0xFF, 0xED, 0x78, 0x32, 0x00, 0x90, 0xC9,
    ];
    let player = AyPlayer::new(&ay_with_init(&code), 0, 48_000).unwrap();

    assert!(player.ay_read);
    assert_eq!(
        player.host.mem.read(0x9000),
        0x0D,
        "the AY's register-read port returned something other than R8's value"
    );
}

/// Every other port still reads as absent. The chip is the one thing wired
/// to this host's bus; a joystick, a disk interface or the keyboard is not,
/// and a machine that answered them would be lying about what it is.
///
///   01 1F 00     LD BC,0x001F     ; the Kempston joystick's port
///   ED 78        IN A,(C)
///   32 00 90     LD (0x9000),A
///   C9           RET
#[test]
fn reading_a_port_nothing_is_wired_to_returns_the_unattached_bus() {
    let code = [0x01, 0x1F, 0x00, 0xED, 0x78, 0x32, 0x00, 0x90, 0xC9];
    let player = AyPlayer::new(&ay_with_init(&code), 0, 48_000).unwrap();

    assert!(player.any_port_read);
    assert!(!player.ay_read);
    assert_eq!(player.host.mem.read(0x9000), 0xFF);
}

/// An `.ay` block that lands in the paged window belongs to the window, not
/// to bank 0: the format addresses memory and never names a bank, so a tune
/// finds the file's bytes there whichever bank it selects.
///
/// The archive's one paging tune depends on exactly this — its code block
/// runs from 0xBA91 to 0xD970 and it selects bank 1, so a host that loaded
/// the window into bank 0 alone would have it page six kilobytes of its own
/// code out from under itself. This fixture is that shape in miniature: the
/// routine keeps executing across 0xC000 after switching banks.
///
///   at 0xC000:
///   01 FD 7F     LD BC,0x7FFD
///   3E 03        LD A,3
///   ED 79        OUT (C),A        ; bank 3 — the rest of this routine had
///                                 ; better still be here
///   3A 10 C0     LD A,(0xC010)
///   32 00 90     LD (0x9000),A
///   C9           RET
///   at 0xC010: 5A
#[test]
fn a_block_loaded_into_the_window_is_there_whichever_bank_is_paged_in() {
    let mut code = vec![0u8; 0x11];
    code[..14].copy_from_slice(&[
        0x01, 0xFD, 0x7F, 0x3E, 0x03, 0xED, 0x79, 0x3A, 0x10, 0xC0, 0x32, 0x00, 0x90, 0xC9,
    ]);
    code[0x10] = 0x5A;
    let bytes = build_ay(0xC000, 0xC000, 0xC000, &code);
    let player = AyPlayer::new(&bytes, 0, 48_000).unwrap();

    assert_eq!(player.host.mem.paged_bank(), 3);
    assert_eq!(
        player.host.mem.read(0x9000),
        0x5A,
        "the tune lost its own code and data by paging a bank in over them"
    );
}

/// Banks 2 and 5 keep what the file put at their own fixed addresses. They
/// are the two banks the file can address directly, so filling them with the
/// window's image instead would throw that away — and both are reachable
/// through the window as well, which is where it would show.
#[test]
fn mirroring_the_window_leaves_the_two_fixed_banks_alone() {
    let mut mem = Memory::new();
    mem.load(0x4000, &[0x55]); // bank 5, at its own address
    mem.load(0x8000, &[0x66]); // bank 2, at its own address
    mem.load(0xC000, &[0x77]); // the window
    mem.mirror_window_into_the_pageable_banks();

    for bank in [0u8, 1, 3, 4, 6, 7] {
        mem.page(bank);
        assert_eq!(
            mem.read(0xC000),
            0x77,
            "bank {bank} did not start with the window's loaded image"
        );
    }
    mem.page(5);
    assert_eq!(mem.read(0xC000), 0x55, "bank 5 lost what was put at 0x4000");
    mem.page(2);
    assert_eq!(mem.read(0xC000), 0x66, "bank 2 lost what was put at 0x8000");
}

/// Once the tune's routine has returned, nothing of the tune's runs for the
/// rest of the frame — and that must not depend on what happens to be in
/// memory at the return address.
///
/// A `HALT` byte written at the return address is not a safe way to stop
/// it, for two independent reasons. A block that covers the address
/// overwrites it — 57 of the archive's 1,536 playable tunes end a run with
/// something other than a `HALT` at 0xFFFF, measured against the model that
/// wrote one, before any banking existed. And 0xFFFF is inside the window a
/// `$7FFD` write repoints, where banks 2 and 5 carry the file's image of
/// their own fixed addresses rather than the window's.
///
/// A CPU left running from that address executes the tune's data as code
/// for the rest of every frame, which sounds like a tune playing badly
/// rather than like a fault. This fixture builds the second case: it pages
/// bank 5 into the window with a known non-`HALT` byte at the top of it.
///
///   init at 0x8000:      C9  RET
///   interrupt at 0x8010:
///   3E 5A        LD A,0x5A
///   32 FF 7F     LD (0x7FFF),A   ; the top byte of bank 5
///   01 FD 7F     LD BC,0x7FFD
///   3E 05        LD A,5
///   ED 79        OUT (C),A       ; bank 5 into the window: 0xFFFF is now
///                                ; that same byte, and it is not a HALT
///   C9           RET
#[test]
fn nothing_executes_after_the_routine_returns_whatever_is_at_the_sentinel() {
    let mut code = vec![0u8; 0x20];
    code[0x00] = 0xC9;
    code[0x10..0x1D].copy_from_slice(&[
        0x3E, 0x5A, 0x32, 0xFF, 0x7F, 0x01, 0xFD, 0x7F, 0x3E, 0x05, 0xED, 0x79, 0xC9,
    ]);
    let mut player = AyPlayer::new(&build_ay(0x8000, 0x8010, 0x8000, &code), 0, 48_000).unwrap();

    for frame in 1..=3 {
        assert!(player.frame(), "frame {frame}: the routine returned");
        assert_eq!(
            player.host.mem.read(0xFFFF),
            0x5A,
            "frame {frame}: the fixture is only meaningful while 0xFFFF is not a HALT"
        );
        assert_eq!(
            player.host.cpu.regs.pc, 0xFFFF,
            "frame {frame}: the CPU ran on past the address it returned to"
        );
    }
}

/// The host answers the AY's read port itself, so the public `step` gives
/// one answer rather than one per caller. A chip owned by `AyPlayer` and
/// patched in afterwards would leave anything else driving the same host
/// told there is no sound chip.
///
///   01 FD FF     LD BC,0xFFFD
///   3E 07        LD A,7
///   ED 79        OUT (C),A       ; select R7, the mixer, which keeps all 8 bits
///   01 FD BF     LD BC,0xBFFD
///   3E 3E        LD A,0x3E
///   ED 79        OUT (C),A       ; R7 = 0x3E
///   01 FD FF     LD BC,0xFFFD
///   ED 78        IN A,(C)
///   32 00 90     LD (0x9000),A
///   76           HALT
#[test]
fn the_host_answers_the_ay_port_with_no_player_driving_it() {
    let program = [
        0x01, 0xFD, 0xFF, 0x3E, 0x07, 0xED, 0x79, 0x01, 0xFD, 0xBF, 0x3E, 0x3E, 0xED, 0x79, 0x01,
        0xFD, 0xFF, 0xED, 0x78, 0x32, 0x00, 0x90, 0x76,
    ];

    let mut fitted = SpectrumHost::new();
    fitted.ay = Some(Ay3_8910::new(1_773_400, 48_000, 960));
    fitted.mem.load(0x8000, &program);
    fitted.cpu.regs.pc = 0x8000;
    for _ in 0..400 {
        fitted.step();
    }
    assert_eq!(
        fitted.mem.read(0x9000),
        0x3E,
        "a host with a chip fitted must answer its port from the chip"
    );

    let mut bare = SpectrumHost::new();
    bare.mem.load(0x8000, &program);
    bare.cpu.regs.pc = 0x8000;
    for _ in 0..400 {
        bare.step();
    }
    assert_eq!(
        bare.mem.read(0x9000),
        0xFF,
        "a host with no chip fitted must read as the unattached bus it is"
    );
}

/// The sentinel is only a return when an instruction *finished* on the cycle
/// PC reached it.
///
/// `Z80::instruction_complete` looks like the predicate for that and is not:
/// the crate documents it as a level that stays true throughout the
/// following opcode fetch. So an instruction sitting at 0xFFFE satisfies it
/// while its own operand byte — the one at 0xFFFF — is still being fetched,
/// and `call` reports a return that has not happened and stops the CPU
/// part-way through the instruction. The next frame then resumes that
/// instruction against the next routine's first byte.
///
/// This fixture puts `LD A,0x7B` at 0xFFFE, so PC passes through the
/// sentinel mid-instruction and then wraps to the `RET` stub, which pops the
/// real return. A player that stops on the level flag never loads `A`.
///
///   init at 0x0001:      the RET stub, so init returns at once
///   interrupt at 0xFFFE: 3E 7B  LD A,0x7B   (PC = 0xFFFF for the operand)
///                        then PC wraps to 0x0000 and the stub RETs
#[test]
fn the_sentinel_is_a_return_only_at_an_instruction_boundary() {
    // Not `init == 0`: the format reads that as "start at the first block's
    // address", which is the fixture's code rather than the stub.
    let bytes = build_ay(0x0001, 0xFFFE, 0xFFFE, &[0x3E, 0x7B]);
    let mut player = AyPlayer::new(&bytes, 0, 48_000).unwrap();

    assert!(player.frame(), "the routine did return, by way of the stub");
    assert_eq!(
        player.host.cpu.regs.af >> 8,
        0x7B,
        "the routine was stopped part-way through the instruction at 0xFFFE"
    );
    assert_eq!(player.host.cpu.regs.pc, 0xFFFF);
}

/// A `sample_rate` of zero is nonsense and must not panic or produce
/// infinities. `Engine::new` makes the same ruling for the same reason.
///
/// The tune has to make a noise for this to test anything. The failure is
/// not in the arithmetic that sets the rate up — it is in the DC blocker,
/// whose pole `1 - 2*pi*fc/fs` leaves the stable region entirely at a low
/// enough `fs`, and a filter with `|R| > 1` only diverges once something
/// feeds it. So this fixture drives the speaker every frame and runs well
/// past the frame where an unclamped pole reaches infinity, which is frame
/// 17 at `fs = 0`.
#[test]
fn a_zero_sample_rate_neither_panics_nor_diverges() {
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
    let mut player = AyPlayer::new(&bytes, 0, 0).unwrap();

    let mut out = vec![0.0f32; 64];
    for frame in 1..=40 {
        player.frame();
        player.render(&mut out);
        assert!(
            player.peak_before_clamp().is_finite(),
            "frame {frame}: a zero sample rate diverged to {}",
            player.peak_before_clamp()
        );
        assert!(
            out.iter().all(|sample| sample.is_finite()),
            "frame {frame}: a rendered sample was not finite"
        );
    }
}

/// Frame 0 is frame 0, not init's leftovers.
///
/// `new()` runs the tune's init routine through the whole host, so the chip
/// has been accumulating output and the beeper buffer filling before any
/// frame is asked for. Both are drained afterwards. The two accumulate at
/// different rates, so leaving them would not merely delay the start — it
/// would offset the chip against the beeper by different amounts.
///
///   init at 0x8000: writes the speaker high, programs channel A, then RET
#[test]
fn the_first_rendered_frame_does_not_carry_inits_output() {
    let mut code = vec![0u8; 0x40];
    code[..20].copy_from_slice(&[
        0x3E, 0x10, // LD A,0x10
        0xD3, 0xFE, // OUT (0xFE),A     speaker high
        0x01, 0xFD, 0xFF, // LD BC,0xFFFD
        0x3E, 0x08, // LD A,8
        0xED, 0x79, // OUT (C),A        select R8
        0x01, 0xFD, 0xBF, // LD BC,0xBFFD
        0x3E, 0x0F, // LD A,0x0F
        0xED, 0x79, // OUT (C),A        volume A full
        0xC9, 0x00, // RET
    ]);
    let player = AyPlayer::new(&build_ay(0x8000, 0x8000, 0x8000, &code), 0, 48_000).unwrap();

    assert!(
        player.buffered_beeper_samples() == 0,
        "init's beeper samples were carried into frame 0"
    );
}
