//! Real-repo smoke tests.
//!
//! These tests clone public repositories and run `ifttt-lint` against them.
//! They require network access and significant disk space, so they are marked
//! `#[ignore]` and excluded from normal CI. Run with:
//!
//! ```sh
//! cargo smoke              # all repos
//! cargo smoke chromium     # just chromium
//! cargo smoke tensorflow   # just tensorflow
//! ```
//!
//! Clones are cached under `target/smoke/` to avoid re-downloading
//! on subsequent runs. Run `cargo clean` or delete the directory to force
//! a fresh clone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::Command as AssertCmd;

const THREAD_COUNTS: &[usize] = &[1, 2, 4, 8, 16];

/// Clone (or reuse cached clone of) a repo under `target/smoke/<name>`.
fn ensure_repo(name: &str, url: &str) -> PathBuf {
    let cache_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/smoke");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let repo_path = cache_dir.join(name);

    if repo_path.join(".git").exists() {
        eprintln!("using cached clone at {}", repo_path.display());
        return repo_path;
    }

    eprintln!("cloning {name} (shallow)...");
    let status = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(&repo_path)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()
        .expect("failed to spawn git clone");
    assert!(status.success(), "git clone failed");

    repo_path
}

/// Find files containing LINT directives via `git grep`.
fn find_lint_files(repo_path: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["grep", "-l", r"LINT\."])
        .current_dir(repo_path)
        .output()
        .expect("failed to spawn git grep");
    assert!(
        output.status.success(),
        "git grep failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(String::from)
        .collect()
}

/// Run ifttt-lint and return (exit code, elapsed time).
fn run_lint(
    repo_path: &Path,
    threads: usize,
    extra_args: &[&str],
    files: &[String],
) -> (i32, Duration) {
    let start = Instant::now();
    let result = AssertCmd::cargo_bin("ifttt-lint")
        .unwrap()
        .current_dir(repo_path)
        .args(["--threads", &threads.to_string(), "--strict=false"])
        .args(extra_args)
        .args(files)
        .output()
        .expect("failed to spawn ifttt-lint");
    let elapsed = start.elapsed();
    let code = result.status.code().unwrap_or(-1);
    (code, elapsed)
}

/// Print a thread-scaling table and assert times are non-pathological.
fn print_scaling_table(name: &str, results: &[(usize, i32, Duration)]) {
    let baseline = results[0].2.as_secs_f64();

    eprintln!();
    eprintln!("  {name} — thread scaling");
    eprintln!("  {:-<42}", "");
    eprintln!("  {:>7}  {:>10}  {:>8}", "threads", "time", "speedup");
    eprintln!("  {:-<42}", "");
    for &(threads, _, elapsed) in results {
        let secs = elapsed.as_secs_f64();
        let speedup = baseline / secs;
        eprintln!("  {:>7}  {:>9.2}s  {:>7.2}x", threads, secs, speedup);
    }
    eprintln!("  {:-<42}", "");
    eprintln!();
}

#[test]
#[ignore]
fn chromium_structural_validation() {
    let repo_path = ensure_repo("chromium", "https://github.com/chromium/chromium");

    let files = find_lint_files(&repo_path);
    assert!(!files.is_empty(), "expected LINT directives in Chromium");
    eprintln!("directive files found: {}", files.len());

    let extra = &[
        "--ignore",
        "depot/*",
        "--ignore",
        "<INTERNAL>/*",
        "--ignore",
        "<ROOT_DIR>/*",
    ];

    let mut results = Vec::new();
    for &threads in THREAD_COUNTS {
        let (code, elapsed) = run_lint(&repo_path, threads, extra, &files);
        assert!(
            code == 0 || code == 1,
            "expected exit code 0 or 1, got {code} (threads={threads})",
        );
        results.push((threads, code, elapsed));
    }

    print_scaling_table("Chromium", &results);
}

#[test]
#[ignore]
fn tensorflow_structural_validation() {
    let repo_path = ensure_repo("tensorflow", "https://github.com/tensorflow/tensorflow");

    let files = find_lint_files(&repo_path);
    assert!(!files.is_empty(), "expected LINT directives in TensorFlow");
    eprintln!("directive files found: {}", files.len());

    let mut results = Vec::new();
    for &threads in THREAD_COUNTS {
        let (code, elapsed) = run_lint(&repo_path, threads, &[], &files);
        assert!(
            code == 0 || code == 1,
            "expected exit code 0 or 1, got {code} (threads={threads})",
        );
        results.push((threads, code, elapsed));
    }

    print_scaling_table("TensorFlow", &results);
}
