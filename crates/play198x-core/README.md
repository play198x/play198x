# play198x-core

Open, identify and render retro media: Sinclair Spectrum, Commodore 64 and
Commodore Amiga images, ProTracker modules, Spectrum `.ay` tunes, and callable
ROM-free PSID tunes (playback behind the `ay` and `sid` features).

The library takes a path, works out what the bytes are, and hands back either an
RGBA image or a module you can play. It is the engine underneath the Play198x
player and viewer, and it is published so the other 198x projects can use it
directly.

## What it does

- **Opens three container shapes** — a plain file, a ZIP archive, and an Amiga
  ADF disk image — behind one interface, so the rest of the crate never cares
  where bytes came from.
- **Identifies a format from its bytes**, never from the file extension, and
  says how confident it is. A 6912-byte file is only *probably* a Spectrum
  screen; a ProTracker module's magic number is certain.
- **Decodes images to RGBA** and modules to a playable song.
- **Plays a module** through a pull-based engine that never owns an audio
  device: you ask it for frames, and it fills your buffer without allocating.

Decoding itself is delegated to the [Format198x](https://github.com/format198x)
crates, and palettes come from `mediaspec198x`. This crate holds the glue, the
palette lookups those crates deliberately do not do, and the ProTracker engine.

## What it does not do

No user interface, no audio device, no async. NSF and SAP are not here. AY and
SID are the code-driven exceptions:
identifying an `.ay` needs no chip at all (see "What it does"), and playing
one — a virtual 128K Spectrum running the tune's own Z80 code, driving an
AY-3-8910 — lives here behind the optional `ay` feature. Callable PSID uses
Emu198x's published 6502 and SID cores behind `sid`; both are off by default so
a picture consumer acquires neither CPU nor sound chip.
Animation formats are not here either.

## Design constraints

These hold from the first commit because the crate is destined for an FFI
boundary and for thumbnailers running inside other people's processes:

- No panic is reachable from any container, probe or decode path. Errors are
  typed; `unsafe_code` is forbidden.
- No global mutable state, no lazy global initialisation, and no assumption of
  running on a main thread.
- Rendering audio allocates nothing.

## Status

Early. The public surface is settling and will break before 1.0.

## Licence

GPL-2.0-or-later.
