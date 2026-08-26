//! Identify a format from its bytes, and say how sure that identification is.

/// A format this crate can decode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Format {
    /// Sinclair ZX Spectrum screen memory dump.
    Scr,
    /// Commodore 64 Koala Painter multicolour bitmap.
    Koala,
    /// Commodore 64 Advanced Art Studio high-resolution bitmap.
    ArtStudio,
    /// Amiga IFF interleaved bitmap.
    Ilbm,
    /// ProTracker module.
    ProTracker,
}
