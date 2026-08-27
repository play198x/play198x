# @play198x/web

Decode retro computer image files and play tracker music, in the browser or in a
build step. WebAssembly, no dependencies, nothing uploaded anywhere.

This is the browser build of [`play198x-core`](https://crates.io/crates/play198x-core),
the library behind [Play198x](https://github.com/play198x/play198x) — a media
player for vintage formats.

## What it reads

| Format | Machine |
|---|---|
| SCREEN$ (`.scr`) | ZX Spectrum |
| Koala Painter | Commodore 64 |
| Advanced Art Studio | Commodore 64 |
| ILBM / IFF | Commodore Amiga |
| ProTracker module (`.mod`) | Commodore Amiga |

Files may be plain, inside a ZIP, or inside an Amiga ADF disk image — PowerPacked
entries are decrunched in passing.

## Use

```js
import init, { probe, decode_image } from '@play198x/web';

await init();

const bytes = new Uint8Array(await file.arrayBuffer());
const found = probe(bytes);
if (!found) throw new Error('not a format this recognises');

const image = decode_image(bytes, found.format);
found.free();

const canvas = document.querySelector('canvas');
canvas.width = image.width;
canvas.height = image.height;
canvas.getContext('2d').putImageData(
  new ImageData(new Uint8ClampedArray(image.rgba), image.width, image.height),
  0, 0,
);
image.free();
```

`free()` isn't cleanup you can skip. Every object this package hands back —
`Probed`, `DecodedImage`, `ImageMeta`, `Container`, `ModulePlayer` — holds
`wasm-bindgen`-managed memory on the wasm side, and JavaScript's garbage
collector cannot see it: it only knows about the small JS object wrapping a
pointer, not what that pointer holds in linear memory. Call `free()` once you
are done with a value, or that memory sits until the page unloads.

### Metadata, not a reimplementation of it

`DecodedImage.metadata(source)` reports the same facts a file browser or a
thumbnail strip wants — format, dimensions, palette — plus the `source` string
you pass it, for display. Read it from here rather than re-deriving a label,
a dimensions string, or swatches from `width`/`height`/`format`/`palette`
yourselves: this is the one place that logic is allowed to live, and a second
copy can only drift from it.

```js
const image = decode_image(bytes, found.format);
const meta = image.metadata(file.name);

console.log(meta.format, `${meta.width}×${meta.height}`, meta.source);
meta.free();
image.free();
```

### Opening archives and disk images

A visitor drops a ZIP or an Amiga ADF disk image, not a single file. `Container`
opens it once and keeps it open, so clicking through several entries — a music
disk's tunes — never re-sends or re-parses the whole archive per click:

```js
import init, { Container } from '@play198x/web';

await init();

const bytes = new Uint8Array(await file.arrayBuffer());
const container = new Container(bytes, file.name);

for (let i = 0; i < container.entry_count; i++) {
  console.log(container.entry_path(i), container.entry_len(i));
}

const moduleBytes = container.read('mod.title_tune');
container.free();
```

A plain file — not a ZIP, not an ADF — is a `Container` of exactly one entry,
named by `file.name`. `entry_path`/`entry_len` return `undefined` past the last
index, and `read` throws when no entry answers to the name given.

Call `free()` on it the same as any other value from this package — and mean
it here especially: a `Container` is sized for up to 64 MiB, and a visitor
opening several archives in a session (the music-disk case above) leaks one
whole archive's worth of wasm memory per `Container` left unfreed.

**Check `file.size` before this.** `Container`'s constructor takes ownership of
the bytes you hand it, so it cannot undo an oversized `Uint8Array` your own code
already allocated to build them. Refuse a huge `File` before reading it, rather
than after.

### Playing a module

`ModulePlayer` wraps a decoded ProTracker module and a running transport. It is
built for an `AudioWorkletProcessor`: construct it once inside the worklet,
then call `render` from `process()` with the buffer Web Audio hands you —
128 samples, called roughly every 2.7 ms at 48 kHz. `render` never allocates,
so it is safe on that callback.

```js
import init, { ModulePlayer } from '@play198x/web';

await init();

const bytes = new Uint8Array(await file.arrayBuffer());
const player = new ModulePlayer(bytes, sampleRate);

// Inside AudioWorkletProcessor.process(inputs, outputs):
const channel = outputs[0][0]; // mono view; see the note below on layout
player.render(channel);

// Later, from the main thread via a message to the worklet:
player.set_playing(false); // pauses; render keeps filling with silence
player.seek_order(4); // jumps to order 4, clamped to the song's length

console.log(player.order, player.pattern, player.row, player.tick);

player.free();
```

`render` always fills the buffer you pass it, playing or paused — a paused
player writes silence rather than returning fewer samples than it was asked
for, because a starved Web Audio callback clicks. Pause and resume hold the
song's position and its clock, so resuming continues the row it stopped in.

`render`'s buffer is interleaved stereo (`[left, right, left, right, ...]`),
matching the engine's own output; a worklet's `process()` callback deals in
per-channel arrays, so the site wiring the two together is what reshapes one
into the other — that reshaping is site logic, not this shell's job.

### Two things worth knowing

**`confidence` is not decoration.** `probe` returns `certain` or `probable`. A
ZX Spectrum SCREEN$ has no magic number — its length, exactly 6912 bytes, is the
entire signal — so it is always `probable`. If you render a `probable` result
without saying so, a file that happens to be 6912 bytes renders as garbage with
nothing to tell anyone.

**Mode pixels are not display pixels.** `pixel_aspect_w` and `pixel_aspect_h`
describe the shape of one pixel against the machine's own square one. A C64
multicolour bitmap is 160×200 mode pixels at 2:1 — draw it 160 wide and you have
halved it. Set the canvas buffer from `width`/`height` and its CSS size from the
aspect.

## Licence

GPL-2.0-or-later.
