//! A `wasm-bindgen` shell over `play198x-core`.
//!
//! The surface is deliberately dumb: bytes in, data out. No PNG encoding and no
//! canvas work happens here, because the build-time consumer wants PNG bytes and
//! a browser consumer wants `putImageData` — and neither may grow logic the
//! other would have to reimplement.

use play198x_core::probe::{Confidence, Format};
use wasm_bindgen::prelude::*;

/// Frames [`Player::render`] fills per call.
///
/// The Web Audio render quantum: fixed by the Web Audio spec at 128 samples,
/// and confirmed by the spike this crate's plan cites (128 samples at 48 kHz,
/// called roughly every 2.7 ms, 0 glitches across 45,750 callbacks).
/// [`Player`]'s per-channel buffers are sized to exactly this many
/// frames, once, at construction, and nothing afterwards grows them — which
/// is what lets a caller build a `Float32Array` view over them and reuse
/// that view across calls instead of re-acquiring it every render.
const RENDER_QUANTUM: usize = 128;

/// What `probe` found.
#[wasm_bindgen]
pub struct Probed {
    format: String,
    confidence: String,
}

#[wasm_bindgen]
impl Probed {
    /// One of `scr`, `koala`, `art-studio`, `ilbm`, `protracker`, `ay`.
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
        // `Format::Ay` is not behind the core's `ay` feature (identifying
        // one needs no Z80), and this crate does not enable `ay` — so
        // `probe` can return `Format::Ay` in every build of this shell, and
        // naming it "ay" here is what stops that case from crossing as
        // "unknown". Reporting certainty about an unidentified format would
        // be the exact silent failure the core's identify() was changed not
        // to produce; a wildcard arm here would just move that failure to
        // the boundary.
        Format::Ay => "ay",
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
///
/// `"protracker"` already mapped to `Some(Format::ProTracker)` although this
/// shell has no picture decoder for a module — `decode_image` still calls
/// `play198x_core::decode::image`, and that refuses a `ProTracker` with a
/// named reason ("a ProTracker module is music, not a picture; use
/// `decode::module`") rather than this function claiming the name is not
/// even recognised. `"ay"` follows the same precedent rather than falling
/// to the `None` arm below: an unhandled name is right for a name this
/// build has genuinely never seen, but `"ay"` is a name `probe` itself can
/// hand back, so refusing it as unrecognised would be false. Routing it
/// through means the caller gets the core's own accurate refusal ("an .ay
/// tune is code for a Z80 to run, not a picture; use `player::ay`") instead
/// of this shell's generic "not a format this build knows".
fn format_from_name(name: &str) -> Option<Format> {
    match name {
        "scr" => Some(Format::Scr),
        "koala" => Some(Format::Koala),
        "art-studio" => Some(Format::ArtStudio),
        "ilbm" => Some(Format::Ilbm),
        "protracker" => Some(Format::ProTracker),
        "ay" => Some(Format::Ay),
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

/// What a module says about itself: its title, its shape, every sample name,
/// and how long it plays.
///
/// Built from bytes rather than from a [`Player`], and that is the
/// point. A player is constructed on the audio thread, inside the worklet,
/// after a visitor has asked for sound; a player's info panel needs to say
/// what the file is *before* that. So this reads the module the same way
/// [`probe`] does — on whatever thread is asking — and never touches the
/// playing engine.
#[wasm_bindgen]
#[derive(Debug)]
pub struct ModuleMeta {
    inner: play198x_core::metadata::ModuleMeta,
}

/// The rate the timing walk runs at.
///
/// It changes none of the numbers this type reports — `play198x_core`'s
/// `module_meta` states that a tick is `2500 / tempo` milliseconds whatever
/// the output rate is, and takes the rate only so the walk happens under the
/// same engine a caller would play with. Fixed here rather than asked for,
/// because a caller cannot supply a meaningful value for something that
/// cannot affect the answer, and one that had to invent a number would
/// reasonably assume it mattered.
const META_SAMPLE_RATE: u32 = 48_000;

/// Read a ProTracker module's metadata.
///
/// # Errors
///
/// When the bytes are not a 4-channel ProTracker module — carrying the core's
/// own message unchanged, as [`Player::new`] does.
#[wasm_bindgen(js_name = moduleMeta)]
pub fn module_meta(bytes: &[u8]) -> Result<ModuleMeta, JsError> {
    let module =
        play198x_core::decode::module(bytes).map_err(|error| JsError::new(&error.to_string()))?;
    Ok(ModuleMeta {
        inner: play198x_core::metadata::module_meta(&module, META_SAMPLE_RATE),
    })
}

#[wasm_bindgen]
impl ModuleMeta {
    /// The 20-byte title field, Latin-1, trimmed at its first NUL. Empty when
    /// the author left it empty, which is common.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn title(&self) -> String {
        self.inner.title.clone()
    }

    /// The four-byte magic as text — `M.K.`, `FLT4`, and the rest.
    #[wasm_bindgen(getter, js_name = formatTag)]
    #[must_use]
    pub fn format_tag(&self) -> String {
        self.inner.format_tag.clone()
    }

    /// Voices the magic names: 4 for the modules this crate decodes.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn channels(&self) -> u8 {
        self.inner.channels
    }

    /// Patterns the file physically holds, which is not the number the order
    /// table names — a module can carry patterns no position ever plays.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn patterns(&self) -> u32 {
        u32::try_from(self.inner.patterns).unwrap_or(u32::MAX)
    }

    /// Order-table entries the song plays: its `song_length` prefix, not the
    /// format's fixed 128.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn orders(&self) -> u32 {
        u32::try_from(self.inner.orders).unwrap_or(u32::MAX)
    }

    /// One name per sample slot, in slot order: **always 31 entries**,
    /// including slots holding no sample. Authors used the empty ones for
    /// greetings, credits and whole paragraphs, so the empty ones are often
    /// the interesting ones — see `play198x_core::metadata`'s own note.
    #[wasm_bindgen(getter, js_name = sampleNames)]
    #[must_use]
    pub fn sample_names(&self) -> Vec<String> {
        self.inner.sample_names.clone()
    }

    /// How long one pass lasts, in milliseconds: from the top of the order
    /// table to whichever comes first — the end of the song, an `F00` that
    /// stops it, or a position the song has already played.
    #[wasm_bindgen(getter, js_name = durationMs)]
    #[must_use]
    pub fn duration_ms(&self) -> f64 {
        self.inner.timing.duration.as_secs_f64() * 1_000.0
    }

    /// Whether the song returns to a position it has already played instead
    /// of running off the end of its order table. A looping module has no
    /// single length, so an interface that shows [`Self::duration_ms`] as an
    /// ending needs this to say otherwise.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn loops(&self) -> bool {
        self.inner.timing.loops
    }

    /// How far into [`Self::duration_ms`] the repeated position was first
    /// reached — the point playback comes back to. `undefined` when the song
    /// does not loop.
    #[wasm_bindgen(getter, js_name = loopStartMs)]
    #[must_use]
    pub fn loop_start_ms(&self) -> Option<f64> {
        self.inner
            .timing
            .loop_start
            .map(|start| start.as_secs_f64() * 1_000.0)
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

/// Whichever player the dropped file called for.
///
/// An enum rather than a boxed trait object, because not every operation is
/// shared: a module can be seeked to an order and a `.ay` cannot, and the
/// enum is what lets [`Player::seek_order`] say so instead of silently doing
/// nothing. Everything that *is* shared goes through
/// [`play198x_core::player::Player`].
///
/// Both variants are boxed, so the enum is a pointer whichever is playing and
/// neither one's size decides the other's. `Engine` measures at least 2,552
/// bytes inline while the `.ay` player keeps its Spectrum RAM on the heap
/// already, so leaving either unboxed makes every player pay for the larger —
/// and which one that is has flipped once during this design, which is the
/// argument for not depending on the answer. Both allocations happen once, at
/// construction, off the audio thread.
enum Inner {
    Module(Box<play198x_core::engine::Engine>),
    Ay(Box<play198x_core::player::pump::FramePump<play198x_core::player::ay::AyPlayer>>),
}

impl Inner {
    /// The shared half of the two, as the core's trait.
    fn as_player(&mut self) -> &mut dyn play198x_core::player::Player {
        match self {
            Inner::Module(engine) => engine.as_mut(),
            Inner::Ay(pump) => pump.as_mut(),
        }
    }
}

/// A playing tune, wrapped for an `AudioWorkletProcessor`.
///
/// The worklet calls [`Self::render`] roughly every 2.7 ms — see the spike
/// report this crate's plan cites. A first version of this type had `render`
/// take a caller-owned `&mut [f32]`, which reads as allocation-free in Rust
/// and is not: wasm-bindgen's generated glue for a mutable slice parameter
/// mallocs a buffer, copies the caller's array into it, calls the wasm
/// function, copies the result back out, and frees the buffer — every call,
/// on the audio-rendering thread. `play198x-core`'s counting-allocator test
/// only ever watched [`play198x_core::engine::Engine::render`] itself, one
/// layer below where that malloc happens, so it could not have caught this.
///
/// This shape avoids it structurally instead of by care: [`Self::render`]
/// takes no slice at all. It fills two buffers *this struct already owns*
/// and hands the caller a pointer into wasm linear memory
/// ([`Self::left_ptr`], [`Self::right_ptr`]) plus [`wasm_memory`] to build a
/// `Float32Array` view over it. A view is a window onto the same bytes, not
/// a copy — writing through it happens on the wasm side, reading it happens
/// on the JS side, and nothing crosses the boundary but two pointers (read
/// once) and a frame count (every call). See the package README for the
/// exact JS-side pattern, including the one caveat a view brings with it:
/// it detaches if the wasm module's memory grows.
#[wasm_bindgen]
pub struct Player {
    inner: Inner,
    /// Interleaved scratch the engine writes into — [`RENDER_QUANTUM`]
    /// stereo frames, sized once at construction and only ever borrowed as a
    /// slice afterwards, never resized. `Engine::render` wants one
    /// contiguous interleaved buffer; [`Self::render`] de-interleaves it
    /// into `left`/`right` below, which is what a Web Audio channel wants.
    interleaved: Vec<f32>,
    /// De-interleaved per-channel output, [`RENDER_QUANTUM`] frames each.
    /// Never reallocated after construction, which is what makes their
    /// addresses ([`Self::left_ptr`]/[`Self::right_ptr`]) stable for a
    /// caller to build a view over once and reuse.
    left: Vec<f32>,
    right: Vec<f32>,
}

#[wasm_bindgen]
impl Player {
    /// Frames [`Self::render`] fills per call, and the length of the buffers
    /// [`Self::left_ptr`]/[`Self::right_ptr`] point at. A plain function
    /// rather than a duplicated literal on the JavaScript side, so the two
    /// can never quietly disagree.
    #[wasm_bindgen(js_name = renderQuantum)]
    #[must_use]
    pub fn render_quantum() -> usize {
        RENDER_QUANTUM
    }

    /// Identify `bytes`, build the player the format calls for, and start it
    /// at `sample_rate`.
    ///
    /// `song` selects a subtune. A `.ay` carries a table of them — 278 of the
    /// 696 files in the local archive are multi-song — and each is a separate
    /// entry point with its own initial register state, which is why choosing
    /// one *constructs a player* rather than seeking an existing one. A
    /// module has no subtunes and ignores it.
    ///
    /// `bytes` is only borrowed: both decoders copy what they need, so
    /// nothing here holds a reference into the caller's buffer past this call.
    ///
    /// # Errors
    ///
    /// When the bytes are not a format this shell can play, or are a `.ay`
    /// whose song table has no entry `song` — carrying the core's own message
    /// unchanged, so the reason reaches the caller rather than a generic
    /// refusal.
    #[wasm_bindgen(constructor)]
    pub fn new(bytes: &[u8], song: usize, sample_rate: u32) -> Result<Player, JsError> {
        let inner = match play198x_core::probe::identify(bytes) {
            Some((play198x_core::probe::Format::Ay, _)) => {
                let ay = play198x_core::player::ay::AyPlayer::new(bytes, song, sample_rate)
                    .map_err(|err| JsError::new(&format!("{err:?}")))?;
                Inner::Ay(Box::new(play198x_core::player::pump::FramePump::new(ay)))
            }
            _ => {
                let module = play198x_core::decode::module(bytes)
                    .map_err(|err| JsError::new(&err.to_string()))?;
                Inner::Module(Box::new(play198x_core::engine::Engine::new(
                    module,
                    sample_rate,
                )))
            }
        };
        Ok(Self {
            inner,
            interleaved: vec![0.0; RENDER_QUANTUM * 2],
            left: vec![0.0; RENDER_QUANTUM],
            right: vec![0.0; RENDER_QUANTUM],
        })
    }

    /// Render into this player's own buffers, returning how many frames it
    /// actually wrote — read the result through [`Self::left_ptr`] and
    /// [`Self::right_ptr`], not a return value.
    ///
    /// `frames` past [`Self::render_quantum`] is clamped rather than grown
    /// into: the buffers are sized once, at construction, and this call
    /// never reallocates them, which is the property a cached `Float32Array`
    /// view depends on. A worklet always asks for exactly the render
    /// quantum, so this only clips a caller error — it does not fail loudly
    /// because nothing on this side of the FFI boundary is allowed to.
    ///
    /// A paused player still renders a full quantum of silence rather than
    /// fewer frames: the engine renders exact zeroes for a paused transport
    /// (see [`play198x_core::engine::Engine::render`]'s own doc), and a
    /// worklet callback that gets fewer samples than it asked for is a
    /// worklet callback that clicks.
    ///
    /// Allocates nothing: `interleaved` is only ever sliced, never resized,
    /// and the de-interleave loop below writes into `left`/`right` in place.
    pub fn render(&mut self, frames: usize) -> usize {
        let frames = frames.min(RENDER_QUANTUM);
        let rendered = self
            .inner
            .as_player()
            .render(&mut self.interleaved[..frames * 2]);
        for i in 0..rendered {
            self.left[i] = self.interleaved[i * 2];
            self.right[i] = self.interleaved[i * 2 + 1];
        }
        rendered
    }

    /// Pointer to the left channel's buffer in wasm linear memory,
    /// [`Self::render_quantum`] frames long. Build a `Float32Array` over it
    /// with [`wasm_memory`] — see that function's doc for the one thing to
    /// get right before holding onto the view it lets you build.
    #[wasm_bindgen(getter, js_name = leftPtr)]
    #[must_use]
    pub fn left_ptr(&self) -> *const f32 {
        self.left.as_ptr()
    }

    /// The right channel's counterpart to [`Self::left_ptr`].
    #[wasm_bindgen(getter, js_name = rightPtr)]
    #[must_use]
    pub fn right_ptr(&self) -> *const f32 {
        self.right.as_ptr()
    }

    /// Start or pause playback. A paused player keeps its position and its
    /// clock — see [`Self::render`] — so resuming continues where it stopped
    /// rather than restarting.
    #[wasm_bindgen(js_name = setPlaying)]
    pub fn set_playing(&mut self, playing: bool) {
        self.inner.as_player().set_playing(playing);
    }

    /// Jump to the top of an order, clamped to the song's played prefix.
    /// Cuts any sounding notes, the way a listener dragging a scrub bar
    /// expects.
    ///
    /// **Modules only**, and it returns whether it applied. A `.ay` has no
    /// seek: each of its songs is an entry point with its own initial
    /// register state, so moving between them means building a new player
    /// (see [`Self::new`]) rather than moving within this one. Returning
    /// `false` rather than doing nothing quietly is the difference between a
    /// caller learning that and a scrub bar that looks broken.
    #[wasm_bindgen(js_name = seekOrder)]
    pub fn seek_order(&mut self, order: usize) -> bool {
        match &mut self.inner {
            Inner::Module(engine) => {
                engine.seek_order(order);
                true
            }
            Inner::Ay(_) => false,
        }
    }

    /// Which shape of position this player reports: `"module"` or `"frame"`.
    ///
    /// A caller reads this once, when the player is built, and then knows
    /// which of the two groups of getters below mean anything — rather than
    /// calling all of them and inferring from which returned `undefined`.
    #[wasm_bindgen(js_name = positionKind)]
    #[must_use]
    pub fn position_kind(&self) -> String {
        match self.position() {
            play198x_core::player::Position::Module(_) => "module".into(),
            play198x_core::player::Position::Frame { .. } => "frame".into(),
        }
    }

    /// Index into the order table's played prefix. Modules only.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn order(&self) -> Option<usize> {
        self.module_position().map(|at| at.order)
    }

    /// The pattern the current order names. Modules only.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn pattern(&self) -> Option<usize> {
        self.module_position().map(|at| at.pattern)
    }

    /// Row within the current pattern, `0..64`. Modules only.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn row(&self) -> Option<usize> {
        self.module_position().map(|at| at.row)
    }

    /// Tick within the current row, `0..speed`. Modules only.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn tick(&self) -> Option<u8> {
        self.module_position().map(|at| at.tick)
    }

    /// The subtune being played. Frame-driven formats only.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn song(&self) -> Option<usize> {
        match self.position() {
            play198x_core::player::Position::Frame { song, .. } => Some(song),
            play198x_core::player::Position::Module(_) => None,
        }
    }

    /// Frames played since the song started. Frame-driven formats only.
    ///
    /// A tune's own unit of time: a `.ay` declares its length in 50Hz frames,
    /// so this and that length are directly comparable without either side
    /// converting to milliseconds first.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn frame(&self) -> Option<u32> {
        match self.position() {
            play198x_core::player::Position::Frame { frame, .. } => Some(frame),
            play198x_core::player::Position::Module(_) => None,
        }
    }
}

impl Player {
    /// The position, whichever shape it has.
    ///
    /// Called through the trait explicitly rather than as a method: `Engine`
    /// has an inherent `position` returning a bare [`play198x_core::engine::ModulePosition`],
    /// and method resolution prefers it, so `engine.position()` would quietly
    /// be the wrong one. Naming the trait keeps the decision about how a
    /// module's position is wrapped in the trait impl, where it belongs.
    fn position(&self) -> play198x_core::player::Position {
        use play198x_core::player::Player as CorePlayer;
        match &self.inner {
            Inner::Module(engine) => CorePlayer::position(engine.as_ref()),
            Inner::Ay(pump) => CorePlayer::position(pump.as_ref()),
        }
    }

    /// The position if this is a module, so the four module getters above do
    /// not each repeat the match.
    fn module_position(&self) -> Option<play198x_core::engine::ModulePosition> {
        match self.position() {
            play198x_core::player::Position::Module(at) => Some(at),
            play198x_core::player::Position::Frame { .. } => None,
        }
    }
}

/// Test-facing accessors, deliberately **not** in the `#[wasm_bindgen] impl`
/// block above: anything in that block becomes a JS binding, and a
/// convenient "just copy the samples out for me" method is exactly the
/// shortcut [`Player::render`]'s design exists to make impossible —
/// production code reads `left`/`right` through [`Player::left_ptr`],
/// [`Player::right_ptr`] and [`wasm_memory`] without copying. These
/// two exist only so the Rust-side tests in `tests/boundary.rs` can check
/// what `render` wrote without reaching for `unsafe` themselves — this
/// crate denies `unsafe_code` even in tests, and a raw-pointer read is the
/// one thing standing between a test and the same buffers a browser reads
/// through a view. Named and hidden the same way
/// [`play198x_core::engine::Engine::debug_channel_volume`] is, for the same
/// reason.
impl Player {
    #[doc(hidden)]
    #[must_use]
    pub fn debug_left(&self) -> Vec<f32> {
        self.left.clone()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn debug_right(&self) -> Vec<f32> {
        self.right.clone()
    }
}

/// A handle to this wasm instance's linear memory, for building a
/// `Float32Array` view over [`Player::left_ptr`]/
/// [`Player::right_ptr`] without copying — `new
/// Float32Array(wasmMemory().buffer, ptr, len)`, where `ptr` is a byte
/// offset (as `left_ptr`/`right_ptr` return it) and `len` is in elements,
/// per the `Float32Array` constructor's own contract. No new dependency
/// needed for it: `wasm_bindgen::memory()` is part of the `wasm-bindgen`
/// crate already in this crate's `Cargo.toml`, not `js-sys`.
///
/// # Memory growth detaches existing views
///
/// A `Float32Array` built over `.buffer` **detaches** the moment this wasm
/// module's linear memory grows: the runtime gives the module a new, larger
/// `ArrayBuffer` rather than resizing the old one in place, and a view over
/// the old one goes dead — reads come back zero, forever, with no
/// exception. [`Player::render`] never causes this itself (its buffers
/// are sized once, at construction, and never grow) — but *another*
/// allocation in the same wasm instance can: decoding a second module for a
/// different [`Player`] in the same worklet, for one.
///
/// Nothing on this side of the boundary can stop memory from growing, so the
/// fix belongs to the caller: compare a cached view's `.buffer` against a
/// fresh `wasmMemory().buffer` by reference before trusting it, and rebuild
/// the view when they differ. That comparison is a reference check, not an
/// allocation — cheap enough to run every [`Player::render`] call, and
/// the package README shows the exact pattern.
///
/// Available for the `web` and `nodejs` targets this package builds for —
/// `wasm_bindgen::memory`'s own restriction, and both targets `wasm-pack
/// build` verifies for this crate.
#[wasm_bindgen(js_name = wasmMemory)]
#[must_use]
pub fn wasm_memory() -> JsValue {
    wasm_bindgen::memory()
}

/// What a `.ay` file says about itself, and about each of its songs.
///
/// Built from bytes rather than from a [`Player`], for the same reason
/// [`module_meta`] is: a player is constructed on the audio thread inside the
/// worklet, after a visitor has asked for sound, and an info panel — and the
/// song list a visitor chooses from — must exist before that.
#[wasm_bindgen]
pub struct AyMeta {
    inner: play198x_core::metadata::AyMeta,
}

/// Describe a `.ay` file.
///
/// # Errors
///
/// When the bytes are not a `.ay`, carrying the core's own message.
#[wasm_bindgen(js_name = ayMeta)]
pub fn ay_meta(bytes: &[u8]) -> Result<AyMeta, JsError> {
    let file = play198x_core::player::ay::format::parse(bytes)
        .map_err(|err| JsError::new(&format!("{err:?}")))?;
    Ok(AyMeta {
        inner: play198x_core::metadata::ay_meta(&file),
    })
}

#[wasm_bindgen]
impl AyMeta {
    /// Song 0's name, standing in for a title the format has no file-level
    /// field for.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn title(&self) -> String {
        self.inner.title.clone()
    }

    /// `PAuthor` from the file header.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn author(&self) -> String {
        self.inner.author.clone()
    }

    /// `PMisc` from the file header — often a year, a group, or a note.
    #[wasm_bindgen(getter)]
    #[must_use]
    pub fn misc(&self) -> String {
        self.inner.misc.clone()
    }

    /// How many songs the file carries. More than one for 278 of the 696
    /// files in the local archive, so an interface that ignores this leaves
    /// most of the corpus unreachable.
    #[wasm_bindgen(js_name = songCount)]
    #[must_use]
    pub fn song_count(&self) -> usize {
        self.inner.songs.len()
    }

    /// Song `index`'s name, or `undefined` past the end.
    #[wasm_bindgen(js_name = songName)]
    #[must_use]
    pub fn song_name(&self, index: usize) -> Option<String> {
        self.inner.songs.get(index).map(|song| song.name.clone())
    }

    /// How long song `index` plays before fading, in milliseconds.
    ///
    /// Converted here rather than at the call site: the file states it in
    /// 50Hz frames, and a panel that did the arithmetic itself would be a
    /// second place to get the 50 wrong.
    #[wasm_bindgen(js_name = songLengthMs)]
    #[must_use]
    pub fn song_length_ms(&self, index: usize) -> Option<f64> {
        self.inner
            .songs
            .get(index)
            .map(|song| f64::from(song.length_frames) * 1000.0 / 50.0)
    }

    /// How long song `index`'s fade lasts, in milliseconds, following
    /// [`Self::song_length_ms`].
    #[wasm_bindgen(js_name = songFadeMs)]
    #[must_use]
    pub fn song_fade_ms(&self, index: usize) -> Option<f64> {
        self.inner
            .songs
            .get(index)
            .map(|song| f64::from(song.fade_frames) * 1000.0 / 50.0)
    }
}
