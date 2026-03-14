use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::vcs::{ChangeMap, FileContent, FileFilter, VcsProvider};

#[path = "udiff.rs"]
mod udiff;

pub struct GitVcsProvider {
    root: PathBuf,
    /// Git ref range (e.g. `main...HEAD`). None when only structural validation is requested.
    diff_range: Option<String>,
    /// When false, accept bare and single-`/` paths in ThenChange targets.
    strict: bool,
    files: Vec<String>,
}

impl GitVcsProvider {
    pub fn new(
        root: PathBuf,
        diff_range: Option<String>,
        strict: bool,
        files: Vec<PathBuf>,
    ) -> Self {
        let files = expand_file_globs(&root, files);
        let files: Vec<PathBuf> = files
            .into_iter()
            .filter(|p| {
                let abs = absolute_path(&root, p);
                !abs.symlink_metadata().is_ok_and(|m| m.is_symlink())
            })
            .collect();
        let normalized = files
            .iter()
            .filter_map(|p| normalize_input_path(p, &root))
            .collect();
        Self {
            root,
            diff_range,
            strict,
            files: normalized,
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
        let range = self
            .diff_range
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("diff() called without a ref range"))?;
        let raw = git_diff(&self.root, range)?;
        let mut changes: ChangeMap = udiff::parse(&mut std::io::Cursor::new(raw), strip_git_prefix)
            .map_err(anyhow::Error::msg)?;
        changes.retain(|path, _| {
            !self
                .root
                .join(path)
                .symlink_metadata()
                .is_ok_and(|m| m.is_symlink())
        });
        Ok(changes)
    }

    fn suppressions(&self) -> Result<Option<String>> {
        let log_range = match &self.diff_range {
            Some(range) => three_dot_to_log_range(range),
            None => return Ok(None),
        };
        Ok(parse_no_ifttt_from_commits(&self.root, &log_range))
    }

    fn read_file(&self, rel_path: &str) -> Result<Option<FileContent>> {
        use std::io::Read;
        let abs = self.root.join(rel_path);
        if abs.metadata().is_ok_and(|m| m.is_dir()) {
            anyhow::bail!("{rel_path} is a directory");
        }
        let mut file = match std::fs::File::open(&abs) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => {
                anyhow::bail!("{rel_path} is a directory")
            }
            Err(e) => return Err(anyhow::anyhow!(e).context(format!("read {rel_path}"))),
        };
        let mut probe = [0u8; 8192];
        let n = file
            .read(&mut probe)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        let head = &probe[..n];
        if head.contains(&0) || std::str::from_utf8(head).is_err_and(|e| e.error_len().is_some()) {
            return Ok(Some(FileContent::Binary));
        }
        let mut buf = Vec::from(head);
        file.read_to_end(&mut buf)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        let text = String::from_utf8(buf)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        Ok(Some(FileContent::Text(text)))
    }

    fn file_exists(&self, rel_path: &str) -> Result<bool> {
        let abs = self.root.join(rel_path);
        Ok(abs.metadata().is_ok_and(|m| m.is_file()))
    }

    fn try_resolve_path(&self, raw: &str) -> Result<String, String> {
        if self.strict {
            crate::vcs::strict_resolve_path(raw)
        } else {
            crate::vcs::permissive_resolve_path(raw)
        }
    }

    fn is_strict(&self) -> bool {
        self.strict
    }

    fn validate_files(&self) -> &[String] {
        &self.files
    }

    fn search_string_in_files(&self, needle: &str, filter: &FileFilter<'_>) -> Result<Vec<String>> {
        if filter.is_empty() {
            return self.run_git_grep(needle, &FileFilter::all());
        }

        // Conservative byte budget for git-grep argument lists.
        // macOS ARG_MAX is ~1 MB, Linux ~2 MB. 256 KB leaves ample room
        // for environment variables, the git binary path, and fixed args.
        const ARGS_BUDGET: usize = 256 * 1024;
        // `-e\0` (3 bytes) + pattern `\0` (1 byte).
        const PER_PATTERN_OVERHEAD: usize = 4;
        // Fixed args: git grep -rl --fixed-strings --all-match -e NEEDLE ( )
        let fixed_cost = 64 + needle.len();
        let remaining = ARGS_BUDGET.saturating_sub(fixed_cost);

        let chunks = filter.partition(remaining, PER_PATTERN_OVERHEAD);
        if chunks.len() == 1 {
            return self.run_git_grep(needle, &chunks[0]);
        }
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for chunk in &chunks {
            for path in self.run_git_grep(needle, chunk)? {
                if seen.insert(path.clone()) {
                    result.push(path);
                }
            }
        }
        Ok(result)
    }
}

impl GitVcsProvider {
    /// Run a single `git grep` call that fits within ARG_MAX.
    fn run_git_grep(&self, needle: &str, filter: &FileFilter<'_>) -> Result<Vec<String>> {
        let mut args: Vec<String> = vec!["grep".into(), "-rl".into(), "--fixed-strings".into()];
        args.extend(["-e".into(), needle.to_string()]);

        let patterns = filter.patterns();
        if !patterns.is_empty() {
            args.push("--all-match".into());
            args.push("(".into());
            for pat in patterns {
                args.extend(["-e".into(), pat.as_str().to_string()]);
            }
            args.push(")".into());
        }

        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(&self.root)
            .output()
            .context("git grep")?;

        // git-grep(1) exit codes follow the POSIX grep(1) convention:
        //   0  — one or more lines matched
        //   1  — no lines matched (not an error)
        //   2+ — an actual error (bad option, I/O failure, …)
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

fn strip_git_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

fn absolute_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }

    root.join(path)
}

fn normalize_input_path(path: &Path, root: &Path) -> Option<String> {
    if !path.is_absolute() {
        return Some(path.to_string_lossy().replace('\\', "/"));
    }

    let Ok(rel) = path.strip_prefix(root) else {
        eprintln!(
            "warning: '{}' is outside the project root and will be skipped",
            path.display()
        );
        return None;
    };
    Some(rel.to_string_lossy().replace('\\', "/"))
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

/// No `current_dir` override — we don't have a root yet (that's what we're
/// finding), and git searches upward from cwd automatically.
fn detect_root() -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let s = String::from_utf8(output.stdout).ok()?;
    Some(PathBuf::from(s.trim()))
}

/// Uses `rsplit_once` so only the rightmost `...` (range separator) is converted,
/// leaving any `...` inside a ref name intact.
fn three_dot_to_log_range(range: &str) -> String {
    match range.rsplit_once("...") {
        Some((base, head)) => format!("{base}..{head}"),
        None => range.to_string(),
    }
}

fn is_glob_pattern(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

/// Expands file arguments that contain glob characters (`*`, `?`, `[`) against
/// `git ls-files`. Non-glob arguments pass through unchanged. This avoids
/// shell command-line length limits when validating the entire repo: the caller
/// can pass `'*'` (quoted to prevent shell expansion) and let the tool
/// enumerate tracked files internally.
fn expand_file_globs(root: &Path, files: Vec<PathBuf>) -> Vec<PathBuf> {
    if !files.iter().any(|p| is_glob_pattern(&p.to_string_lossy())) {
        return files;
    }

    let tracked = match list_tracked_files(root) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warning: failed to list tracked files for glob expansion: {e}");
            return files;
        }
    };

    let mut result = Vec::new();
    for file in &files {
        let s = file.to_string_lossy();
        if !is_glob_pattern(&s) {
            result.push(file.clone());
            continue;
        }
        let glob = match globset::GlobBuilder::new(&s)
            .literal_separator(false)
            .build()
        {
            Ok(glob) => glob,
            Err(e) => {
                eprintln!("warning: invalid glob pattern '{s}': {e}");
                result.push(file.clone());
                continue;
            }
        };
        let matcher = glob.compile_matcher();
        for tracked_file in &tracked {
            if matcher.is_match(tracked_file) {
                result.push(PathBuf::from(tracked_file));
            }
        }
    }
    result
}

fn list_tracked_files(root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .context("git ls-files")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git ls-files failed: {stderr}");
    }

    Ok(output
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).replace('\\', "/"))
        .collect())
}

#[cfg(test)]
#[path = "vcs_git_test.rs"]
mod tests;
