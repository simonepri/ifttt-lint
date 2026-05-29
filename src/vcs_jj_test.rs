use std::path::Path;
use std::process::Stdio;

use yare::parameterized;

use super::*;
use crate::vcs::{FileFilter, FilePattern};

fn jj(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("jj")
        .args(args)
        .current_dir(dir)
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .expect("jj must be available for integration tests; run `mise install`");
    assert!(status.success(), "jj {args:?} failed");
}

fn jj_output(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("jj")
        .args(args)
        .current_dir(dir)
        .stderr(Stdio::null())
        .output()
        .expect("jj must be available for integration tests; run `mise install`");
    assert!(output.status.success(), "jj {args:?} failed");
    String::from_utf8(output.stdout).unwrap()
}

/// Initialise a fresh jj repo. Configures user identity per-repo so commits
/// don't emit the "empty identity" warning in test output.
fn init_repo(dir: &Path) {
    jj(dir, &["git", "init"]);
    jj(
        dir,
        &["config", "set", "--repo", "user.email", "test@example.com"],
    );
    jj(dir, &["config", "set", "--repo", "user.name", "Test"]);
}

/// Set the working-copy commit's description and seal it: after this, a new
/// empty working-copy commit sits on top, so the described commit is
/// addressable as `@-`.
fn describe_and_advance(dir: &Path, msg: &str) {
    jj(dir, &["describe", "-m", msg]);
    jj(dir, &["new"]);
}

fn commit_id(dir: &Path, revset: &str) -> String {
    jj_output(
        dir,
        &[
            "--no-pager",
            "log",
            "--no-graph",
            "-T",
            "commit_id",
            "-r",
            revset,
        ],
    )
    .trim()
    .to_string()
}

#[test]
fn search_files_lint_finds_matching() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "LINT.IfChange\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "nothing\n").unwrap();

    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let mut found = vcs
        .search_string_in_files("LINT.", &FileFilter::all())
        .unwrap();
    found.sort();
    assert_eq!(found, vec!["a.txt".to_string()]);
}

#[test]
fn search_files_lint_empty_when_no_match() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("x.txt"), "nothing\n").unwrap();

    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let found = vcs
        .search_string_in_files("LINT.", &FileFilter::all())
        .unwrap();
    assert!(found.is_empty(), "expected no matches, got: {found:?}");
}

#[test]
fn search_files_filter_any_uses_or_semantics() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    for (name, content) in [
        ("a.txt", "LINT.\n//x.rs\n"),
        ("b.txt", "LINT.\n//y.rs\n"),
        ("c.txt", "LINT.\n//z.rs\n"),
    ] {
        std::fs::write(dir.path().join(name), content).unwrap();
    }

    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let mut found = vcs
        .search_string_in_files(
            "LINT.",
            &FileFilter::any(vec![
                FilePattern::Contains("x.rs"),
                FilePattern::Contains("y.rs"),
            ]),
        )
        .unwrap();
    found.sort();
    assert_eq!(found, vec!["a.txt".to_string(), "b.txt".to_string()]);
}

/// Locks in the two-automaton design: if anyone collapses needle + filter
/// into a single combined automaton, `MatchKind::Standard`'s non-overlapping
/// advance will commit the needle hit at byte 0 and skip past the overlapping
/// filter pattern, dropping this file from the result set.
#[test]
fn search_files_filter_pattern_with_needle_prefix_still_hits() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("a.txt"), "LINT.IfChange\n").unwrap();

    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let found = vcs
        .search_string_in_files(
            "LINT.",
            &FileFilter::any(vec![FilePattern::Contains("LINT.IfChange")]),
        )
        .unwrap();
    assert_eq!(
        found,
        vec!["a.txt".to_string()],
        "filter pattern overlapping the needle must still match"
    );
}

#[test]
fn search_files_skips_binary() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    // A binary file containing the needle string + a NUL byte in the first
    // 8 KB. The needle should NOT cause this file to be reported.
    let mut bin = b"prefix LINT. suffix".to_vec();
    bin.push(0);
    bin.extend_from_slice(b"more bytes after the NUL\n");
    std::fs::write(dir.path().join("bin.dat"), &bin).unwrap();
    std::fs::write(dir.path().join("text.txt"), "LINT.IfChange\n").unwrap();

    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let found = vcs
        .search_string_in_files("LINT.", &FileFilter::all())
        .unwrap();
    assert_eq!(
        found,
        vec!["text.txt".to_string()],
        "binary file with NUL byte must be skipped"
    );
}

#[test]
fn search_files_skips_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::write(dir.path().join("real.txt"), "LINT.IfChange\n").unwrap();
    // Symlink whose target name contains the needle string. The needle would
    // hit if we read through the symlink, but git grep skips symlinks and
    // we mirror that.
    std::os::unix::fs::symlink("LINT.IfChange", dir.path().join("link.txt")).unwrap();

    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), None, true, vec![]);
    let found = vcs
        .search_string_in_files("LINT.", &FileFilter::all())
        .unwrap();
    assert_eq!(
        found,
        vec!["real.txt".to_string()],
        "symlinks must not be searched"
    );
}

#[test]
fn diff_with_explicit_revset_reports_added_lines() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    std::fs::write(dir.path().join("f.txt"), "line1\n").unwrap();
    describe_and_advance(dir.path(), "baseline");
    let base = commit_id(dir.path(), "@-");

    std::fs::write(dir.path().join("f.txt"), "line1\nline2\n").unwrap();
    describe_and_advance(dir.path(), "add line2");
    let head = commit_id(dir.path(), "@-");

    let revset = format!("{base}..{head}");
    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), Some(revset), true, vec![]);
    let changes = vcs.diff().unwrap();
    let fc = changes.get("f.txt").expect("f.txt should appear in diff");
    assert!(fc.added_lines.contains(&2), "line 2 should be marked added");
}

#[test]
fn diff_skips_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    std::fs::write(dir.path().join("real.txt"), "content\n").unwrap();
    describe_and_advance(dir.path(), "baseline");
    let base = commit_id(dir.path(), "@-");

    // Replace real.txt with a symlink pointing to /etc/hostname-like target
    // (any target — content isn't dereferenced). The diff retain() filter
    // must drop entries whose on-disk path is now a symlink.
    std::fs::remove_file(dir.path().join("real.txt")).unwrap();
    std::os::unix::fs::symlink("/nonexistent", dir.path().join("real.txt")).unwrap();
    describe_and_advance(dir.path(), "swap to symlink");
    let head = commit_id(dir.path(), "@-");

    let revset = format!("{base}..{head}");
    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), Some(revset), true, vec![]);
    let changes = vcs.diff().unwrap();
    assert!(
        !changes.contains_key("real.txt"),
        "symlinked paths should be filtered from the diff, got: {changes:?}"
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
    describe_and_advance(dir.path(), "baseline");
    let base = commit_id(dir.path(), "@-");

    std::fs::write(dir.path().join("f.txt"), "v2\n").unwrap();
    describe_and_advance(dir.path(), commit_msg);
    let head = commit_id(dir.path(), "@-");

    let revset = format!("{base}..{head}");
    let vcs = JjVcsProvider::new(dir.path().to_path_buf(), Some(revset), true, vec![]);
    assert_eq!(vcs.suppressions().unwrap().as_deref(), expected);
}

#[test]
fn glob_expansion_matches_tracked_files() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());

    std::fs::write(dir.path().join("a.rs"), "// LINT.IfChange\n").unwrap();
    std::fs::write(dir.path().join("b.rs"), "// other\n").unwrap();
    std::fs::write(dir.path().join("readme.md"), "docs\n").unwrap();
    describe_and_advance(dir.path(), "baseline");

    let vcs = JjVcsProvider::new(
        dir.path().to_path_buf(),
        None,
        true,
        vec![PathBuf::from("**/*.rs")],
    );
    let mut files = vcs.validate_files().to_vec();
    files.sort();
    assert_eq!(files, vec!["a.rs".to_string(), "b.rs".to_string()]);
}
