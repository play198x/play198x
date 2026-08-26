//! An Amiga disk image is a container like any other — and a disk that boots
//! from its bootblock is not a damaged one.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use play198x_core::Error;
use play198x_core::container::Container;

/// The length of a double-density Amiga floppy image: 1,760 blocks of 512.
const DD: usize = 901_120;

/// A file in a temporary directory of its own, removed when the test that made
/// it drops it. Four lines rather than a `tempfile` dependency; without it,
/// every run left its disk images behind under `std::env::temp_dir()`.
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

/// Write `bytes` to a uniquely-placed file called `name`. No fixture is ever
/// read from the repository — every disk image below is built in the test that
/// uses it.
fn write_temp(name: &str, bytes: &[u8]) -> TempFile {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("play198x-adf-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    TempFile { dir, path }
}

#[test]
fn a_bootblock_disk_is_not_a_corrupt_adf() {
    // 880K of plausible-looking disk that carries no DOS filesystem.
    let mut img = vec![0u8; DD];
    img[0..4].copy_from_slice(b"\x00\x00\x03\xF3"); // Hunk header, not DOS
    let path = write_temp("boot.adf", &img);

    match Container::open(&path) {
        Err(Error::NotAFilesystem) => {}
        other => panic!("expected NotAFilesystem, got {other:?}"),
    }
}

#[test]
fn an_ipf_is_named_rather_than_measured() {
    // Right length for a double-density ADF, wrong format entirely. Telling
    // the reader this is a damaged disk would be a lie about a file that is
    // perfectly intact — just not an ADF.
    let mut img = vec![0u8; DD];
    img[0..4].copy_from_slice(b"CAPS");
    let path = write_temp("preserved.adf", &img);

    match Container::open(&path) {
        Err(Error::UnsupportedContainer { format, detail }) => {
            assert_eq!(format, "IPF");
            assert!(
                detail.contains("Software Preservation Society"),
                "the detail must say what an IPF is: {detail}"
            );
        }
        other => panic!("expected UnsupportedContainer, got {other:?}"),
    }
}

#[test]
fn a_high_density_image_is_unsupported_rather_than_corrupt() {
    // 1.76 MB of Amiga disk. Real media, and this crate cannot read it yet —
    // which is not the same statement as "your disk is damaged".
    let path = write_temp("hd.adf", &vec![0u8; DD * 2]);

    match Container::open(&path) {
        Err(Error::UnsupportedContainer { format, detail }) => {
            assert_eq!(format, "HD ADF");
            assert!(detail.contains("high-density"), "{detail}");
        }
        other => panic!("expected UnsupportedContainer, got {other:?}"),
    }
}

#[test]
fn a_disk_lists_its_files_by_full_path_and_reads_each_one_back() {
    // `master` lays down exactly the shape wanted here: the executable at the
    // root, and `s/startup-sequence` one directory down. Building the fixture
    // with the same crate that reads it keeps media out of the repository.
    let exe = b"\x00\x00\x03\xf3 a very small hunk executable".repeat(4);
    let img = format198x_commodore_amiga_adf::master(&exe, "demo", "Music").unwrap();
    assert_eq!(img.len(), DD, "master must produce a DD image");
    let path = write_temp("music.adf", &img);

    let c = Container::open(&path).unwrap();
    let mut listed: Vec<_> = c
        .entries()
        .unwrap()
        .into_iter()
        .map(|e| (e.path, e.len))
        .collect();
    listed.sort();

    assert_eq!(
        listed,
        vec![
            ("demo".to_owned(), exe.len() as u64),
            // `master` writes the command name and a newline.
            ("s/startup-sequence".to_owned(), 5),
        ]
    );
    assert_eq!(c.read("demo").unwrap(), exe);
    assert_eq!(c.read("s/startup-sequence").unwrap(), b"demo\n");
}

#[test]
fn a_directory_on_a_disk_is_neither_listed_nor_readable() {
    let img = format198x_commodore_amiga_adf::master(b"payload", "demo", "Music").unwrap();
    let file = write_temp("dirs.adf", &img);
    let c = Container::open(&file).unwrap();

    let names: Vec<_> = c.entries().unwrap().into_iter().map(|e| e.path).collect();
    assert!(
        !names.contains(&"s".to_owned()),
        "listed a directory: {names:?}"
    );

    match c.read("s") {
        Err(Error::NoSuchEntry { path }) => assert_eq!(path, "s"),
        other => panic!("expected NoSuchEntry, got {other:?}"),
    }
}

#[test]
fn reading_a_path_the_disk_does_not_hold_names_it() {
    let img = format198x_commodore_amiga_adf::master(b"payload", "demo", "Music").unwrap();
    let file = write_temp("named.adf", &img);
    let c = Container::open(&file).unwrap();

    match c.read("mods/nope.mod") {
        Err(Error::NoSuchEntry { path }) => assert_eq!(path, "mods/nope.mod"),
        other => panic!("expected NoSuchEntry, got {other:?}"),
    }
}

/// A disk image is recognised by its length, not by what the file is called —
/// the same rule the ZIP path follows with its signature.
#[test]
fn a_disk_image_named_anything_is_still_a_disk_image() {
    let img = format198x_commodore_amiga_adf::master(b"payload", "demo", "Music").unwrap();
    let file = write_temp("collection.mod", &img);
    let c = Container::open(&file).unwrap();

    assert_eq!(c.read("demo").unwrap(), b"payload");
}

#[test]
fn a_hostile_disk_image_never_panics() {
    let good = format198x_commodore_amiga_adf::master(b"payload", "demo", "Music").unwrap();
    // The boot block and the root block are where every structural claim
    // lives; corrupting bytes anywhere else only damages file data. Every
    // outcome must be a value or a typed error.
    const ROOT: usize = 880 * 512;
    let sites = (0..24).chain(ROOT..ROOT + 24).chain(ROOT + 488..ROOT + 512);
    for at in sites {
        let mut bad = good.clone();
        bad[at] ^= 0xFF;
        let path = write_temp("fuzzed.adf", &bad);
        let Ok(c) = Container::open(&path) else {
            continue;
        };
        for entry in c.entries().unwrap_or_default() {
            match c.read(&entry.path) {
                Ok(_) => {}
                Err(
                    Error::Container { .. }
                    | Error::Io(_)
                    | Error::NoSuchEntry { .. }
                    | Error::NotAFilesystem
                    | Error::TooLarge { .. }
                    | Error::UnsupportedContainer { .. },
                ) => {}
                Err(other) => panic!("unexpected error for byte {at}: {other:?}"),
            }
        }
    }
}

#[test]
fn a_disk_whose_directories_point_at_themselves_is_refused_rather_than_walked_forever() {
    let mut volume = format198x_commodore_amiga_adf::Volume::new(
        "Loop",
        format198x_commodore_amiga_adf::FileSystem::Ofs,
    );
    volume.add_file("a/tune.mod", b"x").unwrap();
    let mut img = volume.build().unwrap();

    // Point every hash slot of directory `a` back at `a` itself. Nothing in
    // the read path checksums a directory header, so this is a disk the ADF
    // crate opens and lists quite happily — and walking it never ends.
    let block = find_header_block(&img, "a", DIRECTORY);
    for slot in 0..72 {
        let at = block * 512 + 24 + 4 * slot;
        img[at..at + 4].copy_from_slice(&(block as u32).to_be_bytes());
    }
    let file = write_temp("loop.adf", &img);
    let c = Container::open(&file).unwrap();

    match c.entries() {
        Err(Error::Container { what }) => assert!(
            what.contains("does not terminate"),
            "the refusal must say why it stopped: {what}"
        ),
        other => panic!("expected Container, got {other:?}"),
    }
}

/// The secondary type at a header block's end: 2 for a directory, -3 for a
/// file.
const DIRECTORY: u32 = 2;
const FILE: u32 = 0xFFFF_FFFD;

/// The block number of the header named `name` and of kind `sec_type`. Header
/// blocks carry block type 2 at offset 0 and their secondary type at the
/// block's end, with the name length 80 bytes from that end.
fn find_header_block(img: &[u8], name: &str, sec_type: u32) -> usize {
    let u32_at = |b: &[u8], at: usize| u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    for block in 2..1760 {
        let b = &img[block * 512..][..512];
        if u32_at(b, 0) != 2 || u32_at(b, 508) != sec_type {
            continue;
        }
        let len = usize::from(b[432]);
        if &b[433..433 + len] == name.as_bytes() {
            return block;
        }
    }
    panic!("no header named {name} on the image");
}

/// Recompute an Amiga block's checksum, so a rewritten header is still a
/// header the ADF crate will read rather than a damaged block.
///
/// The 128 big-endian words of a block sum to zero; the word at offset 20 is
/// what makes up the difference.
fn fix_checksum(img: &mut [u8], block: usize) {
    let b = &mut img[block * 512..][..512];
    b[20..24].copy_from_slice(&0u32.to_be_bytes());
    let (words, _) = b.as_chunks::<4>();
    let sum = words.iter().fold(0u32, |acc, word| {
        acc.wrapping_add(u32::from_be_bytes(*word))
    });
    b[20..24].copy_from_slice(&sum.wrapping_neg().to_be_bytes());
}

#[test]
fn a_disk_file_declaring_more_than_the_cap_is_refused_by_its_declaration() {
    const CAP: u64 = 16 * 1024 * 1024;
    const DECLARED: u32 = 16 * 1024 * 1024 + 4096;

    let mut img = format198x_commodore_amiga_adf::master(b"payload", "demo", "Music").unwrap();
    // A file header's byte-size field is four bytes the disk gets to state,
    // and `Disk::read` reserves that much before reading a sector. An 880K
    // disk plainly cannot hold sixteen mebibytes, which is the whole reason
    // the claim is checked rather than allocated on trust.
    let block = find_header_block(&img, "demo", FILE);
    img[block * 512 + 324..][..4].copy_from_slice(&DECLARED.to_be_bytes());
    fix_checksum(&mut img, block);
    let file = write_temp("liar.adf", &img);

    let c = Container::open(&file).unwrap();
    let entries = c.entries().unwrap();
    let demo = entries.iter().find(|e| e.path == "demo").unwrap();
    assert_eq!(
        demo.len,
        u64::from(DECLARED),
        "the disk has to be the one lying, or the test proves nothing"
    );

    match c.read("demo") {
        Err(Error::TooLarge { path, len, limit }) => {
            assert_eq!(path, "demo");
            assert_eq!(len, u64::from(DECLARED));
            assert_eq!(limit, CAP);
        }
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// PowerPacker
// ---------------------------------------------------------------------------

/// Emit a valid PP20 stream holding `plain` as a single literal run.
///
/// PowerPacker's compressor was never released and no port of it survives, so
/// this cannot compress: the stream it produces is *larger* than its input and
/// contains no back-references. It is nonetheless a real PP20 stream — the
/// format allows a run of literals with no match after it — and the real
/// decruncher is what reads it back, so the behaviour under test is the
/// shipping one rather than a stub.
///
/// The bitstream runs backwards: the decruncher reads the crunched body from
/// its last byte toward its first, LSB-first within each byte, and fills its
/// output from the end toward the start. So the bits are collected in read
/// order and packed into bytes in reverse, and the literals are emitted in
/// reverse order of the input.
fn powerpack(plain: &[u8]) -> Vec<u8> {
    assert!(!plain.is_empty(), "an empty run has nothing to encode");
    assert!(
        plain.len() <= 0xFF_FFFF,
        "the 3-byte trailer cannot declare {} bytes",
        plain.len()
    );

    let mut bits = Vec::new();
    push_bits(&mut bits, 0, 1); // a clear bit: a literal run follows

    // The run length starts at 1 and grows by 2-bit chunks; a chunk of 3
    // continues, anything else ends it.
    let mut remaining = plain.len() as u32 - 1;
    while remaining >= 3 {
        push_bits(&mut bits, 3, 2);
        remaining -= 3;
    }
    push_bits(&mut bits, remaining, 2);

    for byte in plain.iter().rev() {
        push_bits(&mut bits, u32::from(*byte), 8);
    }
    // With the whole output written, the decruncher stops before reading a
    // match, so nothing follows the run.

    // The body is whole bytes, and the trailer's low byte says how many bits
    // of leading padding to discard.
    let skip = (8 - bits.len() % 8) % 8;
    let mut all = vec![false; skip];
    all.extend(bits);

    let count = all.len() / 8;
    let mut body = vec![0u8; count];
    for (i, bit) in all.iter().enumerate() {
        if *bit {
            body[count - 1 - i / 8] |= 1 << (i % 8);
        }
    }

    let len = plain.len();
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(&format198x_commodore_amiga_powerpacker::MAGIC);
    out.extend_from_slice(&[9, 10, 12, 13]); // the offset widths real files use
    out.extend_from_slice(&body);
    out.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8, skip as u8]);
    out
}

/// Append `width` bits of `value`, most significant first — the order
/// `decrunch` reassembles them in.
fn push_bits(bits: &mut Vec<bool>, value: u32, width: u32) {
    for i in (0..width).rev() {
        bits.push((value >> i) & 1 == 1);
    }
}

#[test]
fn the_test_helper_emits_a_stream_the_real_decruncher_accepts() {
    // The helper is a fixture, so it is pinned itself: were it to emit
    // something PP20-shaped but wrong, every test below would fail for a
    // reason that had nothing to do with the container layer.
    for plain in [&b"x"[..], &b"ab"[..], &b"abcd"[..], &vec![0xA5; 1000][..]] {
        let packed = powerpack(plain);
        assert!(format198x_commodore_amiga_powerpacker::is_powerpacked(
            &packed
        ));
        assert_eq!(
            format198x_commodore_amiga_powerpacker::decrunch(&packed).unwrap(),
            plain,
            "round trip failed for {} bytes",
            plain.len()
        );
    }
}

#[test]
fn a_powerpacked_entry_reads_back_decrunched_from_every_container() {
    let plain = b"the quick brown fox ".repeat(64);
    let packed = powerpack(&plain);
    assert!(format198x_commodore_amiga_powerpacker::is_powerpacked(
        &packed
    ));
    assert_ne!(packed, plain, "the fixture must actually be crunched");

    // Loose on the filesystem.
    let loose_file = write_temp("tune.mod", &packed);
    let loose = Container::open(&loose_file).unwrap();
    assert_eq!(loose.read("tune.mod").unwrap(), plain);

    // Inside a ZIP.
    let zip_file = write_temp("tunes.zip", &build_zip("tune.mod", &packed));
    let zipped = Container::open(&zip_file).unwrap();
    assert_eq!(zipped.read("tune.mod").unwrap(), plain);

    // Inside a disk image, one directory down.
    let mut volume = format198x_commodore_amiga_adf::Volume::new(
        "Music",
        format198x_commodore_amiga_adf::FileSystem::Ofs,
    );
    volume.add_file("mods/tune.mod", &packed).unwrap();
    let img = volume.build().unwrap();
    let disk_file = write_temp("tunes.adf", &img);
    let disk = Container::open(&disk_file).unwrap();
    assert_eq!(disk.read("mods/tune.mod").unwrap(), plain);

    // The declared length stays the crunched one: it says what reading costs,
    // not what the bytes become.
    assert_eq!(
        disk.entries().unwrap(),
        vec![play198x_core::container::Entry {
            path: "mods/tune.mod".to_owned(),
            len: packed.len() as u64,
        }]
    );
}

#[test]
fn a_damaged_powerpacked_entry_is_a_typed_error_rather_than_a_panic() {
    let mut packed = powerpack(&b"the quick brown fox ".repeat(8));
    // Keep the magic — so decrunching is still attempted — and ruin the
    // offset-width table behind it.
    packed[4..8].copy_from_slice(&[0, 0, 0, 0]);
    let file = write_temp("broken.mod", &packed);
    let c = Container::open(&file).unwrap();

    match c.read("broken.mod") {
        Err(Error::Container { what }) => assert!(
            what.contains("PowerPacked"),
            "the refusal must say the entry was PowerPacked: {what}"
        ),
        other => panic!("expected Container, got {other:?}"),
    }
}

#[test]
fn a_powerpacked_entry_claiming_the_largest_output_the_format_allows_is_a_typed_error() {
    let mut packed = powerpack(b"small");
    // The largest a three-byte trailer can declare: 16,777,215 bytes, from a
    // stream of a few dozen. The decruncher allocates that before reading a
    // bit, then runs out of input — which must arrive as an error naming the
    // entry, not as a panic and not as sixteen megabytes of zeros.
    let at = packed.len() - 4;
    packed[at..at + 3].copy_from_slice(&[0xFF, 0xFF, 0xFF]);
    let file = write_temp("bomb.mod", &packed);
    let c = Container::open(&file).unwrap();

    match c.read("bomb.mod") {
        Err(Error::Container { what }) => assert!(
            what.contains("bomb.mod") && what.contains("PowerPacked"),
            "the refusal must name the entry and why: {what}"
        ),
        other => panic!("expected Container, got {other:?}"),
    }
}

/// A ZIP holding one file, DEFLATEd, built by the same crate the reader uses.
fn build_zip(name: &str, bytes: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer.start_file(name, options).unwrap();
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap().into_inner()
}

// ---------------------------------------------------------------------------

#[test]
#[ignore = "needs a real Amiga music disk; set PLAY198X_ADF"]
fn reads_a_real_music_disk() {
    // The plan's sketch returned quietly when the variable was unset. It must
    // not: this test only runs when somebody asks for it by name, so a missing
    // path is a mistyped invocation, and a pass would say the disk was read.
    let path = match std::env::var("PLAY198X_ADF") {
        Ok(path) => PathBuf::from(path),
        Err(err) => panic!("PLAY198X_ADF must name a real .adf ({err})"),
    };

    let c = Container::open(&path).unwrap();
    let entries = c.entries().unwrap();
    println!("{} holds {} entries:", path.display(), entries.len());
    let mut read = 0usize;
    for entry in &entries {
        let bytes = c.read(&entry.path).unwrap();
        println!(
            "  {} — {} bytes on disk, {} read",
            entry.path,
            entry.len,
            bytes.len()
        );
        read += 1;
    }
    assert!(read > 0, "{} listed no entries at all", path.display());
}
