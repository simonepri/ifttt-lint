use super::*;
use tempfile::TempDir;

// ─── three_dot_to_log_range ───

#[test]
fn three_dot_to_log_range_basic() {
    assert_eq!(three_dot_to_log_range("main...HEAD"), "main..HEAD");
}

#[test]
fn three_dot_to_log_range_no_separator() {
    assert_eq!(three_dot_to_log_range("HEAD"), "HEAD");
}

#[test]
fn three_dot_to_log_range_two_dot_unchanged() {
    // two-dot range has no "...", so it passes through unmodified
    assert_eq!(three_dot_to_log_range("main..HEAD"), "main..HEAD");
}

#[test]
fn three_dot_to_log_range_rightmost_wins() {
    // rsplit_once converts only the rightmost "..."
    assert_eq!(three_dot_to_log_range("a...b...c"), "a...b..c");
}

#[test]
fn three_dot_to_log_range_sha_refs() {
    assert_eq!(
        three_dot_to_log_range("abc1234...def5678"),
        "abc1234..def5678"
    );
}

// ─── resolve_input: file path ───

#[test]
fn resolve_input_reads_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.patch");
    std::fs::write(&path, "diff content").unwrap();
    let result = resolve_input(&Some(path.to_str().unwrap().to_string())).unwrap();
    assert_eq!(result.diff, "diff content");
}

#[test]
fn resolve_input_file_empty() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("empty.patch");
    std::fs::write(&path, "").unwrap();
    let result = resolve_input(&Some(path.to_str().unwrap().to_string())).unwrap();
    assert_eq!(result.diff, "");
}

// ─── resolve_input: unknown argument ───

#[test]
fn resolve_input_unknown_arg_is_error() {
    let result = resolve_input(&Some("not-a-file-or-range".to_string()));
    let err = result.unwrap_err();
    assert!(
        err.contains("not-a-file-or-range"),
        "error should mention the bad argument: {err}"
    );
}

#[test]
fn resolve_input_unknown_arg_error_hints_syntax() {
    let result = resolve_input(&Some("bogus".to_string()));
    let err = result.unwrap_err();
    assert!(
        err.contains("BASE...HEAD"),
        "error should hint at correct syntax: {err}"
    );
}
