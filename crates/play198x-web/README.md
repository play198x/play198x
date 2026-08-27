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

const canvas = document.querySelector('canvas');
canvas.width = image.width;
canvas.height = image.height;
canvas.getContext('2d').putImageData(
  new ImageData(new Uint8ClampedArray(image.rgba), image.width, image.height),
  0, 0,
);
```

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
```

A plain file — not a ZIP, not an ADF — is a `Container` of exactly one entry,
named by `file.name`. `entry_path`/`entry_len` return `undefined` past the last
index, and `read` throws when no entry answers to the name given.

**Check `file.size` before this.** `Container`'s constructor takes ownership of
the bytes you hand it, so it cannot undo an oversized `Uint8Array` your own code
already allocated to build them. Refuse a huge `File` before reading it, rather
than after.

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
