# Play198x

The retro media player/viewer for the 198x family — plays and previews vintage media without booting a whole machine.

Audio (SID, AY/YM, tracker formats), images (IFF/ILBM, C64 koala/hires, Spectrum SCR), and animation (Amiga ANIM, FLI/FLC). The boundary with [Emu198x](https://github.com/emu198x): Emu198x *executes programs* (boots a machine); Play198x *renders media* (a tune, image, or animation that isn't a bootable program).

## Status — the core and the web player are live

`play198x-core` is the published library underneath everything else: it opens a
retro media file from a plain path, a ZIP or an Amiga ADF, identifies it from
its bytes, and hands back an image or a module. Containers, PowerPacker
decrunching, identification, image decoding, the ProTracker engine and the
timing walk are all in.

`@play198x/web` is the WebAssembly build of that library, and
[play198x.github.io](https://play198x.github.io) is a page that *is* the
player: drop a file on it and it draws or plays in the browser. It reads ZX
Spectrum SCREEN$, Commodore 64 Koala Paint and Art Studio, Amiga IFF ILBM and
ProTracker MOD.

ZX Spectrum `.ay` chiptunes play too, behind `play198x-core`'s optional `ay`
feature: the tune's own Z80 code runs against Emu198x's published CPU and AY
crates, on a host this crate supplies, and **no ROM is involved**. The feature
is off by default, so a consumer decoding a SCREEN$ acquires no Z80 to do it.
The web player does not expose it yet — four more crates enter the `.wasm` the
page fetches, and that cost should be measured before it does. SID is the next
slice.

There is no desktop shell yet. It comes after the web one and is deliberately
thin, the same way: anything a shell can do is an operation the core exposes,
which is what makes a scriptable surface a later addition rather than a
retrofit.

Design lives in [`play198x/docs`](https://github.com/play198x/docs): the
[core design](https://github.com/play198x/docs/blob/main/specs/2026-08-25-data-driven-core-design.md)
and the [website design](https://github.com/play198x/docs/blob/main/specs/2026-08-26-website-design.md).

A guiding rule: Play198x is a **thin consumer of Emu198x's chip and CPU cores** (SID/AY/Paula/VIC, 6502/Z80) for the formats that need a player, and decodes pure-data formats (IFF/SCR images) directly. It never reimplements chip emulation.

Play198x is the media player/viewer sibling in the 198x family. Its boundary is governed by `198x/decisions/play198x-media-player.md`.
