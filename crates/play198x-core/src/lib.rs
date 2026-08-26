//! Open, identify and render retro media.
//!
//! The crate takes bytes from a plain file, a ZIP archive or an Amiga disk
//! image, works out what they are, and produces either an RGBA image or a
//! ProTracker module that plays.
//!
//! Nothing here panics on bad input, holds global state, or assumes a main
//! thread: the crate is built for an FFI boundary and for thumbnailers running
//! once per file inside somebody else's process.

pub mod container;
pub mod decode;
pub mod engine;
pub mod metadata;
pub mod probe;

/// Everything that can go wrong, from opening a path to decoding an entry.
///
/// Container errors say what is true rather than what is convenient. A
/// bootblock Amiga disk is **not** a corrupt ADF — it is a disk with no DOS
/// filesystem, and reporting it as corruption sends the reader looking for a
/// damaged image that does not exist.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The path could not be read.
    Io(std::io::Error),
    /// The container opened, but this entry is not in it.
    NoSuchEntry { path: String },
    /// The bytes are not any format this crate recognises.
    Unrecognised,
    /// A decoder rejected the bytes. Carries the decoder's own message.
    Decode { format: probe::Format, what: String },
    /// The container itself is malformed.
    Container { what: String },
    /// The disk has no DOS filesystem — a bootblock or non-DOS disk, which is
    /// a different thing from a damaged one.
    NotAFilesystem,
    /// The file is a disk image this crate does not read: a flux or archive
    /// container such as IPF or DMS, or a high-density image. Named rather
    /// than measured, for the same reason [`Self::NotAFilesystem`] exists —
    /// telling the reader an IPF is the wrong *size* for an ADF sends them
    /// checking a truncated file that was never an ADF in the first place.
    UnsupportedContainer {
        /// Short name of the format the bytes identify — `"IPF"`, `"DMS"`.
        format: String,
        /// What it is, in a clause that finishes "…, which this crate does
        /// not read".
        detail: String,
    },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(err) => write!(f, "could not read the file: {err}"),
            Self::NoSuchEntry { path } => {
                write!(f, "the container holds no entry named `{path}`")
            }
            Self::Unrecognised => f.write_str("the bytes are not a format this crate recognises"),
            Self::Decode { format, what } => {
                write!(f, "the {format:?} decoder rejected the bytes: {what}")
            }
            Self::Container { what } => write!(f, "the container is damaged: {what}"),
            // Says what is true. A disk that boots from its bootblock simply
            // carries no DOS filesystem, so this must not read as damage.
            Self::NotAFilesystem => {
                f.write_str("the disk carries no DOS filesystem, so it has no files to list")
            }
            Self::UnsupportedContainer { format, detail } => write!(
                f,
                "the file is {format} — {detail}, which this crate does not read"
            ),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}
