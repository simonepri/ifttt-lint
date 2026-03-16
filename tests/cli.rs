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

    fn run(&self) -> AssertCmd {
        let mut cmd = AssertCmd::cargo_bin("ifttt-lint").unwrap();
        cmd.current_dir(self.dir.path());
        cmd.args(["--threads", "1"]);
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

// ---------------------------------------------------------------------------
// Smoke tests
// ---------------------------------------------------------------------------

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

    // Without ignore: should fail
    repo.run()
        .args(["--diff", "HEAD~1..HEAD"])
        .assert()
        .failure()
        .code(1);

    // With glob ignore: should pass
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
    // Use bare paths in ThenChange — only valid with --strict=false.
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

    // Without --strict=false: bare path is unrecognised and reported as error.
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

    // With --strict=false: bare path resolves and the missing change in b.rs
    // is flagged as a finding.
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

#[test]
fn non_strict_double_slash_still_works() {
    let repo = TestRepo::new();
    // Standard // paths should work identically with --strict=false.
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
        .args(["--diff", "HEAD~1..HEAD", "--strict=false"])
        .assert()
        .success();
}
