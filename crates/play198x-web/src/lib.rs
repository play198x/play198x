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
