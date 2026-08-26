//! A plain file and a ZIP archive must present as the same kind of thing.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use play198x_core::Error;
use play198x_core::container::Container;

// ---------------------------------------------------------------------------
// Fixtures. Nothing here reads a file from the repository: the constraint is
// that no media ever lands in it, and a ZIP built by `zip`'s own writer also
// keeps the test honest by exercising the same crate the reader uses.
// ---------------------------------------------------------------------------

/// A file in a temporary directory of its own, removed when the test that made
/// it drops it.
///
/// Deliberately not a `tempfile` dependency: the `Drop` impl is four lines, and
/// without it every run left a hundred-odd files under `std::env::temp_dir()`.
/// Derefs to `Path`, so it passes straight to `Container::open`.
struct TempFile {
    dir: PathBuf,
    path: PathBuf,
}

impl std::ops::Deref for TempFile {
    type Target = std::path::Path;

    fn deref(&self) -> &std::path::Path {
        &self.path
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A unique path for a file called `name`, with nothing written to it yet.
fn temp_path(name: &str) -> TempFile {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("play198x-container-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    TempFile { dir, path }
}

/// Write `bytes` to a uniquely-placed file called `name`.
fn write_temp(name: &str, bytes: &[u8]) -> TempFile {
    let file = temp_path(name);
    std::fs::write(&file.path, bytes).unwrap();
    file
}

/// A file that *declares* `len` bytes, of which only `prefix` is written.
///
/// `set_len` extends with zeros, which every filesystem in play stores
/// sparsely, so a sixty-four-mebibyte declaration costs neither disk nor time.
/// The declaration is the whole point: it is what a cap has to refuse.
fn sparse(name: &str, prefix: &[u8], len: u64) -> TempFile {
    let file = temp_path(name);
    let mut handle = std::fs::File::create(&file.path).unwrap();
    handle.write_all(prefix).unwrap();
    handle.set_len(len).unwrap();
    file
}

/// The cap `read` puts on one entry, restated here so a change to it has to be
/// a change to these tests too.
const MAX_ENTRY_LEN: u64 = 16 * 1024 * 1024;

/// The cap `open` puts on a whole container.
const MAX_ARCHIVE_LEN: u64 = 64 * 1024 * 1024;

/// A real DEFLATE archive holding `files`, plus explicit entries for `dirs`.
fn build_zip_with(dirs: &[&str], files: &[(&str, &[u8])]) -> Vec<u8> {
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for dir in dirs {
        writer.add_directory(*dir, options).unwrap();
    }
    for (name, bytes) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn build_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    build_zip_with(&[], files)
}

// ---------------------------------------------------------------------------

#[test]
fn a_plain_file_is_a_container_of_one_entry() {
    let file = write_temp("screen.scr", &vec![0u8; 6912]);

    let c = Container::open(&file).unwrap();
    let entries = c.entries().unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "screen.scr");
    assert_eq!(entries[0].len, 6912);
    assert_eq!(c.read("screen.scr").unwrap(), vec![0u8; 6912]);
}

#[test]
fn a_plain_file_answers_to_its_basename_and_nothing_else() {
    let file = write_temp("tune.mod", &[7u8, 8, 9]);
    let c = Container::open(&file).unwrap();

    assert_eq!(c.read("tune.mod").unwrap(), vec![7, 8, 9]);
    match c.read("tune.MOD") {
        Err(Error::NoSuchEntry { path }) => assert_eq!(path, "tune.MOD"),
        other => panic!("expected NoSuchEntry, got {other:?}"),
    }
}

#[test]
fn a_zip_enumerates_its_entries_and_reads_one_back() {
    let zip = build_zip_with(
        &["a/", "b/"],
        &[
            ("a/tune.mod", &[1u8, 2, 3][..]),
            ("b/screen.scr", &[4u8; 10][..]),
        ],
    );
    let path = write_temp("music.zip", &zip);

    let c = Container::open(&path).unwrap();
    let mut names: Vec<_> = c.entries().unwrap().into_iter().map(|e| e.path).collect();
    names.sort();

    // The two directory entries really are in the archive, and must not be
    // listed: a directory has no bytes to probe or decode.
    assert_eq!(names, vec!["a/tune.mod", "b/screen.scr"]);
    assert_eq!(c.read("a/tune.mod").unwrap(), vec![1, 2, 3]);
    assert_eq!(c.read("b/screen.scr").unwrap(), vec![4u8; 10]);
}

#[test]
fn a_zip_entry_reports_its_uncompressed_length() {
    // 6,912 identical bytes deflate to a few dozen, so a compressed-size
    // answer here would be visibly wrong rather than coincidentally right.
    let path = write_temp("one.zip", &build_zip(&[("screen.scr", &[0u8; 6912][..])]));
    let entries = Container::open(&path).unwrap().entries().unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "screen.scr");
    assert_eq!(entries[0].len, 6912);
}

#[test]
fn a_directory_in_a_zip_cannot_be_read_either() {
    let path = write_temp(
        "dirs.zip",
        &build_zip_with(&["a/"], &[("a/tune.mod", &[1u8][..])]),
    );
    let c = Container::open(&path).unwrap();

    match c.read("a/") {
        Err(Error::NoSuchEntry { path }) => assert_eq!(path, "a/"),
        other => panic!("expected NoSuchEntry, got {other:?}"),
    }
}

#[test]
fn reading_a_missing_entry_names_it() {
    let file = write_temp("empty.zip", &build_zip(&[]));
    let c = Container::open(&file).unwrap();
    match c.read("nope.mod") {
        Err(Error::NoSuchEntry { path }) => assert_eq!(path, "nope.mod"),
        other => panic!("expected NoSuchEntry, got {other:?}"),
    }
}

#[test]
fn an_empty_zip_holds_no_entries() {
    let file = write_temp("empty.zip", &build_zip(&[]));
    let c = Container::open(&file).unwrap();
    assert_eq!(c.entries().unwrap(), Vec::new());
}

#[test]
fn an_entry_declaring_more_than_the_cap_is_refused_by_its_declaration() {
    let mut zip = build_zip(&[("bomb.mod", &[0u8; 64][..])]);
    // Rewrite the declared uncompressed size, in both the local file header and
    // the central directory, to just under 4 GiB. A hostile archive says this;
    // `read` must answer without allocating it.
    let declared = 0xFFFF_FFFEu32.to_le_bytes();
    overwrite(&mut zip, b"PK\x03\x04", 22, &declared);
    overwrite(&mut zip, b"PK\x01\x02", 24, &declared);
    let file = write_temp("bomb.zip", &zip);

    let c = Container::open(&file).unwrap();
    assert_eq!(c.entries().unwrap()[0].len, 0xFFFF_FFFE);

    // `TooLarge`, not `Container`: the archive is perfectly well formed and
    // saying it is damaged sends the reader hunting damage that is not there.
    match c.read("bomb.mod") {
        Err(Error::TooLarge { path, len, limit }) => {
            assert_eq!(path, "bomb.mod");
            assert_eq!(len, 0xFFFF_FFFE);
            assert_eq!(limit, MAX_ENTRY_LEN);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn an_entry_expanding_past_the_cap_is_refused_and_the_read_stops_at_the_cap() {
    // Seventeen mebibytes of zeros deflate to a few kilobytes. Rewriting the
    // declared size *down* is the other lie the read promises to survive: the
    // entry looks tiny in the directory and is not.
    let mut zip = build_zip(&[("bomb.bin", &vec![0u8; 17 * 1024 * 1024][..])]);
    let declared = 64u32.to_le_bytes();
    overwrite(&mut zip, b"PK\x03\x04", 22, &declared);
    overwrite(&mut zip, b"PK\x01\x02", 24, &declared);
    let file = write_temp("expanding.zip", &zip);

    let c = Container::open(&file).unwrap();
    assert_eq!(
        c.entries().unwrap()[0].len,
        64,
        "the directory has to be the one lying, or the test proves nothing"
    );

    match c.read("bomb.bin") {
        Err(Error::TooLarge { path, len, limit }) => {
            assert_eq!(path, "bomb.bin");
            // One byte past the cap, not seventeen mebibytes: the read itself
            // stopped, rather than finishing and being refused afterwards.
            assert_eq!(len, MAX_ENTRY_LEN + 1);
            assert_eq!(limit, MAX_ENTRY_LEN);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn a_plain_file_past_the_cap_is_refused_by_its_declared_length() {
    let file = sparse("huge.bin", b"", MAX_ENTRY_LEN + 4096);
    let c = Container::open(&file).unwrap();
    assert_eq!(c.entries().unwrap()[0].len, MAX_ENTRY_LEN + 4096);

    match c.read("huge.bin") {
        Err(Error::TooLarge { path, len, limit }) => {
            assert_eq!(path, "huge.bin");
            // The declared length, not the capped read's `limit + 1`: the
            // refusal came from the metadata, so nothing was read.
            assert_eq!(len, MAX_ENTRY_LEN + 4096);
            assert_eq!(limit, MAX_ENTRY_LEN);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn an_archive_past_the_whole_container_cap_is_refused_before_it_is_loaded() {
    // A ZIP signature and sixty-four mebibytes of nothing behind it. `open`
    // validates the archive only once its bytes are resident, so those four
    // bytes are enough to reach the allocation; the declared length is what
    // has to stop it first.
    let file = sparse("huge.zip", b"PK\x03\x04", MAX_ARCHIVE_LEN + 4096);

    match Container::open(&file) {
        Err(Error::TooLarge { path, len, limit }) => {
            assert!(path.ends_with("huge.zip"), "must name the file: {path}");
            // Again the declared length rather than `limit + 1`, which is what
            // a capped read of the file would have reported.
            assert_eq!(len, MAX_ARCHIVE_LEN + 4096);
            assert_eq!(limit, MAX_ARCHIVE_LEN);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

/// A character device is not a container, and must be refused where it is
/// named rather than where it is read: `/dev/zero` opens, declares a length of
/// zero, and then answers a read forever.
#[test]
#[cfg(unix)]
fn a_non_regular_file_is_refused_rather_than_opened_as_a_plain_container() {
    match Container::open(std::path::Path::new("/dev/zero")) {
        Err(Error::Io(err)) => {
            assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
            assert!(
                err.to_string().contains("not a regular file"),
                "the refusal must say why: {err}"
            );
        }
        other => panic!("expected Io, got {other:?}"),
    }
}

/// Overwrite `bytes` at `offset` past an occurrence of `signature`.
///
/// A local file header opens the archive and the central directory closes it,
/// so each is looked for from the end it belongs to. Searching both from the
/// front would work on a tiny fixture and fail on a multi-megabyte deflate
/// stream, which can contain anything, header-shaped bytes included.
fn overwrite(buf: &mut [u8], signature: &[u8], offset: usize, bytes: &[u8]) {
    let mut windows = buf.windows(signature.len());
    let found = if signature == b"PK\x03\x04" {
        windows.position(|w| w == signature)
    } else {
        windows.rposition(|w| w == signature)
    };
    let at = found.expect("signature not present") + offset;
    buf[at..at + bytes.len()].copy_from_slice(bytes);
}

#[test]
fn a_damaged_archive_is_a_typed_error_rather_than_a_panic() {
    // A ZIP signature with nothing behind it: enough to be taken for an
    // archive, not enough to be one.
    let path = write_temp("truncated.zip", b"PK\x03\x04ruined");
    match Container::open(&path) {
        Err(Error::Container { what }) => assert!(!what.is_empty()),
        other => panic!("expected Container, got {other:?}"),
    }
}

#[test]
fn a_hostile_archive_never_panics_and_never_reads_bytes_that_are_not_there() {
    let good = build_zip(&[("tune.mod", &[1u8, 2, 3][..])]);
    // Corrupt one byte at a time across the header region and read the result
    // back. Every outcome must be a value or a typed error, never a panic.
    //
    // Counted, because every `Ok` below sits inside a fallible open: were
    // `open` to start rejecting all 96 mutations, or accepting all of them,
    // this loop would still run to the end and still report a pass. The
    // numbers are what was measured, so a swing in either direction is a
    // change in behaviour that has to be looked at rather than absorbed.
    let mut mutations = 0;
    let mut opened = 0;
    let mut listed = 0;
    let mut read_ok = 0;
    let mut read_err = 0;
    for at in 0..good.len().min(96) {
        mutations += 1;
        let mut bad = good.clone();
        bad[at] ^= 0xFF;
        let path = write_temp("fuzzed.zip", &bad);
        if let Ok(c) = Container::open(&path) {
            opened += 1;
            for entry in c.entries().unwrap_or_default() {
                listed += 1;
                match c.read(&entry.path) {
                    Ok(bytes) => {
                        read_ok += 1;
                        assert!(
                            bytes.len() as u64 <= entry.len,
                            "read more than the entry declared"
                        );
                    }
                    Err(
                        Error::Container { .. }
                        | Error::Io(_)
                        | Error::NoSuchEntry { .. }
                        | Error::TooLarge { .. },
                    ) => read_err += 1,
                    Err(other) => panic!("unexpected error for byte {at}: {other:?}"),
                }
            }
        }
    }

    assert_eq!(mutations, 96);
    assert_eq!(opened, OPENED, "how many mutated archives still open");
    assert_eq!(listed, LISTED, "how many entries those archives listed");
    assert_eq!(read_ok, READ_OK, "how many of those entries read back");
    assert_eq!(
        read_err, READ_ERR,
        "how many were refused with a typed error"
    );
    assert_eq!(
        read_ok + read_err,
        listed,
        "every listed entry was read back"
    );
}

// Measured on 2026-08-26 with `zip` 8.6.0, by counting a run of the sweep
// above. They are pinned rather than bounded because a bare "more than zero"
// would pass a regression that opened everything and read nothing.
const OPENED: usize = 86;
const LISTED: usize = 82;
const READ_OK: usize = 58;
const READ_ERR: usize = 24;

/// A ZIP is recognised by its bytes, not by what the file is called.
#[test]
fn a_zip_named_anything_is_still_a_zip() {
    let path = write_temp("collection.mod", &build_zip(&[("real.mod", &[9u8][..])]));
    let c = Container::open(&path).unwrap();

    assert_eq!(
        c.entries()
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect::<Vec<_>>(),
        vec!["real.mod"]
    );
}
