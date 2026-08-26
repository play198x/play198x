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
