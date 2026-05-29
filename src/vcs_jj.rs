use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use aho_corasick::{AhoCorasick, MatchKind};
use anyhow::{Context, Result};
use rayon::prelude::*;

use crate::udiff;
use crate::vcs::{ChangeMap, FileContent, FileFilter, VcsProvider};
use crate::vcs_none::{
    absolute_path, is_glob_pattern, is_symlink, normalize_input_path, NoneVcsProvider,
};

/// Probe size for binary detection — matches `vcs_none::read_file`'s heuristic.
const BINARY_PROBE_BYTES: usize = 8192;

/// Read a file's contents, returning `None` for files that look binary
/// (NUL byte or invalid UTF-8 in the first `BINARY_PROBE_BYTES`) or are
/// missing/unreadable. Probes before allocating to bound peak memory under
/// parallel walks: a multi-gigabyte binary blob doesn't get fully read.
///
/// Mid-file read errors are intentionally coerced to `None` — search is
/// best-effort across the working copy and a single transient I/O failure
/// should not abort the whole scan. Use `NoneVcsProvider::read_file` for the
/// strict, error-propagating read used by directive validation.
fn read_text_or_skip(path: &Path) -> Option<Vec<u8>> {
    let mut file = std::fs::File::open(path).ok()?;
    let mut probe = [0u8; BINARY_PROBE_BYTES];
    let n = file.read(&mut probe).ok()?;
    let head = &probe[..n];
    if head.contains(&0) || std::str::from_utf8(head).is_err_and(|e| e.error_len().is_some()) {
        return None;
    }
    let mut buf = Vec::from(head);
    file.read_to_end(&mut buf).ok()?;
    Some(buf)
}

pub struct JjVcsProvider {
    inner: NoneVcsProvider,
    /// jj revset (e.g. `main..@`). None when only structural validation is requested.
    diff_range: Option<String>,
}

impl JjVcsProvider {
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

impl VcsProvider for JjVcsProvider {
    fn diff(&self) -> Result<ChangeMap> {
        let range = self
            .diff_range
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("diff() called without a revset"))?;
        let raw = jj_diff(self.inner.root(), range)?;
        let mut changes: ChangeMap =
            udiff::parse(&mut std::io::Cursor::new(raw), udiff::strip_diff_prefix)
                .map_err(anyhow::Error::msg)?;
        changes.retain(|path, _| !is_symlink(&self.inner.root().join(path)));
        Ok(changes)
    }

    fn suppressions(&self) -> Result<Option<String>> {
        let range = match &self.diff_range {
            Some(r) => r,
            None => return Ok(None),
        };
        Ok(parse_no_ifttt_from_commits(self.inner.root(), range))
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
        let files = jj_file_list(self.inner.root())?;

        // Needle and filter are logically distinct (`needle AND (any of
        // filter)`), so two automata keeps the AND/OR split clean: merging
        // them into one OR'd automaton would collapse the semantics.
        let needle_ac = AhoCorasick::builder()
            .ascii_case_insensitive(false)
            .match_kind(MatchKind::Standard)
            .build([needle])
            .context("aho-corasick needle automaton")?;
        let filter_ac = if filter.is_empty() {
            None
        } else {
            let pats: Vec<&str> = filter.patterns().iter().map(|p| p.as_str()).collect();
            Some(
                AhoCorasick::builder()
                    .ascii_case_insensitive(false)
                    .match_kind(MatchKind::Standard)
                    .build(&pats)
                    .context("aho-corasick filter automaton")?,
            )
        };

        let root = self.inner.root().to_path_buf();

        let mut results: Vec<String> = files
            .par_iter()
            .filter_map(|rel_path| {
                let abs = root.join(rel_path);
                // Skip symlinks: `git grep` doesn't follow them; mirror that.
                if is_symlink(&abs) {
                    return None;
                }
                let content = read_text_or_skip(&abs)?;
                if !needle_ac.is_match(&content) {
                    return None;
                }
                if let Some(ac) = &filter_ac {
                    if !ac.is_match(&content) {
                        return None;
                    }
                }
                Some(rel_path.clone())
            })
            .collect();
        results.sort();
        Ok(results)
    }
}

fn jj_diff(root: &Path, revset: &str) -> Result<String> {
    let output = std::process::Command::new("jj")
        .args(["--no-pager", "diff", "--git", "-r", revset])
        .current_dir(root)
        .output()
        .context("failed to run jj diff")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("jj diff failed: {stderr}");
    }

    String::from_utf8(output.stdout).context("jj diff output is not UTF-8")
}

fn parse_no_ifttt_from_commits(root: &Path, revset: &str) -> Option<String> {
    let output = match std::process::Command::new("jj")
        .args([
            "--no-pager",
            "log",
            "--no-graph",
            "-T",
            r#"description ++ "\n""#,
            "-r",
            revset,
        ])
        .current_dir(root)
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            eprintln!("warning: failed to run jj log for NO_IFTTT detection: {e}");
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

fn jj_file_list(root: &Path) -> Result<Vec<String>> {
    // NUL-separated template (jj has no `-z` shorthand). Mirrors
    // `git ls-files -z`: filenames containing newlines stay intact and
    // can't break the line-by-line parse.
    let output = std::process::Command::new("jj")
        .args([
            "--no-pager",
            "file",
            "list",
            "-r",
            "@",
            "-T",
            r#"path ++ "\0""#,
        ])
        .current_dir(root)
        .output()
        .context("jj file list")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("jj file list failed: {stderr}");
    }

    // Lossy decode mirrors `vcs_git::list_tracked_files` — a single
    // non-UTF-8 path doesn't abort the whole search.
    Ok(output
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).replace('\\', "/"))
        .collect())
}

/// No `current_dir` override — we don't have a root yet (that's what we're
/// finding), and jj searches upward from cwd automatically.
fn detect_root() -> Option<PathBuf> {
    let output = std::process::Command::new("jj")
        .args(["root"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let s = String::from_utf8(output.stdout).ok()?;
    Some(PathBuf::from(s.trim()))
}

/// Expands file arguments that contain glob characters (`*`, `?`, `[`) against
/// `jj file list`. Non-glob arguments pass through unchanged. Mirrors the git
/// backend's behavior using jj's view of tracked files instead.
fn expand_file_globs(root: &Path, files: Vec<PathBuf>) -> Vec<PathBuf> {
    if !files.iter().any(|p| is_glob_pattern(&p.to_string_lossy())) {
        return files;
    }

    let tracked = match jj_file_list(root) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("warning: failed to list jj files for glob expansion: {e}");
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

#[cfg(test)]
#[path = "vcs_jj_test.rs"]
mod tests;
