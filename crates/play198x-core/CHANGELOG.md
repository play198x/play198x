# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0](https://github.com/play198x/play198x/compare/play198x-core-v0.2.0...play198x-core-v0.3.0) - 2026-08-29

### Changed

- **Breaking: the `ay` host is a 128K Spectrum's memory, not a flat 64 KB.**
  It had been claiming to be a 128K in its frame length and its AY clock while
  its RAM had no banking at all, so a tune that paged memory ran code the host
  quietly threw away. `Memory` now holds eight 16 KB RAM banks: bank 5 fixed at
  `$4000`, bank 2 at `$8000`, and port `$7FFD` choosing which bank shows
  through `$C000-$FFFF`. The latch is decoded as the hardware decodes it — A15
  and A1 low, so `$5FFD` and `$3FFD` reach it too — and bit 5 locks paging for
  the rest of the song, there being no reset short of starting the next one.
  `$0000-$3FFF` stays RAM whatever the ROM-select bit asks for: there is no ROM
  to page in, and that region holds the `RET` stub the `.ay` format's player is
  required to supply. An `.ay` block names an address and never a bank, so the
  file's image of `$C000-$FFFF` is what every pageable bank starts with —
  otherwise a tune that pages would find its own code gone. `Memory::page`
  returns whether the latch accepted the write.
- **Breaking: the AY chip belongs to `SpectrumHost`, as `SpectrumHost::ay`,
  and answers its own ports.** An `IN` from the AY's register port used to
  return `0xFF`, which is the right answer for a port with nothing behind it
  and the wrong one for the port the sound chip is on: it tells a tune probing
  for the chip that there is no chip. The host now answers from the chip when
  one is fitted and as the unattached bus when none is, so every caller of the
  public `step` gets one answer rather than one per caller. `SpectrumHost` has
  gained `ay: Option<Ay3_8910>` (with `Ay3_8910` re-exported, since the field
  is public) and lost `ay_write`, which existed only so the player could drain
  chip writes it no longer has to.
- **Breaking: `AyPlayer::fffd_read` and `AyPlayer::fffd_read_would_differ` are
  now `ay_read` and `ay_reads_non_ff`.** Both are corpus-sweep instrumentation
  that no production code reads. The old pair counted what a read *would* have
  returned had the chip been asked; the chip is asked now, so the counter
  reports what reads actually got.

### Fixed

- **a returned play routine no longer leaves the CPU running for the rest of
  the frame.** `AyPlayer::frame` clocks the chip and the beeper through the
  remainder of a frame after the tune's routine returns, and it used to clock
  the CPU with them, relying on a `HALT` byte parked at the return address to
  stop it. Any block that covers that address overwrites the byte — 57 of the
  archive's 1,536 playable tunes ended a run with something else there — and
  banking adds a second way to lose it, since the address is inside the window
  `$7FFD` repoints. Those tunes spent most of every frame executing their own
  data as code, which does not sound like a crash: Ghosts'n'Goblins' play
  routine never touches the sound chip at all, and every note it appeared to
  make was the runaway. The remainder of a frame now advances the chip, the
  beeper and the clock without the CPU, which is what the machine does while
  the player waits for its next interrupt. Across the archive this takes tunes
  overrunning their frame budget from 128 to 85 and overrunning frames from
  17,599 to 14,085. Eight subtunes that rendered silence now play; three that
  appeared to play now render silence, and all three are the fault rather
  than a loss — Ghosts'n'Goblins, Target Renegade song 4 and Star Dragon
  song 2 were making their sound by executing their own data or by resuming
  a mis-detected return, and each writes neither the chip nor the speaker
  once that stops.
- **the sentinel return address is recognised at an instruction boundary.**
  A bare `PC == 0xFFFF` check matches part-way through an instruction whose
  operand fetches pass through that address, which matters now that the CPU is
  left where the check stopped it. The boundary is an edge on
  `Z80::instructions_retired`, not `Z80::instruction_complete`, which is a
  level that stays true throughout the following opcode fetch and so still
  matches mid-instruction — and it is not a hypothetical mismatch: Star
  Dragon's song 2 was stopped mid-instruction by it, and the corrupted resume
  was where its beeper writes came from. `call` also runs on to the end of the
  instruction in flight when its budget expires, so a routine that never
  returns no longer leaves the core part-way through one for the next frame to
  resume. That is a behaviour change as well as a measurement one: ten beeper
  subtunes turn out to overrun every frame rather than every other one, and
  one of them (Starfox song 7) moves its peak by 13%.
- **frame 0 no longer carries the init routine's output.** `new()` runs init
  through the whole host, so the chip had been accumulating output and the
  beeper buffer filling before the first frame was asked for — and since the
  two accumulate at different rates, the first rendered frame was not merely
  late but internally skewed. Both are drained after init, and the beeper's DC
  blocker reset.
- **a `sample_rate` of zero no longer produces infinities.** Flooring the rate
  is not what does it: at 1 Hz the DC blocker's pole is -218.9, far outside
  the region where a one-pole high-pass converges, and a tune that drives the
  speaker reaches an infinite peak by its eighteenth frame. The pole itself is
  floored at 0, which leaves a plain difference — still a DC blocker, and
  incapable of diverging. The rate keeps its floor of 1 for the division, and
  the frame's sample count one of its own, because the chip's downsampler
  takes it as a divisor.
- **`probe::identify` and the `.ay` parser agree on how short is too short.**
  An eight-byte `ZXAYEMUL` file identified as `Confidence::Certain` and then
  failed to parse as `NotAnAyFile`. Both now use `probe::AY_MIN_LEN`, which is
  declared in the ungated module so the gated parser can share it.

## [0.2.0](https://github.com/play198x/play198x/compare/play198x-core-v0.1.3...play198x-core-v0.2.0) - 2026-08-29

### Added

- play Spectrum `.ay` tunes, behind the optional `ay` feature. An `.ay` file is
  Z80 code plus the addresses to call, not sample data, so `AyPlayer` loads the
  tune's blocks into a bare 128K Spectrum host — RAM, a CPU, and the two ports a
  tune makes a noise with — runs its init routine, then calls its interrupt
  routine once per 50Hz frame and renders the AY chip and the beeper together.
  No ROM is loaded and none is needed. The feature is off by default: a consumer
  decoding a SCREEN$ should not acquire a Z80 and a sound chip to do it.
- identify an `.ay` file and report what it says about itself, with no feature
  needed. `probe::identify` returns the new `Format::Ay` from the eight-byte
  `ZXAYEMUL` magic, and `Metadata::Ay` carries the file's author, its misc
  field, and every song's name. Both are ungated because naming a format costs
  nothing a caller has to pay for; only playing it needs the CPU.

### Changed

- **Breaking: `Metadata` has a new `Ay` variant.** `Metadata` is deliberately
  not `#[non_exhaustive]` (see its own doc), so an exhaustive `match` on it in
  a consumer stops compiling until it handles `Metadata::Ay`. Taken knowingly:
  the alternative — marking the enum `#[non_exhaustive]` to avoid this — would
  force every consumer into a wildcard arm forever, which loses the compiler
  error that tells them a new format has arrived. `Format` gained an `Ay`
  variant too, but `Format` is already `#[non_exhaustive]`, so that one breaks
  nothing.

## [0.1.3](https://github.com/play198x/play198x/compare/play198x-core-v0.1.2...play198x-core-v0.1.3) - 2026-08-27

### Fixed

- The `probe` module's documentation claimed Art Studio bitmaps were identified
  with `Confidence::Certain`. The code has always returned `Confidence::Probable`
  for them, so the table on docs.rs contradicted the function it described. The
  table now matches, and states the fact it never did: what divides the certain
  formats from the probable ones is whether a wrong answer can be caught. A magic
  number or a checksum fails to match; a load address and a length do not.

## [0.1.2](https://github.com/play198x/play198x/compare/play198x-core-v0.1.1...play198x-core-v0.1.2) - 2026-08-27

### Added

- open a container from resident bytes, for callers with no filesystem

## [0.1.1](https://github.com/play198x/play198x/compare/play198x-core-v0.1.0...play198x-core-v0.1.1) - 2026-08-27

**Nothing changes for a caller.** This moves
`format198x-commodore-amiga-adf` from 0.2.3 to 0.3.0
([#7](https://github.com/play198x/play198x/pull/7)), which fixes `Disk::verify`
reporting ordinary data disks as corrupt — any disk whose boot-checksum field
is zero, which is what AmigaDOS `Format` leaves until `Install` writes a
bootstrap.

`play198x-core` does not call `verify`, so no behaviour here is affected today.
It is taken deliberately rather than left: the container layer was sitting on a
version that would have misreported an ordinary data disk the moment it started
verifying one, and a minor bump on a `0.x` crate is semver-incompatible in
Cargo, so it would never have arrived on its own.

### Other

- Dependency updated: `format198x-commodore-amiga-adf` 0.2.3 → 0.3.0. Its
  additions — a raw sector layer, high-density floppy support, an exhaustive
  `check` — are unused here.
- High-density images are still refused, but the reason has expired. `open`
  rejects a 1.76 MB image as `UnsupportedContainer`, on the grounds that it is
  real media this crate cannot read; as of this dependency it can. Wiring it up
  is a capability rather than a fix, so it is left for its own change.

## [0.1.0](https://github.com/play198x/play198x/releases/tag/play198x-core-v0.1.0) - 2026-08-26

### Added

- report what an interface shows about a work
- measure how long a module lasts by playing it silently
- run ProTracker's effects from its three dispatch tables
- play a module's samples at the right pitch and the right tempo
- decode Spectrum, C64 and Amiga images to RGBA
- identify a format from its bytes, and say how sure it is
- read Amiga disk images, decrunching PowerPacked entries in passing
- open plain files and ZIP archives as one kind of container
- scaffold the play198x-core library and its release pipeline

### Fixed

- swing tremolo the full depth ProTracker gives it
- report Art Studio as probable, because nothing downstream can catch a miss
- bound what opening a container allocates, and stop calling a refusal damage

### Other

- measure the engine against libxmp on derived measures
- narrow zip to the one deflate backend this crate reads with
