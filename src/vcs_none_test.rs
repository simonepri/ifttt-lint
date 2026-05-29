use yare::parameterized;

use super::*;
use crate::vcs::{FileFilter, FilePattern};

#[parameterized(
    existing_file  = { Some("world\n"), Some("world\n") },
    missing_file   = { None,            None            },
)]
fn read_file(content: Option<&str>, expected: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(c) = content {
        std::fs::write(dir.path().join("f.txt"), c).unwrap();
    }
    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    let result = vcs.read_file("f.txt").unwrap();
    assert_eq!(result.as_ref().and_then(FileContent::as_text), expected);
}

#[test]
fn file_exists_present_and_absent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("present.txt"), "").unwrap();
    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    assert!(vcs.file_exists("present.txt").unwrap());
    assert!(!vcs.file_exists("absent.txt").unwrap());
}

#[test]
fn read_file_utf8_boundary() {
    let dir = tempfile::tempdir().unwrap();
    // Create a string that puts the start of a multi-byte character exactly at byte index 8192.
    // 8191 bytes of 'a' means the 8192nd byte is the start of the next character.
    let mut content = "a".repeat(8191);
    content.push('🚀'); // multi-byte character

    // The chunk read by `read_file` internally evaluates the first 8192 bytes.
    // This perfectly cuts off the emoji, yielding an incomplete UTF-8 error.
    std::fs::write(dir.path().join("f.txt"), &content).unwrap();

    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    // The `read_file` method should NOT treat it as a binary file,
    // and should successfully return the full file content.
    let result = vcs.read_file("f.txt").unwrap();
    assert_eq!(
        result.as_ref().and_then(FileContent::as_text),
        Some(content.as_str()),
    );
}

#[test]
fn read_file_invalid_utf8_after_probe() {
    let dir = tempfile::tempdir().unwrap();
    // First 8192 bytes are valid ASCII, then invalid UTF-8 follows.
    // The 8192-byte probe passes (valid text, no NUL), but the full
    // file is not valid UTF-8 — read_file must surface the error
    // instead of silently returning an empty string.
    let mut content = vec![b'a'; 8192];
    content.extend_from_slice(&[0xFF, 0xFE]);
    std::fs::write(dir.path().join("f.txt"), &content).unwrap();

    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    assert!(
        vcs.read_file("f.txt").is_err(),
        "should report error for invalid UTF-8 after probe window"
    );
}

#[test]
fn diff_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    assert!(vcs.diff().is_err());
}

#[test]
fn suppressions_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    assert!(vcs.suppressions().is_err());
}

#[test]
fn search_string_in_files_is_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let vcs = NoneVcsProvider::new(dir.path().to_path_buf(), true, vec![]);
    assert!(vcs
        .search_string_in_files("LINT.", &FileFilter::all())
        .is_err());
    assert!(vcs
        .search_string_in_files(
            "LINT.",
            &FileFilter::any(vec![FilePattern::Contains("x.rs")])
        )
        .is_err());
}
