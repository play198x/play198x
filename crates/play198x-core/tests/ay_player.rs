#![cfg(feature = "ay")]
use play198x_core::host::spectrum::SpectrumHost;

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
