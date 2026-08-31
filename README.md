# Play198x

The retro media player/viewer for the 198x family — plays and previews vintage media without booting a whole machine.

Audio (SID, AY/YM, tracker formats), images (IFF/ILBM, C64 koala/hires, Spectrum SCR), and animation (Amiga ANIM, FLI/FLC). The boundary with [Emu198x](https://github.com/emu198x): Emu198x *executes programs* (boots a machine); Play198x *renders media* (a tune, image, or animation that isn't a bootable program).

## Status — the core and the web player are live

`play198x-core` is the published library underneath everything else: it
opens a retro media file from a plain path, a ZIP or an Amiga ADF, identifies
it from its bytes, and hands back an image or something playable. Containers,
PowerPacker decrunching, identification, image decoding, the ProTracker engine
and ZX Spectrum `.ay` plus ROM-free callable PSID playback are all in. AY and
SID have separate optional features, so an image-only consumer acquires no CPU
or sound chip.

`@play198x/web` is the WebAssembly build of that library, and
[play198x.github.io](https://play198x.github.io) is a page that *is* the
player: drop a file on it and it draws or plays in the browser. It reads ZX
Spectrum SCREEN$ and `.ay`, Commodore 64 Koala Paint, Art Studio and PSID, Amiga IFF
ILBM, and ProTracker MOD. An `.ay` tune's own Z80 code runs against Emu198x's
published CPU and AY crates on a ROM-free 128K Spectrum host, including its
beeper, subtunes, memory paging and AY register reads. A callable PSID's 6502
code runs against Emu198x's SID core; RSID, self-driven, multi-SID and
ROM-dependent tunes are identified and explicitly declined.
Open replacement ROMs do not extend that callable slice; the measured benefit
does not justify a firmware distribution surface or output that cannot be
claimed compatible with an original C64. See the
[decision record](decisions/open-roms-do-not-extend-callable-sid-playback.md).
Self-driven SID remains the emulator's job: RSID and zero-play-address PSID
require continuously scheduled C64 interrupt hardware, not another callable
media routine. The [self-driven SID decision](decisions/self-driven-sid-belongs-to-a-c64-emulator.md)
records the corpus and reference-player evidence.

There is no desktop shell yet. It comes after the web one and is deliberately
thin, the same way: anything a shell can do is an operation the core exposes,
which is what makes a scriptable surface a later addition rather than a
retrofit.

Design lives in [`play198x/docs`](https://github.com/play198x/docs): the
[core design](https://github.com/play198x/docs/blob/main/specs/2026-08-25-data-driven-core-design.md)
and the [website design](https://github.com/play198x/docs/blob/main/specs/2026-08-26-website-design.md).

A guiding rule: Play198x is a **thin consumer of Emu198x's chip and CPU cores** (SID/AY/Paula/VIC, 6502/Z80) for the formats that need a player, and decodes pure-data formats (IFF/SCR images) directly. It never reimplements chip emulation.

## Building and checking

The web shell is deliberately excluded from the root Cargo workspace, so a
plain workspace command does not cover the whole repository. Run
`scripts/check` to apply the same format, lint, test, lockfile and WebAssembly
build contract as CI to both Cargo trees. It requires `wasm-pack` and the
`wasm32-unknown-unknown` Rust target.

Before committing, `scripts/check prepare` formats both trees and refreshes the
web shell's path-dependency lock entries without updating unrelated transitive
dependencies. Individual CI-sized phases are listed by `scripts/check --help`.

Play198x is the media player/viewer sibling in the 198x family. Its boundary is governed by `198x/decisions/play198x-media-player.md`.
