use std::process::Command;

use assert_cmd::Command as AssertCmd;
use tempfile::TempDir;
use unindent::unindent;

/// Disposable git repo for CLI smoke tests.
struct TestRepo {
    dir: TempDir,
}

impl TestRepo {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let repo = Self { dir };
        repo.git(&["init", "-b", "main"]);
        repo.git(&["config", "user.name", "test"]);
        repo.git(&["config", "user.email", "test@test.com"]);
        repo
    }

    fn write(&self, path: &str, content: &str) {
        let file_path = self.dir.path().join(path);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(file_path, unindent(content)).unwrap();
    }

    fn commit(&self, msg: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-m", msg]);
    }

    fn stage(&self, path: &str) {
        self.git(&["add", path]);
    }

    fn remove(&self, path: &str) {
        self.git(&["rm", path]);
    }

    fn run(&self) -> AssertCmd {
        self.run_with_threads(1)
    }

    fn run_with_threads(&self, n: usize) -> AssertCmd {
        let mut cmd = AssertCmd::cargo_bin("ifttt-lint").unwrap();
        cmd.current_dir(self.dir.path());
        cmd.args(["--threads", &n.to_string()]);
        cmd
    }

    fn git(&self, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

#[test]
fn no_directives_exits_zero() {
    let repo = TestRepo::new();
    repo.write("hello.rs", "fn main() {}\n");
    repo.commit("initial");

    repo.write("hello.rs", "fn main() { println!(\"hi\"); }\n");
    repo.commit("update");

    repo.run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .success();
}

#[test]
fn both_sides_changed_exits_zero() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "new_b\n");
    repo.commit("update both");

    repo.run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .success();
}

#[test]
fn missing_change_reports_finding() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("update a only");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("b.rs"),
        "expected finding mentioning b.rs, got: {stderr}",
    );
}

#[test]
fn unpaired_directive_reports_error() {
    let repo = TestRepo::new();
    repo.write(
        "bad.rs",
        "
        // LINT.IfChange
        some code
        ",
    );
    repo.commit("initial");

    repo.write(
        "bad.rs",
        "
        // LINT.IfChange
        changed code
        ",
    );
    repo.commit("update");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(!stderr.is_empty(), "expected error output on stderr");
}

#[test]
fn ignore_suppresses_finding() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("update a only");

    repo.run()
        .args(["--diff", "HEAD~1..HEAD", "--ignore", "b.rs"])
        .assert()
        .success();
}

#[test]
fn ignore_glob_pattern() {
    let repo = TestRepo::new();
    repo.write(
        "src/api.rs",
        "
        // LINT.IfChange
        old_api
        // LINT.ThenChange(//generated/client.rs)
        ",
    );
    repo.write("generated/client.rs", "old_client\n");
    repo.commit("initial");

    repo.write(
        "src/api.rs",
        "
        // LINT.IfChange
        new_api
        // LINT.ThenChange(//generated/client.rs)
        ",
    );
    repo.commit("update api only");

    repo.run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);

    repo.run()
        .args(["--diff", "HEAD~1..HEAD", "--ignore", "generated/**"])
        .assert()
        .success();
}

#[test]
fn file_list_validates_targets() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        code
        // LINT.ThenChange(//nonexistent.rs)
        ",
    );
    repo.commit("initial");

    let result = repo.run().arg("a.rs").assert().failure().code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("nonexistent.rs"),
        "expected error mentioning nonexistent.rs, got: {stderr}",
    );
}

#[test]
fn diff_ignores_structural_errors_in_untouched_target_files() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write(
        "b.rs",
        "
        // LINT.IfChange(bad)
        old_b
        ",
    );
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("update a only");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("may need to be reflected in b.rs"),
        "expected diff finding for unchanged b.rs, got: {stderr}",
    );
    assert!(
        !stderr.contains("LINT.IfChange without matching ThenChange"),
        "untouched target-file structural errors must not leak into diff mode: {stderr}",
    );
}

#[test]
fn json_format_produces_valid_json() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("update a only");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD", "--format", "json"])
        .assert()
        .failure()
        .code(1);
    let stdout = String::from_utf8_lossy(&result.get_output().stdout);
    assert!(!stdout.is_empty(), "expected JSON on stdout");
    serde_json::from_str::<serde_json::Value>(&stdout).expect("stdout should be valid JSON");
}

#[test]
fn non_strict_resolves_bare_targets() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(b.rs)
        ",
    );
    repo.commit("update a only");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("must start with //"),
        "expected prefix error without --strict=false, got: {stderr}",
    );

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD", "--strict=false"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("may need to be reflected in b.rs"),
        "expected cross-file finding for b.rs with --strict=false, got: {stderr}",
    );
}

// The pre-commit hook invokes `ifttt-lint <staged-files>`. When files are
// provided without --diff, only structural validation runs (are targets valid?
// do labels exist?). Co-change enforcement belongs to the push hook.

#[test]
fn precommit_skips_cochange_validation() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.stage("a.rs");

    repo.run().args(["a.rs", "b.rs"]).assert().success();
}

#[test]
fn diff_detects_stale_label_after_rename() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange(old_name)
        code
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "b\n");
    repo.write(
        "ref.rs",
        "
        // LINT.IfChange
        code
        // LINT.ThenChange(//a.rs:old_name)
        ",
    );
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange(new_name)
        code
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("rename label");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("old_name"),
        "expected finding mentioning old_name not found, got: {stderr}",
    );
}

// A commit message line `NO_IFTTT=<reason>` suppresses diff-based validation
// for the entire diff range. Deleted-file reverse lookup is NOT suppressed.
// Suppression is only active in --diff mode; staged (pre-commit) mode never
// scans commit messages.

#[test]
fn no_ifttt_suppresses_diff_finding() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("update a\n\nNO_IFTTT=docs follow in next PR");

    repo.run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .success();
}

#[test]
fn no_ifttt_in_any_commit_suppresses_entire_range() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.commit("update a");

    repo.write("c.rs", "unrelated\n");
    repo.commit("add c\n\nNO_IFTTT=intentional one-way sync");

    repo.run()
        .args(["--diff", "HEAD~2..HEAD"])
        .assert()
        .success();
}

#[test]
fn no_ifttt_does_not_suppress_deleted_file_finding() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        code
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "b\n");
    repo.commit("initial");

    repo.remove("b.rs");
    repo.commit("delete b.rs\n\nNO_IFTTT=intentional removal");

    let result = repo
        .run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);
    assert!(
        stderr.contains("b.rs"),
        "expected finding for deleted b.rs despite NO_IFTTT, got: {stderr}",
    );
}

#[test]
fn no_ifttt_inert_in_precommit_mode() {
    let repo = TestRepo::new();
    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        old_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.write("b.rs", "old_b\n");
    repo.commit("initial");

    repo.write("c.rs", "c\n");
    repo.commit("add c\n\nNO_IFTTT=pre-existing suppression");

    repo.write(
        "a.rs",
        "
        // LINT.IfChange
        new_a
        // LINT.ThenChange(//b.rs)
        ",
    );
    repo.stage("a.rs");

    repo.run().args(["a.rs", "b.rs"]).assert().success();
}

// The parallel passes (rayon) return findings in arbitrary order; check()
// must sort them before returning so the output is deterministic across runs.

#[test]
fn multi_thread_output_is_deterministically_sorted() {
    let repo = TestRepo::new();
    for (name, old) in [("c.rs", "old_c"), ("a.rs", "old_a"), ("b.rs", "old_b")] {
        repo.write(
            name,
            &format!(
                "
                // LINT.IfChange
                {old}
                // LINT.ThenChange(//target.rs)
                "
            ),
        );
    }
    repo.write("target.rs", "old_target\n");
    repo.commit("initial");

    for (name, new) in [("c.rs", "new_c"), ("a.rs", "new_a"), ("b.rs", "new_b")] {
        repo.write(
            name,
            &format!(
                "
                // LINT.IfChange
                {new}
                // LINT.ThenChange(//target.rs)
                "
            ),
        );
    }
    repo.commit("update all sources, not target");

    let result = repo
        .run_with_threads(4)
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);
    let stderr = String::from_utf8_lossy(&result.get_output().stderr);

    let a_pos = stderr.find("a.rs:").expect("finding for a.rs");
    let b_pos = stderr.find("b.rs:").expect("finding for b.rs");
    let c_pos = stderr.find("c.rs:").expect("finding for c.rs");
    assert!(
        a_pos < b_pos && b_pos < c_pos,
        "expected findings sorted a.rs < b.rs < c.rs, got:\n{stderr}",
    );
}

#[test]
fn symlink_files_are_skipped_in_structural_validation() {
    let repo = TestRepo::new();
    repo.write("real.rs", "content\n");
    std::os::unix::fs::symlink("real.rs", repo.dir.path().join("link.rs")).unwrap();
    repo.commit("initial");

    repo.run().args(["link.rs"]).assert().success();
}

#[test]
fn symlink_to_directory_is_skipped_in_glob_expansion() {
    let repo = TestRepo::new();
    std::fs::create_dir_all(repo.dir.path().join("subdir")).unwrap();
    repo.write("subdir/a.rs", "content\n");
    std::os::unix::fs::symlink("subdir", repo.dir.path().join("link")).unwrap();
    repo.commit("initial");

    repo.run().args(["*"]).assert().success();
}
