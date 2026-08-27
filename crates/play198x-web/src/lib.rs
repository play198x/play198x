//! A `wasm-bindgen` shell over `play198x-core`.
//!
//! The surface is deliberately dumb: bytes in, data out. No PNG encoding and no
//! canvas work happens here, because the build-time consumer wants PNG bytes and
//! a browser consumer wants `putImageData` — and neither may grow logic the
//! other would have to reimplement.

use play198x_core::probe::{Confidence, Format};
use wasm_bindgen::prelude::*;

/// What `probe` found.
#[wasm_bindgen]
pub struct Probed {
    format: String,
    confidence: String,
}

#[wasm_bindgen]
impl Probed {
    /// One of `scr`, `koala`, `art-studio`, `ilbm`, `protracker`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn format(&self) -> String {
        self.format.clone()
    }

    /// `certain` or `probable`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn confidence(&self) -> String {
        self.confidence.clone()
    }
}

/// The format's stable name across the boundary.
///
/// A string, not a discriminant: `Format` is `#[non_exhaustive]`, so a number
/// would shift silently when the core gains a format — and a build-time decode
/// that picks the wrong decoder produces a plausible wrong picture rather than
/// an error.
fn format_name(format: Format) -> &'static str {
    match format {
        Format::Scr => "scr",
        Format::Koala => "koala",
        Format::ArtStudio => "art-studio",
        Format::Ilbm => "ilbm",
        Format::ProTracker => "protracker",
        // `Format` is #[non_exhaustive]: a new variant must be named here
        // before it can cross, rather than crossing as something wrong.
        _ => "unknown",
    }
}

/// Identify `bytes`. Returns `null` in JavaScript when nothing matches.
#[wasm_bindgen]
#[must_use]
pub fn probe(bytes: &[u8]) -> Option<Probed> {
    let (format, confidence) = play198x_core::probe::identify(bytes)?;
    Some(Probed {
        format: format_name(format).to_owned(),
        confidence: match confidence {
            Confidence::Certain => "certain",
            Confidence::Probable => "probable",
        }
        .to_owned(),
    })
}

/// A decoded picture, flattened for JavaScript.
#[wasm_bindgen]
#[derive(Debug)]
pub struct DecodedImage {
    inner: play198x_core::decode::Image,
}

#[wasm_bindgen]
impl DecodedImage {
    /// Width in mode pixels — not display pixels. See `pixel_aspect_w`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn height(&self) -> u32 {
        self.inner.height
    }

    /// Row-major RGBA8, `width * height * 4` bytes, alpha always opaque.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn rgba(&self) -> Vec<u8> {
        self.inner.rgba.clone()
    }

    /// Horizontal component of one mode pixel's shape, against the machine's
    /// own single-width pixel. A consumer that ignores this draws a C64
    /// multicolour picture at half its real width.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn pixel_aspect_w(&self) -> u32 {
        self.inner.pixel_aspect.0
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn pixel_aspect_h(&self) -> u32 {
        self.inner.pixel_aspect.1
    }

    /// The picture's colours in hardware index order, flattened to RGB triples.
    ///
    /// Crosses the boundary even though the build-time consumer draws none of
    /// it: it cannot be recovered from the pixels afterwards — a picture that
    /// never uses colour 5 has lost it — and the palette view is the first
    /// interactive figure anyone will ask for.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn palette(&self) -> Vec<u8> {
        self.inner
            .palette
            .iter()
            .flat_map(|&(r, g, b)| [r, g, b])
            .collect()
    }
}

/// Parse a format name from the boundary back into the core's enum.
fn format_from_name(name: &str) -> Option<Format> {
    match name {
        "scr" => Some(Format::Scr),
        "koala" => Some(Format::Koala),
        "art-studio" => Some(Format::ArtStudio),
        "ilbm" => Some(Format::Ilbm),
        "protracker" => Some(Format::ProTracker),
        _ => None,
    }
}

/// Decode `bytes` as `format`, which is one of the names [`probe`] returns.
///
/// # Errors
///
/// When `format` is not a name this shell knows, or when the core's decoder
/// rejects the bytes — carrying the core's own message unchanged.
#[wasm_bindgen]
pub fn decode_image(bytes: &[u8], format: &str) -> Result<DecodedImage, JsError> {
    let Some(format) = format_from_name(format) else {
        return Err(JsError::new(&format!(
            "`{format}` is not a format this build knows"
        )));
    };

    play198x_core::decode::image(bytes, format)
        .map(|inner| DecodedImage { inner })
        .map_err(|err| JsError::new(&err.to_string()))
}

/// What a decoded picture says about itself, flattened for JavaScript.
///
/// Exists so a shell never has to re-derive this from [`DecodedImage`]'s raw
/// fields. Before this method existed, `@play198x/web`'s one browser
/// consumer mapped `format` to a display label, combined `width`/`height`
/// into a dimensions string, and rendered `palette` as swatches — in
/// JavaScript, a second copy of logic that already lives in
/// `play198x_core::metadata::image_meta`. A shell is bytes in, data out; the
/// site's copy could only ever drift from this one, so this method replaces
/// it rather than sitting beside it.
#[wasm_bindgen]
#[derive(Debug)]
pub struct ImageMeta {
    inner: play198x_core::metadata::ImageMeta,
}

#[wasm_bindgen]
impl ImageMeta {
    /// One of `scr`, `koala`, `art-studio`, `ilbm`, `protracker` — the same
    /// names [`probe`] and [`decode_image`] use.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn format(&self) -> String {
        format_name(self.inner.format).to_owned()
    }

    /// Width in mode pixels — not display pixels. See `DecodedImage`'s
    /// `pixel_aspect_w`/`pixel_aspect_h` for the shape of one mode pixel.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn width(&self) -> u32 {
        self.inner.width
    }

    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn height(&self) -> u32 {
        self.inner.height
    }

    /// The picture's colours in hardware index order, flattened to RGB
    /// triples — identical in shape to [`DecodedImage::palette`]. Repeated
    /// here so a caller that only wants the metadata (a file list, a
    /// thumbnail strip) is never forced to keep the whole decoded image
    /// around just to read its swatches.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn palette(&self) -> Vec<u8> {
        self.inner
            .palette
            .iter()
            .flat_map(|&(r, g, b)| [r, g, b])
            .collect()
    }

    /// The container path the bytes came from — caller-supplied (an
    /// [`Entry::path`](play198x_core::container::Entry::path), or a plain
    /// file's own name) and passed through unchanged. Not sanitised: see
    /// that field's own warning before using it as anything other than a
    /// display string.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn source(&self) -> String {
        self.inner.source.clone()
    }
}

#[wasm_bindgen]
impl DecodedImage {
    /// What this picture says about itself, for `source` — the path or name
    /// it was read from, since a decoded image does not know that on its own.
    #[must_use]
    pub fn metadata(&self, source: &str) -> ImageMeta {
        ImageMeta {
            inner: play198x_core::metadata::image_meta(&self.inner, source),
        }
    }
}

/// One entry inside an opened [`Container`], flattened for JavaScript.
///
/// [`Container::entry_path`] and [`Container::entry_len`] hand these fields
/// back individually rather than as a struct: `wasm-bindgen` cannot return a
/// `Vec` of a `#[wasm_bindgen]` type without `js-sys`, which this crate does
/// not depend on, so index-based accessors are what the boundary can express
/// without a new dependency.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Container {
    inner: play198x_core::container::Container,
    // Computed once, in `new`, rather than on every accessor call: a ZIP's
    // `entries()` re-parses its whole central directory, and an ADF's walks
    // the disk's directory tree. Caching means a visitor clicking through an
    // 880K disk's tunes pays that cost once, at open, not once per click.
    entries: Vec<play198x_core::container::Entry>,
}

#[wasm_bindgen]
impl Container {
    /// Open `bytes` — a plain file, a ZIP archive, or an Amiga disk image,
    /// decided from the bytes themselves, exactly as
    /// [`play198x_core::container::Container::from_bytes`] decides it.
    /// `name` is the browser's `File.name`; it becomes the sole entry's name
    /// if the bytes turn out to be a plain file, and is passed through
    /// unsanitised — see [`Entry::path`](play198x_core::container::Entry::path).
    ///
    /// Parses the archive once, here, and keeps both the opened container and
    /// its entry list resident for the methods below. The alternative —
    /// two free functions, `open_container(bytes)` and
    /// `read_entry(bytes, index)` — would force every entry read to re-send
    /// the whole archive's bytes across the `wasm-bindgen` boundary and
    /// re-validate them from scratch, which is the wrong cost to pay on every
    /// click through a disk's tunes. This struct pays the copy-in and the
    /// parse once, at construction, and every method after that is `&self`.
    ///
    /// Ownership: `bytes: Vec<u8>` is copied out of the JavaScript
    /// `Uint8Array` by `wasm-bindgen` before this function runs, so the
    /// `Vec` — and the [`play198x_core::container::Container`] built over it
    /// — is owned outright. Nothing here borrows from JavaScript memory, so
    /// there is nothing that can dangle if the caller's buffer is dropped,
    /// detached, or reused the moment this call returns.
    ///
    /// # Errors
    ///
    /// When the bytes are too large, or turn out to be a damaged or
    /// unsupported archive or disk image — carrying the core's own message
    /// unchanged.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: Vec<u8>, name: &str) -> Result<Container, JsError> {
        let inner = play198x_core::container::Container::from_bytes(bytes, name)
            .map_err(|err| JsError::new(&err.to_string()))?;
        let entries = inner
            .entries()
            .map_err(|err| JsError::new(&err.to_string()))?;
        Ok(Self { inner, entries })
    }

    /// How many entries the container holds.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn entry_count(&self) -> u32 {
        // usize is 32-bit on the wasm32 target this crate builds for, and the
        // core's own caps (an archive of at most 64 MiB; a disk of at most
        // `MAX_DISK_ENTRIES` headers) keep the real count far below `u32::MAX`
        // regardless.
        self.entries.len() as u32
    }

    /// The entry's name at `index`, exactly as the container states it and
    /// **not sanitised** — see
    /// [`Entry::path`](play198x_core::container::Entry::path). `undefined`
    /// in JavaScript once `index` reaches [`Self::entry_count`].
    #[must_use]
    pub fn entry_path(&self, index: u32) -> Option<String> {
        self.entries
            .get(index as usize)
            .map(|entry| entry.path.clone())
    }

    /// How many bytes reading the entry at `index` yields, before any
    /// PowerPacker decrunching [`Self::read`] does on the way out.
    /// `undefined` in JavaScript once `index` reaches [`Self::entry_count`].
    ///
    /// A `number`, not the exact byte count past 2^53: JavaScript has no
    /// 64-bit integer here, and the core's own per-entry cap
    /// (`MAX_ENTRY_LEN`, 16 MiB) sits so far under that ceiling this can
    /// never lose precision on anything this crate will actually read.
    #[must_use]
    pub fn entry_len(&self, index: u32) -> Option<f64> {
        self.entries
            .get(index as usize)
            .map(|entry| entry.len as f64)
    }

    /// Read one entry's bytes by name, decrunched if it arrived PowerPacked.
    ///
    /// # Errors
    ///
    /// When no entry answers to `path`, or the container turns out to be
    /// damaged in a way only a read discovers — carrying the core's own
    /// message unchanged.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, JsError> {
        self.inner
            .read(path)
            .map_err(|err| JsError::new(&err.to_string()))
    }
}

/// A playing ProTracker module, wrapped for an `AudioWorkletProcessor`.
///
/// The worklet calls [`Self::render`] roughly every 2.7 ms with a 128-sample
/// buffer it owns — see the spike report this crate's plan cites. That shape
/// is why `render` takes a caller-owned `&mut [f32]` rather than returning a
/// `Vec`: the core's [`play198x_core::engine::Engine::render`] is
/// allocation-free by design (`play198x-core`'s counting-allocator test holds
/// it to that), and an audio callback that allocates is one that eventually
/// glitches on somebody else's machine. This shell adds nothing per call
/// either — it forwards straight into the engine's own buffer.
#[wasm_bindgen]
pub struct ModulePlayer {
    engine: play198x_core::engine::Engine,
}

#[wasm_bindgen]
impl ModulePlayer {
    /// Decode `bytes` as a ProTracker module and start it playing at
    /// `sample_rate`.
    ///
    /// `bytes` is only borrowed for the parse: [`play198x_core::decode::module`]
    /// copies everything it needs (sample PCM included) into the returned
    /// `Module`, so nothing here holds a reference into the caller's buffer
    /// past this call.
    ///
    /// # Errors
    ///
    /// When the bytes are not a 4-channel ProTracker module — carrying the
    /// core's own message unchanged.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], sample_rate: u32) -> Result<ModulePlayer, JsError> {
        let module =
            play198x_core::decode::module(bytes).map_err(|err| JsError::new(&err.to_string()))?;
        Ok(Self {
            engine: play198x_core::engine::Engine::new(module, sample_rate),
        })
    }

    /// Fill `out` with interleaved stereo samples, returning how many it
    /// wrote — always `out.len()` rounded down to an even count, whether
    /// playing or paused.
    ///
    /// A paused player still fills `out`, with silence rather than a short
    /// count: the engine renders exact zeroes for a paused transport (see
    /// [`play198x_core::engine::Engine::render`]'s own doc), and a worklet
    /// callback that gets fewer samples than it asked for is a worklet
    /// callback that clicks. `out.len()` in, an even number back — the core
    /// works in whole stereo frames, so an odd trailing sample is left
    /// untouched and not counted.
    pub fn render(&mut self, out: &mut [f32]) -> usize {
        self.engine.render(out) * 2
    }

    /// Start or pause playback. A paused player keeps its position and its
    /// clock — see [`Self::render`] — so resuming continues the row it
    /// stopped in rather than restarting the song.
    pub fn set_playing(&mut self, playing: bool) {
        self.engine.set_playing(playing);
    }

    /// Jump to the top of an order, clamped to the song's played prefix.
    /// Cuts any sounding notes, the way a listener dragging a scrub bar
    /// expects.
    pub fn seek_order(&mut self, order: usize) {
        self.engine.seek_order(order);
    }

    /// Index into the order table's played prefix.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn order(&self) -> usize {
        self.engine.position().order
    }

    /// The pattern the current order names.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn pattern(&self) -> usize {
        self.engine.position().pattern
    }

    /// Row within the current pattern, `0..64`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn row(&self) -> usize {
        self.engine.position().row
    }

    /// Tick within the current row, `0..speed`.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn tick(&self) -> u8 {
        self.engine.position().tick
    }
}
