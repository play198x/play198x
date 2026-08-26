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

/// A fresh directory under the system temporary directory, unique per call.
fn tempdir() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("play198x-container-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write `bytes` to a uniquely-placed file called `name`, and give back its path.
fn write_temp(name: &str, bytes: &[u8]) -> PathBuf {
    let path = tempdir().join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

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
    // Ten identical bytes deflate to fewer than ten, so a compressed-size
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
    let c = Container::open(&write_temp("empty.zip", &build_zip(&[]))).unwrap();
    match c.read("nope.mod") {
        Err(Error::NoSuchEntry { path }) => assert_eq!(path, "nope.mod"),
        other => panic!("expected NoSuchEntry, got {other:?}"),
    }
}

#[test]
fn an_empty_zip_holds_no_entries() {
    let c = Container::open(&write_temp("empty.zip", &build_zip(&[]))).unwrap();
    assert_eq!(c.entries().unwrap(), Vec::new());
}

#[test]
fn an_entry_declaring_more_than_the_cap_is_refused_before_it_is_allocated() {
    let mut zip = build_zip(&[("bomb.mod", &[0u8; 64][..])]);
    // Rewrite the declared uncompressed size, in both the local file header and
    // the central directory, to just under 4 GiB. A hostile archive says this;
    // `read` must answer without allocating it.
    let declared = 0xFFFF_FFFEu32.to_le_bytes();
    overwrite(&mut zip, b"PK\x03\x04", 22, &declared);
    overwrite(&mut zip, b"PK\x01\x02", 24, &declared);
    let path = write_temp("bomb.zip", &zip);

    let c = Container::open(&path).unwrap();
    assert_eq!(c.entries().unwrap()[0].len, 0xFFFF_FFFE);

    match c.read("bomb.mod") {
        Err(Error::Container { what }) => assert!(
            what.contains("4294967294"),
            "the refusal must name the declared size: {what}"
        ),
        other => panic!("expected Container, got {other:?}"),
    }
}

/// Overwrite `bytes` at `offset` past the first occurrence of `signature`.
fn overwrite(buf: &mut [u8], signature: &[u8], offset: usize, bytes: &[u8]) {
    let at = buf
        .windows(signature.len())
        .position(|w| w == signature)
        .expect("signature not present")
        + offset;
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
    for at in 0..good.len().min(96) {
        let mut bad = good.clone();
        bad[at] ^= 0xFF;
        let path = write_temp("fuzzed.zip", &bad);
        if let Ok(c) = Container::open(&path) {
            for entry in c.entries().unwrap_or_default() {
                match c.read(&entry.path) {
                    Ok(bytes) => assert!(
                        bytes.len() as u64 <= entry.len,
                        "read more than the entry declared"
                    ),
                    Err(Error::Container { .. } | Error::Io(_) | Error::NoSuchEntry { .. }) => {}
                    Err(other) => panic!("unexpected error for byte {at}: {other:?}"),
                }
            }
        }
    }
}

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
