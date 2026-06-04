# Play198x

The retro media player/viewer for the [198x family](https://github.com/play198x) — plays and previews vintage media without booting a whole machine.

Audio (SID, AY/YM, tracker formats), images (IFF/ILBM, C64 koala/hires, Spectrum SCR), and animation (Amiga ANIM, FLI/FLC). The boundary with [Emu198x](https://github.com/emu198x): Emu198x *executes programs* (boots a machine); Play198x *renders media* (a tune, image, or animation that isn't a bootable program).

## Status — decided, not yet started

This repository is a placeholder. Play198x has no near-term pull, so the decision to pursue it is made and the work waits for a concrete need — curriculum media previews, or a preview surface for Cat198x's catalogue.

A guiding rule: Play198x is a **thin consumer of Emu198x's chip and CPU cores** (SID/AY/Paula/VIC, 6502/Z80) for the formats that need a player, and decodes pure-data formats (IFF/SCR images) directly. It never reimplements chip emulation — the same thin-consumer rule Forge198x lives under.

Seventh sibling of the 198x family, alongside Code198x, Emu198x, Asm198x, Cat198x, Forge198x, and Build198x.
