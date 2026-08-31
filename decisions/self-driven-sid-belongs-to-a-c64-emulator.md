# Decision: Self-driven SID belongs to a C64 emulator

**Status:** Active  
**Date:** 2026-08-31  
**Issue:** [play198x/play198x#39](https://github.com/play198x/play198x/issues/39)

## Decision

Play198x identifies but does not execute RSID files or PSID files whose play
address is zero. They remain distinct typed refusals:

- `RsidNotSupported`: the file requires a power-on C64 machine;
- `SelfDrivenNotSupported`: the file requires hardware-driven interrupts
  rather than a callable play routine.

Both direct the listener toward Emu198x. Play198x will not reproduce part of a
C64 around its existing CPU and SID dependencies. There is therefore no
implementation plan for these formats inside `play198x-core`.

This is not a judgement that the files are unimportant. It assigns their verb
honestly: callable PSID asks a media host to invoke two routines; self-driven
SID asks a machine to keep executing a program.

## The minimum environment

HVSC's canonical
[`SID_file_format.txt`](https://www.hvsc.c64.org/download/C64Music/DOCUMENTS/SID_file_format.txt)
says RSID “strictly require[s] a true Commodore-64 environment” and receives
the default power-on environment. A zero play address means init installs an
interrupt handler; the format no longer gives the host a routine to call per
frame.

An honest implementation therefore needs, at minimum:

- continuous 6510 execution after init, including real IRQ/NMI entry and
  vectors rather than Play198x's bounded subroutine sentinel;
- CIA 1 and CIA 2 timers, interrupt masks/status/acknowledgement, and their
  observable registers;
- VIC-II raster position, raster compare, interrupt registers, and cycle
  timing; bus stealing matters to busy loops and sample playback even when no
  picture is rendered;
- the C64 PLA/banking contract across RAM, I/O, BASIC, KERNAL, and CHARGEN,
  including `$00`/`$01` processor-port behaviour;
- the format's PAL/NTSC power-on register and timer state, then every hardware
  change the tune makes itself;
- multiple SID address decoding for the 12 multi-SID RSIDs in HVSC #85;
- ROM behaviour for the 589 RSIDs carrying the C64 BASIC flag, plus any tune
  that reaches a KERNAL vector or mapped ROM at runtime.

That list is a partial C64 only in the sense that it omits the keyboard, disk,
and rendered pixels. In the timing and execution layers that determine the
audio, it is a C64 emulator. The reference implementation makes the same
boundary visible: libsidplayfp's C64 environment contains its own CPU, CIA,
VIC-II, MMU/banks, event scheduler, and SID rather than a callable-tune host.
VICE's VSID path likewise installs a driver into a C64 machine and lets CIA or
VIC-II generate the interrupts.

Emu198x currently publishes the CPU and SID leaf crates Play198x consumes. Its
CIA 6526 and VIC-II crates are not published. Reimplementing those chips here
would violate the binding thin-consumer rule; publishing them would remove a
dependency obstacle but would not change the semantic result that assembling
them with memory, interrupts, and power-on state creates a machine.

## Corpus and reference measurement

The entire local HVSC #85 corpus was parsed after enforcing RSID's reserved
fields and address constraints:

| Cohort | Files | Additional facts |
|---|---:|---|
| RSID | 3,924 (6.4%) | 589 C64 BASIC; 12 multi-SID |
| PSID with zero play address | 111 (0.18%) | no multi-SID files |

Every file reached its deliberate typed boundary: 3,924
`RsidNotSupported`, 111 `SelfDrivenNotSupported`. This census is retained as
an ignored HVSC #85 regression test; no media is committed.

For a reference-output check, the first 12 sorted files in each cohort were
rendered for two seconds at 8 kHz mono/32-bit float with sidplayfp using its
SIDLite engine. Its idle/DC floor in this run was approximately 0.0083 RMS.
Eight of 12 RSIDs and ten of 12 zero-play PSIDs exceeded 0.01 RMS in the first
two seconds. That is a lower bound—some tunes can start later—but it establishes
that both refused cohorts contain promptly audible work.

There is no honest waveform differential against Play198x because Play198x
correctly produces a typed refusal, not audio. A prototype that generated
samples without the minimum environment above would not be a candidate to
compare: plausible sound is not evidence that interrupt timing, digi samples,
or busy loops are correct.

## Why the two cohorts have the same destination

**RSID settles itself.** Its purpose is to distinguish files that require a
real C64 environment from older media-player approximations. Treating “Real
SID” as a request for a smaller host would contradict the format.

**Zero-play PSID is smaller, but crosses the same mechanism boundary.** There
are only 111 such files in 61,157. A host-supplied driver can establish initial
vectors, but after init the tune chooses CIA/VIC interrupt sources and may busy
loop. Supporting that contract honestly requires the same continuously
scheduled interrupt hardware, even if fewer files exercise every part of it.

## Consequences

- Callable, ROM-free PSID remains Play198x's complete SID playback surface.
- RSID and zero-play PSID stay identifiable, inspectable, and explicitly
  refused; malformed RSID headers are invalid rather than merely unsupported.
- Play198x does not acquire CIA/VIC implementations, ROMs, or a C64 machine
  package for these files.
- A future user-facing shell may offer “open in Emu198x” when an integration
  surface exists. That is hand-off, not playback inside `play198x-core`.
- This decision may be revisited only if the project boundary changes. Merely
  publishing more chip crates does not by itself change it.

