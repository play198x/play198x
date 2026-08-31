//! Turn identified bytes into something an interface can show or play.
//!
//! The Format198x crates stay index-only, because a palette belongs to
//! `mediaspec198x` and a dependency-free format crate cannot reach it. The
//! conversion to RGBA therefore happens here, where the palette is in hand.
//!
//! # Where each machine's colours come from
//!
//! | Format | Palette | Pixel aspect |
//! |---|---|---|
//! | SCR | `mediaspec198x`, Spectrum default interpretation | Spectrum `standard` mode |
//! | Koala | `mediaspec198x`, C64 default interpretation | C64 `multicolour-bitmap` mode |
//! | Art Studio | `mediaspec198x`, C64 default interpretation | C64 `hires-bitmap` mode |
//! | ILBM | the file's own CMAP chunk | Amiga OCS lores/hires mode, per CAMG |
//!
//! **No palette constant appears in this module**, and none should. The
//! Amiga is the one machine with no default table to look up, and that is not
//! a gap: OCS colour registers hold four bits per gun, so the machine's
//! palette model is a parametric gamut rather than a fixed table, and an ILBM
//! carries the twelve-bit values it actually used in its CMAP chunk. The
//! answer to "what is the Amiga's default palette" is that there isn't one.
//!
//! Pixel aspects come from the same spec for the same reason. They are
//! mode-relative — the width-to-height shape of one mode pixel against the
//! machine's own single-width pixel — so a C64 multicolour pixel is 2:1 and an
//! Amiga hires pixel is 1:2. A shell scales by the ratio and gets a picture
//! the right shape on every machine, which is the whole point of taking one
//! convention from one place rather than inventing a second here.

use crate::{Error, probe::Format};
use mediaspec198x::{MachineGraphics, NamedPalette, ScreenMode};

/// A decoded picture, ready to upload as a texture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Width in mode pixels.
    pub width: u32,
    /// Height in mode pixels.
    pub height: u32,
    /// Row-major RGBA8, `width * height * 4` bytes. Alpha is always opaque:
    /// none of these formats has a transparency channel.
    pub rgba: Vec<u8>,
    /// Width : height of one mode pixel, relative to the machine's own
    /// single-width pixel. Non-square pixels are the norm on this hardware, so
    /// a shell that ignores this draws a C64 multicolour picture at half its
    /// real width. Both components are non-zero.
    pub pixel_aspect: (u32, u32),
    /// The colours the picture was drawn from, in hardware index order —
    /// `mediaspec198x`'s table for the fixed-palette machines, the file's own
    /// CMAP for an ILBM.
    ///
    /// Kept because it cannot be recovered from [`rgba`](Self::rgba)
    /// afterwards: a picture that never uses colour 5 loses it, and the index
    /// order goes with it. `metadata::image_meta` reports this, and a shell
    /// that offers a palette view or a recolour needs it; deriving it from the
    /// pixels instead would be a different, smaller fact wearing the same name.
    pub palette: Vec<(u8, u8, u8)>,
    /// What the bytes were identified as.
    pub format: Format,
}

/// Decode `bytes`, which the caller has already identified as `format`, into
/// RGBA.
///
/// Passing the wrong `format` is not undefined behaviour, only an error: the
/// decoder for that format rejects the bytes and says so.
///
/// # Errors
///
/// [`Error::Decode`] naming `format` and carrying the decoder's own message,
/// whenever the bytes are not a picture that format's decoder accepts, or the
/// spec data the conversion needs is missing.
pub fn image(bytes: &[u8], format: Format) -> Result<Image, Error> {
    match format {
        Format::Scr => spectrum_screen(bytes),
        Format::Koala => koala(bytes),
        Format::ArtStudio => art_studio(bytes),
        Format::Ilbm => ilbm(bytes),
        // A module is played, not displayed. Routed here rather than left to a
        // catch-all so that adding a picture format to `Format` is a compile
        // error in this match instead of a silent "not an image".
        Format::ProTracker => Err(Error::Decode {
            format,
            what: "a ProTracker module is music, not a picture; use `decode::module`".to_owned(),
        }),
        // An `.ay` tune is Z80 code and data run on a virtual 128K Spectrum,
        // not a picture — same reasoning as ProTracker above. `Format::Ay` is
        // named here unconditionally even though playing one needs the `ay`
        // feature, because refusing correctly does not: this arm exists in
        // every build so identifying an `.ay` a build cannot play still gets
        // an honest answer instead of one this match forgot to write.
        Format::Ay => Err(Error::Decode {
            format,
            what: "an .ay tune is code for a Z80 to run, not a picture; use `player::ay`"
                .to_owned(),
        }),
        Format::Sid => Err(Error::Decode {
            format,
            what: "a SID tune is 6502 code and chip data, not a picture; use `player::sid`"
                .to_owned(),
        }),
    }
}

/// Decode a ProTracker module.
///
/// Separate from [`image`] because the result is played rather than displayed,
/// and the engine consumes it directly.
///
/// # Errors
///
/// [`Error::Decode`] naming [`Format::ProTracker`] and carrying the decoder's
/// own message. This is the **expected** outcome for a `6CHN`, `8CHN` or
/// `FLT8` module: [`crate::probe::identify`] reports those as ProTracker with
/// certainty, because that is what they are, while this crate's decoder handles
/// four channels only. Identifying a file honestly and then failing to decode
/// it is a truthful pair of answers, not an inconsistency to paper over.
pub fn module(bytes: &[u8]) -> Result<format198x_commodore_amiga_mod::Module, Error> {
    format198x_commodore_amiga_mod::decode(bytes).map_err(|err| Error::Decode {
        format: Format::ProTracker,
        what: err.to_string(),
    })
}

/// Look a machine up in the spec, or say which one is missing.
fn machine(id: &'static str, format: Format) -> Result<&'static MachineGraphics, Error> {
    mediaspec198x::machine(id).ok_or_else(|| Error::Decode {
        format,
        what: format!("the media spec describes no machine `{id}`"),
    })
}

/// A machine's screen mode by name, or say which one is missing.
fn mode(
    machine: &'static MachineGraphics,
    name: &'static str,
    format: Format,
) -> Result<&'static ScreenMode, Error> {
    machine.mode(name).ok_or_else(|| Error::Decode {
        format,
        what: format!("the media spec gives `{}` no `{name}` mode", machine.id),
    })
}

/// A machine's pinned default palette interpretation, or say it has none.
///
/// Only ever called for the fixed-palette machines. A gamut machine legitimately
/// answers `None` here, which is why the Amiga path never asks.
fn default_palette(
    machine: &'static MachineGraphics,
    format: Format,
) -> Result<&'static NamedPalette, Error> {
    machine.default_palette().ok_or_else(|| Error::Decode {
        format,
        what: format!("the media spec pins `{}` no default palette", machine.id),
    })
}

/// The mode-relative pixel shape, widened for the public type.
fn pixel_aspect(mode: &ScreenMode) -> (u32, u32) {
    (
        u32::from(mode.pixel_aspect.horizontal),
        u32::from(mode.pixel_aspect.vertical),
    )
}

/// A spec palette as plain triples, for [`Image::palette`].
///
/// The public type says `(u8, u8, u8)` rather than `mediaspec198x::Rgb` so that
/// the ILBM path, whose colours come from a file and never touch the spec, can
/// report in the same terms as the machines that do.
fn triples(palette: &NamedPalette) -> Vec<(u8, u8, u8)> {
    palette.colours.iter().map(|c| (c.r, c.g, c.b)).collect()
}

/// Append one hardware colour index's RGBA to `rgba`.
///
/// Indexing `colours` directly would be a panic on a palette shorter than the
/// indices a format can produce — reachable, because the two come from
/// different crates, and undefined behaviour at the FFI boundary this crate is
/// built for. So it is a lookup that can fail and says what failed.
fn push_indexed(
    rgba: &mut Vec<u8>,
    palette: &NamedPalette,
    index: u8,
    format: Format,
) -> Result<(), Error> {
    let colour = palette
        .colours
        .get(usize::from(index))
        .ok_or_else(|| Error::Decode {
            format,
            what: format!(
                "colour index {index} is past the {} entries of palette `{}`",
                palette.colours.len(),
                palette.name
            ),
        })?;
    rgba.extend_from_slice(&[colour.r, colour.g, colour.b, 0xFF]);
    Ok(())
}

/// Say that a decoder produced no pixel where the geometry says there is one.
///
/// Every one of these lookups is in-range by the geometry the same crate
/// reports, so this message means the format crate and its own constants have
/// disagreed — worth naming rather than silently painting black.
fn missing_pixel(x: usize, y: usize, format: Format) -> Error {
    Error::Decode {
        format,
        what: format!("the decoder has no pixel at ({x}, {y}) inside its own dimensions"),
    }
}

/// A Spectrum screen: one bit per pixel, with INK and PAPER per 8×8 cell.
fn spectrum_screen(bytes: &[u8]) -> Result<Image, Error> {
    const FORMAT: Format = Format::Scr;

    let screen =
        format198x_sinclair_zx_spectrum_scr::decode(bytes).map_err(|err| Error::Decode {
            format: FORMAT,
            what: err.to_string(),
        })?;

    let machine = machine("sinclair-zx-spectrum", FORMAT)?;
    let mode = mode(machine, "standard", FORMAT)?;
    let palette = default_palette(machine, FORMAT)?;

    let width = usize::from(mode.paper_width);
    let height = usize::from(mode.paper_height);
    let mut rgba = Vec::with_capacity(width * height * 4);

    for y in 0..height {
        for x in 0..width {
            let attribute = *screen
                .attributes
                .get((y / 8) * format198x_sinclair_zx_spectrum_scr::COLUMNS + x / 8)
                .ok_or_else(|| missing_pixel(x, y, FORMAT))?;

            // `FBPPPIII`. BRIGHT selects the palette's upper half for INK and
            // PAPER together — never one and not the other — which is why it
            // is applied to whichever of the two this pixel took, rather than
            // being folded into either colour. FLASH is deliberately ignored:
            // it swaps INK and PAPER every 16 frames, and a still picture has
            // no frames to count.
            let bright = (attribute >> 6) & 1;
            let set = screen
                .pixel(x, y)
                .ok_or_else(|| missing_pixel(x, y, FORMAT))?;
            let colour = if set {
                attribute & 0b111
            } else {
                (attribute >> 3) & 0b111
            };

            push_indexed(&mut rgba, palette, (bright << 3) | colour, FORMAT)?;
        }
    }

    Ok(Image {
        width: u32::from(mode.paper_width),
        height: u32::from(mode.paper_height),
        rgba,
        pixel_aspect: pixel_aspect(mode),
        palette: triples(palette),
        format: FORMAT,
    })
}

/// A C64 multicolour bitmap: double-wide pixels, four colours per cell, one of
/// them shared by the whole screen.
fn koala(bytes: &[u8]) -> Result<Image, Error> {
    const FORMAT: Format = Format::Koala;

    let picture = format198x_commodore_c64_koala::decode(bytes).map_err(|err| Error::Decode {
        format: FORMAT,
        what: err.to_string(),
    })?;

    let machine = machine("commodore-c64", FORMAT)?;
    let mode = mode(machine, "multicolour-bitmap", FORMAT)?;
    let palette = default_palette(machine, FORMAT)?;

    from_indices(mode, palette, FORMAT, |x, y| picture.color_index(x, y))
}

/// A C64 high-resolution bitmap: two freely chosen colours per 8×8 cell.
fn art_studio(bytes: &[u8]) -> Result<Image, Error> {
    const FORMAT: Format = Format::ArtStudio;

    let picture =
        format198x_commodore_c64_art_studio::decode(bytes).map_err(|err| Error::Decode {
            format: FORMAT,
            what: err.to_string(),
        })?;

    let machine = machine("commodore-c64", FORMAT)?;
    let mode = mode(machine, "hires-bitmap", FORMAT)?;
    let palette = default_palette(machine, FORMAT)?;

    from_indices(mode, palette, FORMAT, |x, y| picture.color_index(x, y))
}

/// Walk a mode's paper, asking `index_at` for each pixel's hardware colour.
///
/// The two C64 formats differ only in their geometry and in how they resolve a
/// cell, both of which their own crates already answer, so the walk itself is
/// stated once.
fn from_indices(
    mode: &ScreenMode,
    palette: &NamedPalette,
    format: Format,
    index_at: impl Fn(usize, usize) -> Option<u8>,
) -> Result<Image, Error> {
    let width = usize::from(mode.paper_width);
    let height = usize::from(mode.paper_height);
    let mut rgba = Vec::with_capacity(width * height * 4);

    for y in 0..height {
        for x in 0..width {
            let index = index_at(x, y).ok_or_else(|| missing_pixel(x, y, format))?;
            push_indexed(&mut rgba, palette, index, format)?;
        }
    }

    Ok(Image {
        width: u32::from(mode.paper_width),
        height: u32::from(mode.paper_height),
        rgba,
        pixel_aspect: pixel_aspect(mode),
        palette: triples(palette),
        format,
    })
}

/// An Amiga ILBM, which carries its own palette.
///
/// The one path with no `mediaspec198x` palette lookup, and the reason the
/// Amiga's `default_interpretation` being `None` is an answer rather than a
/// gap: the file says what its colours were.
fn ilbm(bytes: &[u8]) -> Result<Image, Error> {
    const FORMAT: Format = Format::Ilbm;

    let picture = format198x_commodore_amiga_ilbm::decode(bytes).map_err(|err| Error::Decode {
        format: FORMAT,
        what: err.to_string(),
    })?;

    let width = usize::from(picture.width);
    let height = usize::from(picture.height);
    let mut rgba = Vec::with_capacity(width * height * 4);

    for y in 0..height {
        for x in 0..width {
            let index = *picture
                .pixels
                .get(y * width + x)
                .ok_or_else(|| missing_pixel(x, y, FORMAT))?;

            // A CMAP short of the planes' index range is the file's own
            // inconsistency, and there is nothing here to fall back on: this
            // machine has no fixed table, and inventing one would be exactly
            // the palette constant this module must not contain. So it is an
            // error that names the index, not a pixel quietly painted black.
            let colour = picture
                .palette
                .get(usize::from(index))
                .ok_or_else(|| Error::Decode {
                    format: FORMAT,
                    what: format!(
                        "pixel index {index} has no CMAP entry; the file's palette holds {}",
                        picture.palette.len()
                    ),
                })?;
            rgba.extend_from_slice(&[colour[0], colour[1], colour[2], 0xFF]);
        }
    }

    Ok(Image {
        width: u32::from(picture.width),
        height: u32::from(picture.height),
        rgba,
        pixel_aspect: pixel_aspect(amiga_mode(&picture, FORMAT)?),
        palette: picture.palette.iter().map(|&[r, g, b]| (r, g, b)).collect(),
        format: FORMAT,
    })
}

/// The Amiga screen mode an ILBM's CAMG viewmode word names.
///
/// The BMHD's own `xAspect`/`yAspect` are **not** used, for two reasons. They
/// are a display aspect (10:11 for lores PAL) rather than the mode-relative
/// shape every other format here reports, so mixing them in would put two
/// conventions in one field; and they are routinely written as zero, which is
/// not a ratio at all. The CAMG hires bit is the fact that actually decides the
/// pixel's shape.
///
/// PAL against NTSC is a question about how many lines are visible, not about
/// pixel shape — the spec gives the two the same aspect — and an ILBM states
/// its own height, so the choice below cannot change the answer.
fn amiga_mode(
    picture: &format198x_commodore_amiga_ilbm::Ilbm,
    format: Format,
) -> Result<&'static ScreenMode, Error> {
    let machine = machine("commodore-amiga-ocs", format)?;
    let hires = picture.camg & format198x_commodore_amiga_ilbm::CAMG_HIRES != 0;
    mode(
        machine,
        if hires { "hires-pal" } else { "lores-pal" },
        format,
    )
}
