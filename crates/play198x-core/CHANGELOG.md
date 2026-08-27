# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/play198x/play198x/compare/play198x-core-v0.1.0...play198x-core-v0.1.1) - 2026-08-27

### Fixed

- *(deps)* take the ADF fix that stops ordinary data disks reading as corrupt ([#7](https://github.com/play198x/play198x/pull/7))

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
