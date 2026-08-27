//! Open a path, enumerate its entries, and read one back.
//!
//! A plain file, a ZIP archive and an Amiga ADF disk image all present the same
//! way, so probing and decoding take one code path regardless of where the
//! bytes came from.

use std::io::{Read as _, Seek as _};
use std::path::{Path, PathBuf};

use format198x_commodore_amiga_adf::{Disk, EntryKind};

use crate::Error;

/// The most `read` will hand back for one entry.
///
/// Derived from the largest thing this crate can legitimately open, which is an
/// Amiga disk image: 901,120 bytes for a double-density floppy and 1,802,240
/// for a high-density one. Everything else is smaller by an order of magnitude
/// or more — a ProTracker module runs to a few hundred kilobytes, an ILBM to a
/// few hundred more, a Spectrum screen to exactly 6,912. Sixteen mebibytes is
/// a little over nine high-density disks: headroom for whatever format this
/// crate grows next, while still refusing the four-gigabyte entry a hostile
/// archive can declare for the price of four bytes of central directory.
const MAX_ENTRY_LEN: u64 = 16 * 1024 * 1024;

/// The most `open` will load into memory for a whole container.
///
/// [`MAX_ENTRY_LEN`] bounds one entry; this bounds the archive that holds it,
/// which is a separate allocation made earlier and on weaker evidence. Four
/// bytes of `PK\x03\x04` at the front of an eight-gigabyte file are enough to
/// reach the load, because the archive is only validated once its bytes are
/// resident — so the size has to be refused before the read, not after it.
///
/// Derived the same way as the entry cap. The largest thing legitimately
/// opened is a high-density Amiga disk image at 1,802,240 bytes, and the
/// realistic upper end is a ZIP holding a disk's worth of modules. Sixty-four
/// mebibytes is thirty-seven high-density disks, and four times
/// [`MAX_ENTRY_LEN`], so an archive whose single entry sits right on the entry
/// cap still opens even when stored uncompressed.
const MAX_ARCHIVE_LEN: u64 = 64 * 1024 * 1024;

/// How many bytes `open` looks at to tell one container shape from another.
const SIGNATURE_LEN: usize = 4;

/// A double-density Amiga floppy image: 1,760 blocks of 512 bytes.
///
/// An ADF has no magic number of its own — the bytes at offset 0 are the boot
/// block, which a non-DOS disk fills with whatever it likes — so the length is
/// the identifying signal. That is why `open` needs the file's size as well as
/// its first four bytes.
const ADF_DD_LEN: u64 = 901_120;

/// A high-density Amiga floppy image: twice the double-density size.
const ADF_HD_LEN: u64 = 1_802_240;

/// The most entries `entries` will walk out of one disk image before giving
/// up.
///
/// Every directory and file on an ADF occupies at least one 512-byte header
/// block, and a double-density disk holds 1,760 of them, so a well-formed
/// image cannot exceed this. A hostile one whose directories point at each
/// other can, and the walk would otherwise never finish.
const MAX_DISK_ENTRIES: usize = 1_760;

/// One readable thing inside a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The entry's name, exactly as the container states it, and **not
    /// sanitised**.
    ///
    /// A hostile archive can name an entry `../../../etc/passwd`, `/abs/root`,
    /// `C:\win\evil`, or anything at all with a right-to-left override in it,
    /// and all of those arrive here verbatim. Harmless within this crate — it
    /// only ever reads, never joins this onto a directory and never writes —
    /// but a caller that turns one of these into a filesystem path owns that
    /// decision and must sanitise first.
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
///
/// Noted, not fixed: `Clone` copies a whole resident archive, which is an
/// expensive thing to do by accident. It stays derived because the shells will
/// want it, and 64 MiB is the worst case either way.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Container {
    /// A file on disk, holding one entry named by its basename. Loaded lazily
    /// — see [`Self::open`] — so this variant never holds the file's bytes.
    Plain(PathBuf),
    /// A plain file's bytes, already resident, holding one entry named by the
    /// caller rather than by a basename. [`Self::Plain`] cannot represent
    /// this: it names a path to reopen on demand, and a caller building this
    /// variant — the wasm build handed a browser `File`'s bytes, which is the
    /// case this exists for — has no path at all, only the name the browser
    /// reported and the bytes themselves. Built by [`Self::from_bytes`].
    PlainBytes(String, Vec<u8>),
    /// The whole archive, in memory. A `ZipArchive` is built over a cursor per
    /// call rather than held, so `read` needs only `&self` and no interior
    /// mutability — the access pattern is a handful of entries, not a stream.
    Zip(Vec<u8>),
    /// A whole Amiga disk image, in memory. Held as bytes rather than as a
    /// `Disk` for the same reason as `Zip`: a `Disk` borrows the image, so
    /// storing one would make the container self-referential to buy nothing —
    /// parsing 880K of already-resident bytes is a few microseconds.
    Adf(Vec<u8>),
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
        // paid on every thumbnail. Two cheap signals answer it: the first four
        // bytes, and the length — which an ADF is identified by, having no
        // magic number of its own.
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            // `/dev/zero` opens happily, reports a length of zero, and then
            // hands back bytes for as long as anything asks. Every length
            // signal below is a lie about a thing like that, so it is refused
            // here rather than left for a read to discover — which, for
            // `/dev/zero`, means never returning at all.
            return Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("`{}` is not a regular file", path.display()),
            )));
        }
        let len = metadata.len();
        let mut signature = Vec::with_capacity(SIGNATURE_LEN);
        file.by_ref()
            .take(SIGNATURE_LEN as u64)
            .read_to_end(&mut signature)?;

        match sniff(&signature, len) {
            Sniffed::Zip => {
                // Parse once and discard: this is the check, not the read.
                let bytes = whole(&mut file, path, len)?;
                archive(&bytes)?;
                Ok(Self::Zip(bytes))
            }
            // The reader has an Amiga disk image; say so, rather than let it
            // reach the ADF crate and come back as a complaint about size.
            // High-density images are real media this crate cannot yet read,
            // which is a different fact from a damaged disk.
            Sniffed::AdfHd => Err(hd_adf_unsupported()),
            Sniffed::Adf => {
                // Validated here, like an archive, so a disk that turns out to
                // carry no filesystem is reported where the caller named the
                // file.
                let bytes = whole(&mut file, path, len)?;
                Disk::open(&bytes).map_err(from_adf)?;
                Ok(Self::Adf(bytes))
            }
            Sniffed::Plain => Ok(Self::Plain(path.to_path_buf())),
        }
    }

    /// Build a container from bytes already resident in memory, for a caller
    /// with no filesystem to hand [`Self::open`] a path — the wasm build,
    /// given a browser `File`'s bytes directly, is the case this exists for.
    ///
    /// `name` becomes the sole entry's [`Entry::path`] if the bytes turn out
    /// to be a plain file, exactly the role a path's basename plays for
    /// [`Self::open`]. It is exactly as untrusted: a browser's `File.name` is
    /// caller-supplied and is passed through verbatim, per the warning on
    /// [`Entry::path`] — never sanitised, joined onto a directory, or written
    /// by this crate.
    ///
    /// The decision of what the bytes are is the same one `open` makes from a
    /// signature and a length, so both share it. What differs is the cost:
    /// `open` sniffs before loading specifically to avoid pulling a whole disk
    /// image into memory only to throw it away, and that is a cost this
    /// function cannot avoid, because the bytes are already resident by the
    /// time it is called — there is nothing left to defer. What still applies
    /// is [`MAX_ARCHIVE_LEN`], checked before any sniffing or parsing runs,
    /// for the same reason it gates `open`: a `Vec::with_capacity` of a
    /// hostile size is an allocation that aborts the process rather than
    /// unwinding, and a browser tab is exactly where somebody drops a 400 MB
    /// ZIP.
    pub fn from_bytes(bytes: Vec<u8>, name: &str) -> Result<Self, Error> {
        let len = bytes.len() as u64;
        if len > MAX_ARCHIVE_LEN {
            return Err(too_large(name, len, MAX_ARCHIVE_LEN));
        }

        match sniff(&bytes, len) {
            Sniffed::Zip => {
                archive(&bytes)?;
                Ok(Self::Zip(bytes))
            }
            Sniffed::AdfHd => Err(hd_adf_unsupported()),
            Sniffed::Adf => {
                Disk::open(&bytes).map_err(from_adf)?;
                Ok(Self::Adf(bytes))
            }
            Sniffed::Plain => Ok(Self::PlainBytes(name.to_owned(), bytes)),
        }
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
            Self::PlainBytes(name, bytes) => Ok(vec![Entry {
                path: name.clone(),
                len: bytes.len() as u64,
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
            Self::Adf(bytes) => {
                let disk = Disk::open(bytes).map_err(from_adf)?;
                let mut entries = Vec::new();
                // Depth-first from the root, with subdirectories pushed in
                // reverse so they come back out in listing order.
                let mut pending = vec![String::new()];
                let mut budget = MAX_DISK_ENTRIES;
                while let Some(dir) = pending.pop() {
                    let mut subdirs = Vec::new();
                    for entry in disk.list(&dir).map_err(from_adf)? {
                        let Some(left) = budget.checked_sub(1) else {
                            return Err(Error::Container {
                                what: "the directory tree does not terminate".to_owned(),
                            });
                        };
                        budget = left;
                        let path = join(&dir, &entry.name);
                        match entry.kind {
                            // Directories carry no bytes to probe or decode,
                            // so they are walked but never listed.
                            EntryKind::Directory => subdirs.push(path),
                            EntryKind::File => entries.push(Entry {
                                path,
                                len: u64::from(entry.size),
                            }),
                        }
                    }
                    subdirs.reverse();
                    pending.append(&mut subdirs);
                }
                Ok(entries)
            }
        }
    }

    /// Read one entry's bytes, decrunched if they arrived PowerPacked.
    ///
    /// Transparent decrunching hangs off this one point rather than off each
    /// container kind, so a PowerPacked module behaves the same on a disk
    /// image, inside a ZIP, and sitting loose on the filesystem. Amiga music
    /// disks make that ordinary: three of the first four modules found on a
    /// real Gathering '92 disk during this project's research were PP20.
    pub fn read(&self, entry: &str) -> Result<Vec<u8>, Error> {
        let bytes = self.read_raw(entry)?;
        decrunched(entry, bytes)
    }

    /// The entry's bytes exactly as the container stores them.
    fn read_raw(&self, entry: &str) -> Result<Vec<u8>, Error> {
        match self {
            Self::Plain(path) => {
                if entry != basename(path)? {
                    return Err(missing(entry));
                }
                // One handle for both the length and the bytes, and the read
                // bounded as well as the length checked: `std::fs::read` would
                // reserve whatever the metadata claimed and then read until
                // the file stopped giving, which for anything that grows — or
                // anything that turned into a device behind `open`'s back — is
                // unbounded.
                let mut file = std::fs::File::open(path)?;
                let len = file.metadata()?.len();
                bounded(&mut file, entry, len, MAX_ENTRY_LEN)
            }
            Self::PlainBytes(name, bytes) => {
                if entry != name {
                    return Err(missing(entry));
                }
                // Resident already, so there is nothing to read — only the
                // same entry cap [`Self::Plain`] applies via `bounded`, kept
                // here so `read` never hands back more than `MAX_ENTRY_LEN`
                // regardless of which variant answered it.
                let declared = bytes.len() as u64;
                if declared > MAX_ENTRY_LEN {
                    return Err(too_large(entry, declared, MAX_ENTRY_LEN));
                }
                Ok(bytes.clone())
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
                    return Err(too_large(entry, declared, MAX_ENTRY_LEN));
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
                    return Err(too_large(entry, actual, MAX_ENTRY_LEN));
                }
                Ok(out)
            }
            Self::Adf(bytes) => {
                let disk = Disk::open(bytes).map_err(from_adf)?;
                // Look the entry up in its own directory before reading it.
                // `Disk::read` reserves the file header's declared size up
                // front, and that field is four attacker-controlled bytes, so
                // the claim is checked here rather than allocated on trust.
                // The lookup also settles the directory case the same way the
                // ZIP path does.
                let (dir, name) = split(entry);
                let listing = disk.list(dir).map_err(|err| match from_adf(err) {
                    // The parent directory is not there, so the entry is not
                    // either — and it is the name the caller asked for that
                    // has to come back, not the name of its missing parent.
                    Error::NoSuchEntry { .. } => missing(entry),
                    other => other,
                })?;
                let Some(found) = listing.into_iter().find(|e| e.name == name) else {
                    return Err(missing(entry));
                };
                if found.kind == EntryKind::Directory {
                    return Err(missing(entry));
                }
                let declared = u64::from(found.size);
                if declared > MAX_ENTRY_LEN {
                    return Err(too_large(entry, declared, MAX_ENTRY_LEN));
                }
                disk.read(entry).map_err(from_adf)
            }
        }
    }
}

/// Decrunch `bytes` if they are a PowerPacker stream, and hand them straight
/// back if they are not.
fn decrunched(entry: &str, bytes: Vec<u8>) -> Result<Vec<u8>, Error> {
    if !format198x_commodore_amiga_powerpacker::is_powerpacked(&bytes) {
        return Ok(bytes);
    }
    // No declared-length check precedes this call, deliberately. A PP20 trailer
    // states its decrunched length in three bytes and the decruncher allocates
    // that much before reading a bit, which its documentation flags as the
    // caller's problem — but three bytes cap it at 16,777,215, exactly one
    // below `MAX_ENTRY_LEN`. The format cannot ask for more than this crate
    // already allows, so a guard here would be unreachable, and the invariant
    // that `read` never hands back more than the cap holds without one. Lower
    // `MAX_ENTRY_LEN` past 16 MiB and that stops being true.
    format198x_commodore_amiga_powerpacker::decrunch(&bytes).map_err(|err| Error::Container {
        what: format!("entry `{entry}` is PowerPacked but could not be decrunched: {err}"),
    })
}

/// Read the whole of `file` from the start, bounded by [`MAX_ARCHIVE_LEN`].
fn whole(file: &mut std::fs::File, path: &Path, on_disk: u64) -> Result<Vec<u8>, Error> {
    file.seek(std::io::SeekFrom::Start(0))?;
    bounded(file, &path.display().to_string(), on_disk, MAX_ARCHIVE_LEN)
}

/// Read `src` into a fresh buffer, refusing anything past `limit`.
///
/// `declared` is what the source says about itself. It is checked first, so an
/// oversized thing is refused before a byte of it is reserved — the whole
/// point, since a `Vec::with_capacity` of the declared size is exactly the
/// allocation an attacker is asking for, and an allocation that fails aborts
/// the process rather than unwinding.
///
/// Then the read itself is capped, because `declared` is not a fact: a file
/// can grow between the metadata call and the read, and a non-regular one
/// misreports its length outright. Refusing again on what actually arrived
/// closes that race at the cost of one byte.
fn bounded(
    src: &mut impl std::io::Read,
    name: &str,
    declared: u64,
    limit: u64,
) -> Result<Vec<u8>, Error> {
    if declared > limit {
        return Err(too_large(name, declared, limit));
    }
    let mut out = Vec::with_capacity(usize::try_from(declared).unwrap_or(0));
    src.take(limit + 1).read_to_end(&mut out)?;
    let actual = out.len() as u64;
    if actual > limit {
        return Err(too_large(name, actual, limit));
    }
    Ok(out)
}

/// Map the ADF crate's errors onto this crate's, keeping the distinctions that
/// matter to a reader looking at a disk that will not open.
///
/// The crate has no dedicated "carries no filesystem" variant: a disk whose
/// boot block does not begin `DOS` comes back as `Corrupt` carrying that exact
/// `what`. Matching the string is the only discriminator on offer, and this
/// distinction is the whole reason the mapping exists, so it is matched
/// deliberately rather than folded in with real corruption. The test
/// `a_bootblock_disk_is_not_a_corrupt_adf` pins it, so an upgrade that reworded
/// the string would fail a test rather than quietly reclassify every non-DOS
/// disk as damage.
fn from_adf(err: format198x_commodore_amiga_adf::Error) -> Error {
    use format198x_commodore_amiga_adf::Error as Adf;
    match err {
        Adf::Corrupt {
            what: "boot-block signature",
        } => Error::NotAFilesystem,
        Adf::UnsupportedContainer { format, detail } => Error::UnsupportedContainer {
            format: format.to_owned(),
            detail: detail.to_owned(),
        },
        // Both mean "nothing readable answers to that path" — `BadPath` is
        // what the read side returns for a path naming the wrong kind.
        Adf::NotFound { path } | Adf::BadPath { path, .. } => Error::NoSuchEntry { path },
        other => Error::Container {
            what: other.to_string(),
        },
    }
}

/// Split an entry path into its directory and its leaf name. An entry at the
/// root has an empty directory, which is what `Disk::list` wants for it.
fn split(entry: &str) -> (&str, &str) {
    match entry.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", entry),
    }
}

/// Join a directory path and a child name, with no leading slash at the root.
fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_owned()
    } else {
        format!("{dir}/{name}")
    }
}

/// What a signature and a length identify a container as — the decision
/// [`Container::open`] and [`Container::from_bytes`] both make, from
/// different starting points. `open` reads only [`SIGNATURE_LEN`] bytes off
/// the front of the file before it decides whether to load the rest;
/// `from_bytes` already holds the whole thing and passes it through
/// unsliced. Either works, because [`is_zip`] only ever looks at the first
/// four bytes regardless of how much more is behind them.
///
/// Known gap, deliberately left: a self-extracting archive begins with an
/// executable stub, so these four bytes send it down the plain-file path
/// even though `zip::ZipArchive::new` would parse it — a ZIP is found from
/// its tail, not its head. SFX `.exe` archives are common for this material.
/// Catching them means a tail scan for `PK\x05\x06`, which is new capability
/// rather than a fix, and belongs beside `probe`.
enum Sniffed {
    Zip,
    /// A high-density Amiga disk image: real media this crate cannot yet
    /// read. A typed error, not a variant — see [`hd_adf_unsupported`].
    AdfHd,
    Adf,
    Plain,
}

fn sniff(signature: &[u8], len: u64) -> Sniffed {
    if is_zip(signature) {
        Sniffed::Zip
    } else if len == ADF_HD_LEN {
        Sniffed::AdfHd
    } else if len == ADF_DD_LEN {
        Sniffed::Adf
    } else {
        Sniffed::Plain
    }
}

/// The typed error for a high-density Amiga disk image, shared by `open` and
/// `from_bytes` so the format name and detail can only drift in one place.
fn hd_adf_unsupported() -> Error {
    Error::UnsupportedContainer {
        format: "HD ADF".to_owned(),
        detail: "a 1.76 MB high-density Amiga disk image".to_owned(),
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

/// Parse the central directory of a resident archive.
///
/// Noted, not fixed: `open` parses to validate, `entries` parses again, and
/// every `read` parses again — O(N²) for the obvious walk-and-read pattern.
/// Fine for the handful of entries a music disk holds; revisit when `probe`
/// starts iterating archives.
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

/// Refuse something too big to read, as its own error rather than as damage.
///
/// `Error::Container` would be a lie here: a perfectly well-formed archive is
/// allowed to hold an entry larger than this crate chooses to open, and saying
/// "the container is damaged" sends the reader hunting damage that does not
/// exist. `path` is the entry name inside a container, or the file's own path
/// when it is the whole container being refused.
fn too_large(path: &str, len: u64, limit: u64) -> Error {
    Error::TooLarge {
        path: path.to_owned(),
        len,
        limit,
    }
}

fn basename(path: &Path) -> Result<&str, Error> {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| Error::Container {
            what: format!("`{}` has no usable file name", path.display()),
        })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Error, bounded};

    /// `bounded` is unit-tested here rather than through a file because the
    /// interesting case — a source that holds more than it declared — is a
    /// race when it comes from the filesystem and cannot be staged reliably
    /// from a test. A `Cursor` lies on demand.
    #[test]
    fn a_source_declaring_more_than_the_limit_is_refused_by_its_declaration() {
        // Four bytes behind a claim of five thousand. Only the claim is
        // oversized, and the claim is what has to stop it — before a buffer
        // that size is reserved, since a failed allocation aborts rather than
        // unwinds.
        let mut src = std::io::Cursor::new(vec![0u8; 4]);
        match bounded(&mut src, "claim.mod", 5_000, 100) {
            Err(Error::TooLarge { path, len, limit }) => {
                assert_eq!(path, "claim.mod");
                assert_eq!(len, 5_000);
                assert_eq!(limit, 100);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn a_source_holding_more_than_it_declared_stops_one_byte_past_the_limit() {
        // Five hundred bytes behind a claim of ten: the declaration passes and
        // the read is what has to refuse.
        let mut src = std::io::Cursor::new(vec![7u8; 500]);
        match bounded(&mut src, "liar.mod", 10, 100) {
            Err(Error::TooLarge { path, len, limit }) => {
                assert_eq!(path, "liar.mod");
                assert_eq!(
                    len, 101,
                    "the read must stop one byte past the limit, not run to the end"
                );
                assert_eq!(limit, 100);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn a_source_exactly_on_the_limit_reads_back_whole() {
        let mut src = std::io::Cursor::new(vec![3u8; 100]);
        assert_eq!(
            bounded(&mut src, "fine.mod", 100, 100).unwrap(),
            vec![3u8; 100]
        );
    }
}
