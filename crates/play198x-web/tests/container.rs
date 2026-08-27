#![allow(clippy::unwrap_used, clippy::expect_used)]

//! `Container` is a struct, not a pair of free functions, precisely so a
//! visitor clicking through several entries pays the parse-and-validate cost
//! once. These tests exercise it the way a browser would: open once, then
//! read several entries back without reopening.

use wasm_bindgen_test::wasm_bindgen_test;

use play198x_web::Container;

/// A stored (uncompressed) ZIP holding `entries`, built byte-by-byte.
///
/// No media is committed to this repository and `play198x-web` does not
/// depend on the `zip` crate — it is `play198x-core`'s job to read archives,
/// not this shell's — so the fixture is the format's own layout, written out
/// by hand: a local header and its data per entry, then a central directory
/// record per entry, then one end-of-central-directory record.
fn build_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();

    for (name, data) in entries {
        let offset = u32::try_from(out.len()).unwrap();
        let crc = crc32(data);
        let name_bytes = name.as_bytes();
        let len = u32::try_from(data.len()).unwrap();

        // Local file header.
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes()); // compressed size
        out.extend_from_slice(&len.to_le_bytes()); // uncompressed size
        out.extend_from_slice(&u16::try_from(name_bytes.len()).unwrap().to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(data);

        // Central directory record for this entry.
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&u16::try_from(name_bytes.len()).unwrap().to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra field length
        central.extend_from_slice(&0u16.to_le_bytes()); // comment length
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number start
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name_bytes);
    }

    let central_offset = u32::try_from(out.len()).unwrap();
    let central_size = u32::try_from(central.len()).unwrap();
    out.extend_from_slice(&central);

    // End of central directory record.
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&0u16.to_le_bytes()); // disk number
    out.extend_from_slice(&0u16.to_le_bytes()); // disk with central dir
    out.extend_from_slice(&u16::try_from(entries.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&u16::try_from(entries.len()).unwrap().to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length

    out
}

/// The standard IEEE CRC-32 (polynomial `0xEDB88320`), which a ZIP's headers
/// carry per entry. Written out rather than pulled from a crate: `zip`'s own
/// `crc32fast` is a dependency of `play198x-core`, not of this shell, and
/// this shell adds no dependency to write one fixture.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[wasm_bindgen_test]
fn a_zip_is_opened_once_and_every_entry_reads_back() {
    let zip = build_stored_zip(&[
        ("intro.mod", b"first tune"),
        ("s/loader.mod", b"second one"),
    ]);
    let container = Container::new(zip, "disk.zip").expect("a well-formed stored ZIP opens");

    assert_eq!(container.entry_count(), 2);

    let mut seen: Vec<_> = (0..container.entry_count())
        .map(|i| {
            (
                container.entry_path(i).unwrap(),
                container.entry_len(i).unwrap(),
            )
        })
        .collect();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        seen,
        vec![
            ("intro.mod".to_owned(), 10.0),
            ("s/loader.mod".to_owned(), 10.0),
        ]
    );

    // Reads happen after the fact, against the same opened container — no
    // second copy of the archive's bytes crosses the boundary for either.
    assert_eq!(container.read("intro.mod").unwrap(), b"first tune");
    assert_eq!(container.read("s/loader.mod").unwrap(), b"second one");
}

#[wasm_bindgen_test]
fn indices_past_the_entry_count_are_undefined_not_a_panic() {
    let zip = build_stored_zip(&[("only.mod", b"x")]);
    let container = Container::new(zip, "disk.zip").unwrap();

    assert_eq!(container.entry_count(), 1);
    assert!(container.entry_path(1).is_none());
    assert!(container.entry_len(1).is_none());
    assert!(container.entry_path(u32::MAX).is_none());
}

#[wasm_bindgen_test]
fn reading_a_name_the_archive_does_not_hold_is_an_error() {
    let zip = build_stored_zip(&[("only.mod", b"x")]);
    let container = Container::new(zip, "disk.zip").unwrap();

    assert!(container.read("missing.mod").is_err());
}

#[wasm_bindgen_test]
fn a_plain_file_is_a_container_of_one_entry_named_by_the_caller() {
    // Bytes that are neither a ZIP signature nor an ADF-shaped length: the
    // "plain file" branch of `Container::from_bytes`.
    let bytes = b"\x00\x00\x03\xf3 not an archive".to_vec();
    let container = Container::new(bytes.clone(), "loader.bin").unwrap();

    assert_eq!(container.entry_count(), 1);
    assert_eq!(container.entry_path(0).unwrap(), "loader.bin");
    assert_eq!(container.entry_len(0).unwrap(), bytes.len() as f64);
    assert_eq!(container.read("loader.bin").unwrap(), bytes);
}

#[wasm_bindgen_test]
fn bytes_past_the_archive_cap_are_refused_rather_than_loaded() {
    // One past `MAX_ARCHIVE_LEN` (64 MiB), restated here as a literal because
    // the constant itself is private to `play198x-core`. What matters is
    // that the refusal crosses the boundary as an error, not that this test
    // tracks the exact cap.
    let oversized = vec![0u8; 64 * 1024 * 1024 + 1];
    assert!(Container::new(oversized, "huge.bin").is_err());
}
