# Play198x

The retro media player/viewer for the 198x family — plays and previews vintage media without booting a whole machine.

Audio (SID, AY/YM, tracker formats), images (IFF/ILBM, C64 koala/hires, Spectrum SCR), and animation (Amiga ANIM, FLI/FLC). The boundary with [Emu198x](https://github.com/emu198x): Emu198x *executes programs* (boots a machine); Play198x *renders media* (a tune, image, or animation that isn't a bootable program).

## Status — not yet started

Play198x has no near-term pull. This repo currently holds the future media player/viewer implementation space until a concrete need appears — curriculum media previews, or a preview surface for Cat198x's catalogue.

A guiding rule: Play198x is a **thin consumer of Emu198x's chip and CPU cores** (SID/AY/Paula/VIC, 6502/Z80) for the formats that need a player, and decodes pure-data formats (IFF/SCR images) directly. It never reimplements chip emulation.

Play198x is the future media player/viewer sibling in the 198x family. Its boundary is governed by [`../../decisions/play198x-media-player.md`](../../decisions/play198x-media-player.md).
