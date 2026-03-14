use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

/// Line numbers are 1-based.
#[derive(Debug, Clone, Default)]
pub struct FileChanges {
    /// New-file line numbers.
    pub added_lines: HashSet<usize>,
    /// Old-file line numbers.
    pub removed_lines: HashSet<usize>,
    /// Use this (not `removed_lines`) when comparing against new-file ranges
    /// (e.g. IfChange/ThenChange line numbers). Multiple consecutive removals
    /// collapse to a single new-file position.
    pub(crate) removed_new_positions: HashSet<usize>,
    /// New path was /dev/null — no line data is populated.
    pub deleted: bool,
}

pub type ChangeMap = HashMap<String, FileChanges>;

impl FileChanges {
    /// Create a marker for a deleted file (no line data).
    pub fn deleted() -> Self {
        Self {
            deleted: true,
            ..Self::default()
        }
    }
}

/// Result of reading a file from the VCS provider.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FileContent {
    /// Valid text content.
    Text(String),
    /// File exists but is not valid text (contains NUL bytes or invalid UTF-8).
    Binary,
}

impl FileContent {
    /// Returns the text content by reference, or `None` for binary files.
    #[cfg(test)]
    pub(crate) fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Binary => None,
        }
    }
}

/// Individual file-matching predicate.
#[derive(Clone)]
pub enum FilePattern<'a> {
    /// File content contains this literal string.
    Contains(&'a str),
}

impl FilePattern<'_> {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Contains(s) => s,
        }
    }

    pub fn matches(&self, content: &str) -> bool {
        match self {
            Self::Contains(s) => content.contains(s),
        }
    }
}

/// OR-combined file filter: a file matches if it satisfies at least one
/// pattern. Supports byte-budget partitioning for backends with CLI argument
/// length limits.
#[derive(Clone)]
pub struct FileFilter<'a>(Vec<FilePattern<'a>>);

impl<'a> FileFilter<'a> {
    /// Create a filter matching files that satisfy at least one pattern.
    pub fn any(patterns: Vec<FilePattern<'a>>) -> Self {
        Self(patterns)
    }

    /// Match all files — no additional constraints beyond the needle.
    pub fn all() -> Self {
        Self(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn patterns(&self) -> &[FilePattern<'a>] {
        &self.0
    }

    /// Test whether content matches this filter.
    /// An empty filter matches everything.
    pub fn matches(&self, content: &str) -> bool {
        self.0.is_empty() || self.0.iter().any(|p| p.matches(content))
    }

    /// Split into chunks whose results should be unioned.
    ///
    /// Each chunk's total size (including `per_pattern_overhead` per pattern)
    /// fits within `max_bytes`. A single pattern exceeding the budget is
    /// placed alone in its own chunk — it cannot be split further, and the
    /// caller must tolerate the potential oversize.
    pub fn partition(&self, max_bytes: usize, per_pattern_overhead: usize) -> Vec<FileFilter<'a>> {
        let total: usize = self
            .0
            .iter()
            .map(|p| per_pattern_overhead + p.as_str().len())
            .sum();
        if total <= max_bytes {
            return vec![self.clone()];
        }
        let mut chunks = Vec::new();
        let mut current = Vec::new();
        let mut size = 0usize;
        for pat in &self.0 {
            let cost = per_pattern_overhead + pat.as_str().len();
            if !current.is_empty() && size + cost > max_bytes {
                chunks.push(FileFilter(std::mem::take(&mut current)));
                size = 0;
            }
            current.push(pat.clone());
            size += cost;
        }
        if !current.is_empty() {
            chunks.push(FileFilter(current));
        }
        chunks
    }
}

/// Implement this trait to support a new backend (git, Mercurial, Piper, …).
pub trait VcsProvider: Send + Sync {
    fn diff(&self) -> Result<ChangeMap>;

    /// None = proceed normally. Independent of diff().
    fn suppressions(&self) -> Result<Option<String>>;

    /// None = file does not exist (not an error).
    ///
    /// The git backend reads from the filesystem, so gitignored files are
    /// readable. This is intentional — ThenChange targets must be readable
    /// regardless of tracked status.
    fn read_file(&self, rel_path: &str) -> Result<Option<FileContent>>;

    /// The git backend uses raw `Path::exists`, so gitignored files return
    /// true. `search_string_in_files` uses `git grep` and skips them. In
    /// practice this is benign: LINT directives in gitignored files are a
    /// misconfiguration.
    fn file_exists(&self, rel_path: &str) -> Result<bool>;

    /// Return root-relative paths of files containing `needle` that also
    /// match `filter`. When the filter is empty ([`FileFilter::all`]),
    /// returns all files containing `needle`.
    fn search_string_in_files(&self, needle: &str, filter: &FileFilter<'_>) -> Result<Vec<String>>;

    /// Resolve a raw target path to a root-relative path, with a detailed
    /// error message on failure.
    ///
    /// The default requires a `//` prefix, rejects absolute paths and `..`
    /// traversal, and normalises backslashes. Override to support additional
    /// schemes.
    fn try_resolve_path(&self, raw: &str) -> Result<String, String> {
        strict_resolve_path(raw)
    }

    /// Like `try_resolve_path`, but returns `None` instead of an error
    /// (for call sites that silently skip unrecognised schemes).
    fn resolve_path(&self, raw: &str) -> Option<String> {
        self.try_resolve_path(raw).ok()
    }

    fn validate_files(&self) -> &[String];

    /// Whether strict path mode is enabled.
    fn is_strict(&self) -> bool {
        true
    }
}

pub(crate) fn strict_resolve_path(raw: &str) -> Result<String, String> {
    let rel = raw
        .strip_prefix("//")
        .ok_or_else(|| format!("target path {raw} must start with // (e.g. //{raw})"))?;
    // Normalize backslashes before any checks so mixed-separator tricks
    // like `foo\..\/secret` are caught.
    let normalized = rel.replace('\\', "/");
    if Path::new(&normalized).is_absolute() {
        return Err(format!(
            "target path {raw} must be relative after // (absolute paths are not allowed)"
        ));
    }
    if normalized.split('/').any(|c| c == "..") {
        return Err(format!(
            "path traversal (..) is not allowed in target: {raw}"
        ));
    }
    Ok(normalized)
}

pub(crate) fn permissive_resolve_path(raw: &str) -> Result<String, String> {
    if raw.contains("://") {
        return Err(format!("URL targets are not supported: {raw}"));
    }
    let rel = raw
        .strip_prefix("//")
        .or_else(|| raw.strip_prefix('/'))
        .unwrap_or(raw);
    // Normalize backslashes before any checks so mixed-separator tricks
    // like `foo\..\/secret` are caught.
    let normalized = rel.replace('\\', "/");
    if Path::new(&normalized).is_absolute() {
        return Err(format!(
            "target path {raw} must be relative (absolute paths are not allowed)"
        ));
    }
    if normalized.split('/').any(|c| c == "..") {
        return Err(format!(
            "path traversal (..) is not allowed in target: {raw}"
        ));
    }
    Ok(normalized)
}
