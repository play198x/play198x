# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **the `ay` host has the 128K Spectrum's memory.** It had been claiming to be
  a 128K in its frame length and its AY clock while its RAM was a flat 64 KB
  with no banking, so a tune that paged memory ran code the host quietly threw
  away. There are now eight 16 KB RAM banks: bank 5 fixed at `$4000`, bank 2 at
  `$8000`, and port `$7FFD` choosing which bank shows through `$C000-$FFFF`.
  The latch is decoded as the hardware decodes it — A15 and A1 low, so `$5FFD`
  and `$3FFD` reach it too — and bit 5 locks paging for the rest of the song,
  there being no reset short of starting the next one. `$0000-$3FFF` stays RAM
  whatever the ROM-select bit asks for: there is no ROM to page in, and that
  region holds the `RET` stub the `.ay` format's player is required to supply.
  An `.ay` block names an address and never a bank, so the file's image of
  `$C000-$FFFF` is what every pageable bank starts with — otherwise a tune that
  pages would find its own code gone.
- **`IN` from the AY's register port returns the selected register.** It used
  to answer `0xFF`, which is the right answer for a port with nothing behind it
  and the wrong one for the port the sound chip is on: it tells a tune probing
  for the chip that there is no chip. Every other port still reads as the
  unattached bus it is.

### Changed

- **Breaking: `AyPlayer::fffd_read` and `AyPlayer::fffd_read_would_differ` are
  now `ay_read` and `ay_reads_non_ff`.** Both are corpus-sweep instrumentation
  that no production code reads. The old pair counted what a read *would* have
  returned had the chip been asked; the chip is asked now, so the counter
  reports what reads actually got.

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
