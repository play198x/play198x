# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
