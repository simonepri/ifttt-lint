use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::vcs::{ChangeMap, VcsProvider};

#[path = "udiff.rs"]
mod udiff;

pub struct GitVcsProvider {
    root: PathBuf,
    /// Explicit git ref range (e.g. `main...HEAD`). None = auto-detect.
    diff_range: Option<String>,
    /// TTY → auto-detect upstream range; non-TTY → staged changes (pre-commit).
    is_tty: bool,
    /// Cached so `diff()` and `suppressions()` share one `detect_range()` call.
    /// Stored as `Arc<anyhow::Error>` to preserve the full anyhow context chain.
    detected_range: std::sync::OnceLock<Result<String, std::sync::Arc<anyhow::Error>>>,
    /// When false, accept bare and single-`/` paths in ThenChange targets.
    strict: bool,
    files: Vec<String>,
}

impl GitVcsProvider {
    pub fn new(
        root: PathBuf,
        diff_range: Option<String>,
        is_tty: bool,
        strict: bool,
        files: Vec<PathBuf>,
    ) -> Self {
        let normalized = files
            .iter()
            .filter_map(|p| {
                if p.is_absolute() {
                    match p.strip_prefix(&root) {
                        Ok(rel) => Some(rel.to_string_lossy().replace('\\', "/")),
                        Err(_) => {
                            eprintln!(
                                "warning: '{}' is outside the project root and will be skipped",
                                p.display()
                            );
                            None
                        }
                    }
                } else {
                    Some(p.to_string_lossy().replace('\\', "/"))
                }
            })
            .collect();
        Self {
            root,
            diff_range,
            is_tty,
            detected_range: std::sync::OnceLock::new(),
            strict,
            files: normalized,
        }
    }

    fn get_or_detect_range(&self) -> Result<&str> {
        match self
            .detected_range
            .get_or_init(|| detect_range(&self.root).map_err(std::sync::Arc::new))
        {
            Ok(s) => Ok(s.as_str()),
            // Clone the Arc to preserve the full anyhow context chain.
            Err(e) => Err(anyhow::anyhow!(e.clone())),
        }
    }

    pub fn resolve_root() -> Result<PathBuf> {
        if let Some(root) = detect_root() {
            return Ok(root);
        }
        std::env::current_dir().context("failed to determine project root")
    }
}

impl VcsProvider for GitVcsProvider {
    fn diff(&self) -> Result<ChangeMap> {
        let raw = match &self.diff_range {
            Some(range) => git_diff(&self.root, range)?,
            None => {
                if self.is_tty {
                    git_diff(&self.root, self.get_or_detect_range()?)?
                } else {
                    // Non-TTY (pre-commit hook): use staged changes.
                    staged_diff(&self.root)?
                }
            }
        };
        udiff::parse(&mut std::io::Cursor::new(raw), strip_git_prefix).map_err(anyhow::Error::msg)
    }

    fn suppressions(&self) -> Result<Option<String>> {
        let log_range = match &self.diff_range {
            Some(range) => three_dot_to_log_range(range),
            None => {
                if self.is_tty {
                    three_dot_to_log_range(self.get_or_detect_range()?)
                } else {
                    // Staged diff has no commit range to scan.
                    return Ok(None);
                }
            }
        };
        Ok(parse_no_ifttt_from_commits(&self.root, &log_range))
    }

    // Uses raw FS — reads gitignored files too. See trait docs for rationale.
    fn read_file(&self, rel_path: &str) -> Result<Option<String>> {
        use std::io::Read;
        let abs = self.root.join(rel_path);
        let mut file = match std::fs::File::open(&abs) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(anyhow::anyhow!(e).context(format!("read {rel_path}"))),
        };
        // Probe first 8 KB to avoid loading large binary assets into memory.
        let mut probe = [0u8; 8192];
        let n = file
            .read(&mut probe)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        let head = &probe[..n];
        if head.contains(&0) || std::str::from_utf8(head).is_err() {
            return Ok(Some(String::new()));
        }
        let mut buf = Vec::from(head);
        file.read_to_end(&mut buf)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        match String::from_utf8(buf) {
            Ok(s) => Ok(Some(s)),
            Err(_) => Ok(Some(String::new())),
        }
    }

    // Uses raw FS — returns true for gitignored files. See trait docs for
    // the trade-off vs search_files (which uses git grep and skips them).
    fn file_exists(&self, rel_path: &str) -> Result<bool> {
        Ok(self.root.join(rel_path).exists())
    }

    fn try_resolve_path(&self, raw: &str) -> Result<String, String> {
        if self.strict {
            crate::vcs::default_resolve_path(raw)
        } else {
            crate::vcs::unsafe_resolve_path(raw)
        }
    }

    fn is_strict(&self) -> bool {
        self.strict
    }

    fn validate_files(&self) -> &[String] {
        &self.files
    }

    fn search_files(&self, query: &str) -> Result<Vec<String>> {
        let output = std::process::Command::new("git")
            .args(["grep", "-rl", "--fixed-strings", "--", query])
            .current_dir(&self.root)
            .output()
            .context("git grep")?;

        // git-grep(1) exit codes follow the POSIX grep(1) convention:
        //   0  — one or more lines matched
        //   1  — no lines matched (not an error)
        //   2+ — an actual error (bad option, I/O failure, …)
        // Treating exit code 1 as "no results" is not an assumption we can
        // remove — it is the documented, stable protocol for all grep-style
        // tools. Only codes ≥ 2 (or a missing code, e.g. killed by signal)
        // indicate a real failure.
        if !output.status.success() && output.status.code() != Some(1) {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git grep failed: {stderr}");
        }

        let stdout = String::from_utf8(output.stdout).context("git grep output is not UTF-8")?;

        Ok(stdout
            .lines()
            .filter(|l| !l.is_empty())
            .map(|l| l.replace('\\', "/"))
            .collect())
    }
}

// ─── Private helpers ───

fn strip_git_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

fn staged_diff(root: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--cached"])
        .current_dir(root)
        .output()
        .context("failed to run git diff --cached")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff --cached failed: {stderr}");
    }

    String::from_utf8(output.stdout).context("git diff --cached output is not UTF-8")
}

fn git_diff(root: &Path, range: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        .args(["diff", range])
        .current_dir(root)
        .output()
        .context("failed to run git diff")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git diff failed: {stderr}");
    }

    String::from_utf8(output.stdout).context("git diff output is not UTF-8")
}

fn parse_no_ifttt_from_commits(root: &Path, log_range: &str) -> Option<String> {
    let output = match std::process::Command::new("git")
        .args(["log", "--format=%B", log_range])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("warning: failed to run git log for NO_IFTTT detection: {e}");
            return None;
        }
    };

    if !output.status.success() {
        return None;
    }

    let log = String::from_utf8_lossy(&output.stdout);
    log.lines()
        .find(|l| l.starts_with("NO_IFTTT="))
        .map(|l| l.trim_start_matches("NO_IFTTT=").to_string())
}

fn detect_range(root: &Path) -> Result<String> {
    if let Some(upstream) = rev_parse(root, "@{u}") {
        let head = rev_parse(root, "HEAD").unwrap_or_else(|| "HEAD".to_string());
        return Ok(format!("{upstream}...{head}"));
    }

    for base in ["origin/main", "origin/master"] {
        if ref_exists(root, base) {
            return Ok(format!("{base}...HEAD"));
        }
    }

    anyhow::bail!("no upstream branch found. Provide a ref range (e.g. main...HEAD) via --diff")
}

/// No `current_dir` override — we don't have a root yet (that's what we're
/// finding), and git searches upward from cwd automatically.
fn detect_root() -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if output.status.success() {
        let s = String::from_utf8(output.stdout).ok()?;
        Some(PathBuf::from(s.trim()))
    } else {
        None
    }
}

fn rev_parse(root: &Path, arg: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", arg])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8(output.stdout).ok()?;
        Some(s.trim().to_string())
    } else {
        None
    }
}

fn ref_exists(root: &Path, refname: &str) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", refname])
        .current_dir(root)
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Uses `rsplit_once` so only the rightmost `...` (range separator) is converted,
/// leaving any `...` inside a ref name intact.
fn three_dot_to_log_range(range: &str) -> String {
    match range.rsplit_once("...") {
        Some((base, head)) => format!("{base}..{head}"),
        None => range.to_string(),
    }
}

#[cfg(test)]
#[path = "vcs_git_test.rs"]
mod tests;
