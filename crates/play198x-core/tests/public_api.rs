#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn error_display_distinguishes_a_non_dos_disk_from_a_damaged_one() {
    let not_fs = play198x_core::Error::NotAFilesystem.to_string();
    let damaged = play198x_core::Error::Container {
        what: "root block".into(),
    }
    .to_string();
    assert_ne!(not_fs, damaged);
    assert!(
        !not_fs.to_lowercase().contains("corrupt"),
        "a non-DOS disk must not be described as corrupt: {not_fs}"
    );
}

#[test]
fn a_refused_oversized_entry_does_not_read_as_damage() {
    // The exact wording matters: an archive holding something bigger than this
    // crate will read is not a damaged archive, and calling it one sends the
    // reader hunting corruption that is not there.
    let refused = play198x_core::Error::TooLarge {
        path: "payload.bin".into(),
        len: 419_430_400,
        limit: 16_777_216,
    }
    .to_string();

    assert_eq!(
        refused,
        "`payload.bin` is 419430400 bytes, past the 16777216-byte limit this crate will read"
    );
    assert!(
        !refused.contains("damage"),
        "a refusal must not be reported as damage: {refused}"
    );
}
