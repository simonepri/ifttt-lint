use std::path::Path;

use yare::parameterized;

use super::*;
use crate::vcs::FileFilter;

/// Minimal user config so `git commit` succeeds in CI / sandboxed environments.
fn init_repo(dir: &Path) {
    for args in [
        vec!["init", "-b", "main"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test"],
    ] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir)
            .stderr(std::process::Stdio::null())
            .status()
            .expect("git must be available for integration tests");
        assert!(status.success(), "git {args:?} failed");
    }
}

fn git_add_commit(dir: &Path, msg: &str) {
    for args in [vec!["add", "."], vec!["commit", "-m", msg]] {
        let status = std::process::Command::new("git")
            .args(&args)
            .current_dir(dir)
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }
}

#[parameterized(
    existing_file  = { Some("world\n"), Some("world\n") },
    missing_file   = { None,            None            },
)]
fn read_file(content: Option<&str>, expected: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(c) = content {
        std::fs::write(dir.path().join("f.txt"), c).unwrap();
    }
    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let result = vcs.read_file("f.txt").unwrap();
    assert_eq!(result.as_ref().and_then(FileContent::as_text), expected);
}

#[test]
fn file_exists_present_and_absent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("present.txt"), "").unwrap();
    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    assert!(vcs.file_exists("present.txt").unwrap());
    assert!(!vcs.file_exists("absent.txt").unwrap());
}

#[parameterized(
    finds_matching     = { &[("a.txt", "LINT.IfChange\n"), ("b.txt", "nothing\n")], &["a.txt"] },
    empty_when_no_match = { &[("x.txt", "nothing\n")],                              &[]        },
)]
fn search_files_lint(files: &[(&str, &str)], expected: &[&str]) {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    for (name, content) in files {
        std::fs::write(dir.path().join(name), content).unwrap();
    }
    git_add_commit(dir.path(), "init");

    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let mut found = vcs
        .search_string_in_files("LINT.", &FileFilter::all())
        .unwrap();
    found.sort();
    assert_eq!(found.as_slice(), expected);
}

#[test]
fn diff_with_explicit_range_reports_added_lines() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    std::fs::write(dir.path().join("f.txt"), "line1\n").unwrap();
    git_add_commit(dir.path(), "baseline");

    std::fs::write(dir.path().join("f.txt"), "line1\nline2\n").unwrap();
    git_add_commit(dir.path(), "add line2");

    let vcs = GitVcsProvider::new(
        dir.path().to_path_buf(),
        Some("HEAD~1...HEAD".to_string()),
        true,
        vec![],
    );
    let changes = vcs.diff().unwrap();
    let fc = changes.get("f.txt").expect("f.txt should appear in diff");
    assert!(fc.added_lines.contains(&2), "line 2 should be marked added");
}

#[test]
fn three_dot_diff_ignores_base_only_changes() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    std::fs::write(dir.path().join("shared.txt"), "base\n").unwrap();
    git_add_commit(dir.path(), "base");

    let status = std::process::Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(dir.path())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git checkout feature failed");

    std::fs::write(dir.path().join("feature.txt"), "feature v1\n").unwrap();
    git_add_commit(dir.path(), "feature start");

    let status = std::process::Command::new("git")
        .args(["checkout", "main"])
        .current_dir(dir.path())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git checkout main failed");

    std::fs::write(dir.path().join("main_only.txt"), "main change\n").unwrap();
    git_add_commit(dir.path(), "main advances");

    let base_sha = {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "git rev-parse main failed");
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };

    let status = std::process::Command::new("git")
        .args(["checkout", "feature"])
        .current_dir(dir.path())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git checkout feature failed");

    std::fs::write(dir.path().join("feature.txt"), "feature v2\n").unwrap();
    git_add_commit(dir.path(), "feature update");

    let vcs = GitVcsProvider::new(
        dir.path().to_path_buf(),
        Some(format!("{base_sha}...HEAD")),
        true,
        vec![],
    );
    let changes = vcs.diff().unwrap();

    assert!(
        changes.contains_key("feature.txt"),
        "feature-side changes should appear in three-dot diff"
    );
    assert!(
        !changes.contains_key("main_only.txt"),
        "base-only changes must not appear in three-dot diff"
    );
}

#[parameterized(
    tag_present = { "feat: update\n\nNO_IFTTT=docs follow in next PR", Some("docs follow in next PR") },
    tag_absent  = { "ordinary commit message",                          None                           },
)]
fn suppressions(commit_msg: &str, expected: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("f.txt"), "v1\n").unwrap();
    git_add_commit(dir.path(), "baseline");
    std::fs::write(dir.path().join("f.txt"), "v2\n").unwrap();
    git_add_commit(dir.path(), commit_msg);

    let vcs = GitVcsProvider::new(
        dir.path().to_path_buf(),
        Some("HEAD~1...HEAD".to_string()),
        true,
        vec![],
    );
    assert_eq!(vcs.suppressions().unwrap().as_deref(), expected);
}

#[parameterized(
    basic           = { "main...HEAD",        "main..HEAD"      },
    no_separator    = { "HEAD",               "HEAD"            },
    two_dot_pass    = { "main..HEAD",         "main..HEAD"      },
    rightmost_wins  = { "a...b...c",          "a...b..c"        },
    sha_refs        = { "abc1234...def5678",  "abc1234..def5678"},
)]
fn three_dot_to_log_range_cases(input: &str, expected: &str) {
    assert_eq!(three_dot_to_log_range(input), expected);
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

    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
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

    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    assert!(
        vcs.read_file("f.txt").is_err(),
        "should report error for invalid UTF-8 after probe window"
    );
}
