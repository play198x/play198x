//! Open a path, enumerate its entries, and read one back.
//!
//! A plain file, a ZIP archive and an Amiga ADF disk image all present the same
//! way, so probing and decoding take one code path regardless of where the
//! bytes came from.

use std::io::{Read as _, Seek as _};
use std::path::{Path, PathBuf};

use crate::Error;

/// The most `read` will hand back for one entry.
///
/// Derived from the largest thing this crate can legitimately open, which is an
/// Amiga disk image: 901,120 bytes for a double-density floppy and 1,802,240
/// for a high-density one. Everything else is smaller by an order of magnitude
/// or more — a ProTracker module runs to a few hundred kilobytes, an ILBM to a
/// few hundred more, a Spectrum screen to exactly 6,912. Sixteen mebibytes is
/// a little over nine high-density disks: headroom for a PowerPacked file that
/// expands, and for whatever format this crate grows next, while still refusing
/// the four-gigabyte entry a hostile archive can declare for the price of four
/// bytes of central directory.
const MAX_ENTRY_LEN: u64 = 16 * 1024 * 1024;

/// How many bytes `open` looks at to tell one container shape from another.
const SIGNATURE_LEN: usize = 4;

/// One readable thing inside a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's name within the container, `/`-separated.
    pub path: String,
    /// How many bytes reading it yields, before any decompression this crate
    /// does on the way out.
    pub len: u64,
}

/// A source of entries: a file on disk, or an archive holding several.
///
/// Whatever the shape, a container has entries and each entry has bytes. That
/// uniformity is the point — a plain file is a container of exactly one entry,
/// named by the file's own basename, so nothing downstream has to ask where the
/// bytes came from.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Container {
    /// A file on disk, holding one entry named by its basename.
    Plain(PathBuf),
    /// The whole archive, in memory. A `ZipArchive` is built over a cursor per
    /// call rather than held, so `read` needs only `&self` and no interior
    /// mutability — the access pattern is a handful of entries, not a stream.
    Zip(Vec<u8>),
}

impl Container {
    /// Open `path`, deciding what it is from its bytes rather than its name.
    ///
    /// An archive is validated here rather than at the first `entries` call, so
    /// that a damaged one is reported where the caller named the file.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let mut file = std::fs::File::open(path)?;

        // Sniff before loading. A plain file is read on demand, so pulling a
        // whole disk image into memory only to throw it away would be a cost
        // paid on every thumbnail.
        let mut signature = Vec::with_capacity(SIGNATURE_LEN);
        file.by_ref()
            .take(SIGNATURE_LEN as u64)
            .read_to_end(&mut signature)?;
        if !is_zip(&signature) {
            return Ok(Self::Plain(path.to_path_buf()));
        }

        file.seek(std::io::SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        // Parse once and discard: this is the check, not the read.
        archive(&bytes)?;
        Ok(Self::Zip(bytes))
    }

    /// Every entry the container holds, in the order it holds them.
    ///
    /// Directories are not entries: they carry no bytes to probe or decode, and
    /// listing them would put something in front of a reader that cannot be
    /// opened.
    pub fn entries(&self) -> Result<Vec<Entry>, Error> {
        match self {
            Self::Plain(path) => Ok(vec![Entry {
                path: basename(path)?.to_owned(),
                len: std::fs::metadata(path)?.len(),
            }]),
            Self::Zip(bytes) => {
                let mut archive = archive(bytes)?;
                let mut entries = Vec::with_capacity(archive.len());
                for index in 0..archive.len() {
                    // `by_index_raw` skips decompression and decryption setup:
                    // a name and a length come from the directory, so an
                    // encrypted archive can still be listed.
                    let file = archive.by_index_raw(index).map_err(damaged)?;
                    if file.is_dir() {
                        continue;
                    }
                    entries.push(Entry {
                        path: file.name().to_owned(),
                        len: file.size(),
                    });
                }
                Ok(entries)
            }
        }
    }

    /// Read one entry's bytes.
    ///
    /// Task 3 hangs transparent PowerPacker decrunching off this one point, so
    /// that a packed file behaves the same inside an archive as on disk.
    pub fn read(&self, entry: &str) -> Result<Vec<u8>, Error> {
        match self {
            Self::Plain(path) => {
                if entry != basename(path)? {
                    return Err(missing(entry));
                }
                let len = std::fs::metadata(path)?.len();
                if len > MAX_ENTRY_LEN {
                    return Err(too_large(entry, len));
                }
                Ok(std::fs::read(path)?)
            }
            Self::Zip(bytes) => {
                let mut archive = archive(bytes)?;
                let Some(index) = archive.index_for_name(entry) else {
                    return Err(missing(entry));
                };
                let mut file = archive.by_index(index).map_err(damaged)?;
                if file.is_dir() {
                    return Err(missing(entry));
                }

                // The declared size is the archive's claim, not a fact, so it
                // is checked before anything is reserved and the read itself is
                // bounded as well. A hostile archive can lie in either
                // direction and neither lie gets to allocate.
                let declared = file.size();
                if declared > MAX_ENTRY_LEN {
                    return Err(too_large(entry, declared));
                }
                let mut out = Vec::with_capacity(usize::try_from(declared).unwrap_or(0));
                file.by_ref()
                    .take(MAX_ENTRY_LEN + 1)
                    .read_to_end(&mut out)
                    .map_err(|err| Error::Container {
                        what: format!("entry `{entry}` could not be decompressed: {err}"),
                    })?;
                let actual = out.len() as u64;
                if actual > MAX_ENTRY_LEN {
                    return Err(too_large(entry, actual));
                }
                Ok(out)
            }
        }
    }
}

/// Whether these bytes begin a ZIP archive.
///
/// Identification is from the bytes, never the extension: retro archives are
/// named `.zip`, `.ZIP` and quite often neither.
fn is_zip(bytes: &[u8]) -> bool {
    matches!(
        bytes.first_chunk::<4>(),
        // A local file header; an end-of-central-directory record, which is all
        // an empty archive is; or its ZIP64 counterpart.
        Some(b"PK\x03\x04" | b"PK\x05\x06" | b"PK\x06\x06")
    )
}

fn archive(bytes: &[u8]) -> Result<zip::ZipArchive<std::io::Cursor<&[u8]>>, Error> {
    zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(damaged)
}

/// Everything `zip` rejects is a statement about the archive, not about I/O:
/// the bytes are already in memory by the time any of it runs.
fn damaged(err: zip::result::ZipError) -> Error {
    Error::Container {
        what: err.to_string(),
    }
}

fn missing(entry: &str) -> Error {
    Error::NoSuchEntry {
        path: entry.to_owned(),
    }
}

fn too_large(entry: &str, len: u64) -> Error {
    Error::Container {
        what: format!(
            "entry `{entry}` is {len} bytes, past the {MAX_ENTRY_LEN}-byte limit this crate will read"
        ),
    }
}

fn basename(path: &Path) -> Result<&str, Error> {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| Error::Container {
            what: format!("`{}` has no usable file name", path.display()),
        })
}
