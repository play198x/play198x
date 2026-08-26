# Play198x

The retro media player/viewer for the 198x family — plays and previews vintage media without booting a whole machine.

Audio (SID, AY/YM, tracker formats), images (IFF/ILBM, C64 koala/hires, Spectrum SCR), and animation (Amiga ANIM, FLI/FLC). The boundary with [Emu198x](https://github.com/emu198x): Emu198x *executes programs* (boots a machine); Play198x *renders media* (a tune, image, or animation that isn't a bootable program).

## Status — the core is being built

`play198x-core` is the published library underneath everything else: it opens a
retro media file from a plain path, a ZIP or an Amiga ADF, identifies it from
its bytes, and hands back an image or a module. Containers and PowerPacker
decrunching are in; identification, decoding and the ProTracker engine are next.

The desktop and web shells come after the core works, and are deliberately thin
— anything a shell can do is an operation the core exposes, which is what makes
a scriptable surface a later addition rather than a retrofit.

Design lives in [`play198x/docs`](https://github.com/play198x/docs): the
[core design](https://github.com/play198x/docs/blob/main/specs/2026-08-25-data-driven-core-design.md)
and the [website design](https://github.com/play198x/docs/blob/main/specs/2026-08-26-website-design.md).

A guiding rule: Play198x is a **thin consumer of Emu198x's chip and CPU cores** (SID/AY/Paula/VIC, 6502/Z80) for the formats that need a player, and decodes pure-data formats (IFF/SCR images) directly. It never reimplements chip emulation.

Play198x is the future media player/viewer sibling in the 198x family. Its boundary is governed by [`../../decisions/play198x-media-player.md`](../../decisions/play198x-media-player.md).
