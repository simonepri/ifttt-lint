use std::path::Path;

use yare::parameterized;

use super::*;

// ─── Helpers ───

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

// ─── GitVcsProvider integration ───

#[parameterized(
    existing_file  = { Some("world\n"), Some("world\n") },
    missing_file   = { None,            None            },
)]
fn read_file(content: Option<&str>, expected: Option<&str>) {
    let dir = tempfile::tempdir().unwrap();
    if let Some(c) = content {
        std::fs::write(dir.path().join("f.txt"), c).unwrap();
    }
    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, true, vec![]);
    assert_eq!(vcs.read_file("f.txt").unwrap().as_deref(), expected);
}

#[test]
fn file_exists_present_and_absent() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("present.txt"), "").unwrap();
    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, true, vec![]);
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

    let vcs = GitVcsProvider::new(dir.path().to_path_buf(), None, true, true, vec![]);
    let mut found = vcs.search_files("LINT.").unwrap();
    found.sort();
    assert_eq!(found.as_slice(), expected);
}

#[test]
fn diff_with_explicit_range_reports_added_lines() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    // First commit — baseline.
    std::fs::write(dir.path().join("f.txt"), "line1\n").unwrap();
    git_add_commit(dir.path(), "baseline");

    // Second commit — add a line.
    std::fs::write(dir.path().join("f.txt"), "line1\nline2\n").unwrap();
    git_add_commit(dir.path(), "add line2");

    let vcs = GitVcsProvider::new(
        dir.path().to_path_buf(),
        Some("HEAD~1...HEAD".to_string()),
        true,
        true,
        vec![],
    );
    let changes = vcs.diff().unwrap();
    let fc = changes.get("f.txt").expect("f.txt should appear in diff");
    assert!(fc.added_lines.contains(&2), "line 2 should be marked added");
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
        true,
        vec![],
    );
    assert_eq!(vcs.suppressions().unwrap().as_deref(), expected);
}

// ─── three_dot_to_log_range ───

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
