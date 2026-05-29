use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::udiff;
use crate::vcs::{ChangeMap, FileContent, FileFilter, VcsProvider};
use crate::vcs_none::{
    absolute_path, is_glob_pattern, is_symlink, normalize_input_path, NoneVcsProvider,
};

const NULL_OID: &str = "0000000000000000000000000000000000000000";

pub struct GitVcsProvider {
    inner: NoneVcsProvider,
    /// Git ref range (e.g. `main...HEAD`). None when only structural validation is requested.
    diff_range: Option<String>,
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
            .filter(|p| !is_symlink(&absolute_path(&root, p)))
            .collect();
        let normalized: Vec<String> = files
            .iter()
            .filter_map(|p| normalize_input_path(p, &root))
            .collect();
        Self {
            inner: NoneVcsProvider::new(root, strict, normalized),
            diff_range,
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
        if range_has_null_ref(range) {
            return Ok(ChangeMap::default());
        }
        let raw = git_diff(self.inner.root(), range)?;
        let mut changes: ChangeMap =
            udiff::parse(&mut std::io::Cursor::new(raw), udiff::strip_diff_prefix)
                .map_err(anyhow::Error::msg)?;
        changes.retain(|path, _| !is_symlink(&self.inner.root().join(path)));
        Ok(changes)
    }

    fn suppressions(&self) -> Result<Option<String>> {
        let log_range = match &self.diff_range {
            Some(range) => diff_range_to_log_range(range),
            None => return Ok(None),
        };
        if range_has_null_ref(&log_range) {
            return Ok(None);
        }
        Ok(parse_no_ifttt_from_commits(self.inner.root(), &log_range))
    }

    fn read_file(&self, rel_path: &str) -> Result<Option<FileContent>> {
        self.inner.read_file(rel_path)
    }

    fn file_exists(&self, rel_path: &str) -> Result<bool> {
        self.inner.file_exists(rel_path)
    }

    fn try_resolve_path(&self, raw: &str) -> Result<String, String> {
        self.inner.try_resolve_path(raw)
    }

    fn is_strict(&self) -> bool {
        self.inner.is_strict()
    }

    fn validate_files(&self) -> &[String] {
        self.inner.validate_files()
    }

    fn search_string_in_files(&self, needle: &str, filter: &FileFilter<'_>) -> Result<Vec<String>> {
        if filter.is_empty() {
            return self.run_git_grep(needle, None);
        }

        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        for pattern in filter.patterns() {
            // We need `needle AND (pattern1 OR pattern2 ...)`.
            // `git grep --all-match` can express `needle AND pattern`,
            // so union one grep per pattern rather than collapsing the
            // filter into `needle AND pattern1 AND pattern2 ...`.
            for path in self.run_git_grep(needle, Some(pattern.as_str()))? {
                if !seen.insert(path.clone()) {
                    continue;
                }
                result.push(path);
            }
        }
        Ok(result)
    }
}

impl GitVcsProvider {
    /// Run a single `git grep` call for `needle`, optionally intersected with
    /// one additional literal pattern.
    fn run_git_grep(&self, needle: &str, pattern: Option<&str>) -> Result<Vec<String>> {
        let mut args: Vec<String> = vec!["grep".into(), "-rl".into(), "--fixed-strings".into()];
        args.extend(["-e".into(), needle.to_string()]);

        if let Some(pattern) = pattern {
            args.push("--all-match".into());
            args.extend(["-e".into(), pattern.to_string()]);
        }

        let output = std::process::Command::new("git")
            .args(&args)
            .current_dir(self.inner.root())
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

fn git_diff(root: &Path, range: &str) -> Result<String> {
    let output = std::process::Command::new("git")
        // --no-renames: emit add/delete patches for renames so rename-only
        //   commits stay parseable and old paths naturally participate in
        //   reverse lookup.
        // --ignore-submodules=all: a submodule pointer change is a gitlink
        //   (mode 160000) whose path on disk is the submodule's worktree —
        //   a directory, not a blob. Without this, such an entry reaches
        //   read_file and aborts the run with "<path> is a directory".
        //   The parent repo can't validate LINT directives inside a
        //   submodule anyway, so dropping these changes is the correct scope.
        .args(["diff", "--no-renames", "--ignore-submodules=all", range])
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
fn diff_range_to_log_range(range: &str) -> String {
    match range.rsplit_once("...") {
        Some((base, head)) => format!("{base}..{head}"),
        None => range.to_string(),
    }
}

fn range_has_null_ref(range: &str) -> bool {
    split_range(range).is_some_and(|(base, head)| base == NULL_OID || head == NULL_OID)
}

fn split_range(range: &str) -> Option<(&str, &str)> {
    range.rsplit_once("...").or_else(|| range.rsplit_once(".."))
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
            .literal_separator(true)
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

/// Gitlink mode (`160000`) marks submodule entries in `git ls-files --stage`.
/// `git ls-files` lists these like regular paths, but they reference directories
/// rather than blobs — `read_file` would error out on them. Filter them here.
const GITLINK_MODE: &[u8] = b"160000";

fn list_tracked_files(root: &Path) -> Result<Vec<String>> {
    let output = std::process::Command::new("git")
        .args(["ls-files", "-z", "--stage"])
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
        .filter_map(parse_ls_files_stage_record)
        .collect())
}

/// `--stage` records look like `<mode> <oid> <stage>\t<path>` (NUL-terminated).
/// Returns `None` for empty records, malformed lines, and submodule gitlinks.
fn parse_ls_files_stage_record(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let tab_pos = bytes.iter().position(|&b| b == b'\t')?;
    let (meta, rest) = bytes.split_at(tab_pos);
    let mode = meta.split(|&b| b == b' ').next()?;
    if mode == GITLINK_MODE {
        return None;
    }
    let path = &rest[1..];
    Some(String::from_utf8_lossy(path).replace('\\', "/"))
}

#[cfg(test)]
#[path = "vcs_git_test.rs"]
mod tests;
