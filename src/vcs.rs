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
    /// Internal: maps removed lines to new-file positions for range-overlap queries.
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
pub enum FileContent {
    /// Valid text content.
    Text(String),
    /// File exists but is not valid text (contains NUL bytes or invalid UTF-8).
    Binary,
}

impl FileContent {
    /// Returns the text content by reference, or `None` for binary files.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            Self::Binary => None,
        }
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
    /// true. `search_files` uses `git grep` and skips them. In practice this
    /// is benign: LINT directives in gitignored files are a misconfiguration.
    fn file_exists(&self, rel_path: &str) -> Result<bool>;

    /// Return root-relative paths of files whose content contains `query`.
    /// The git backend uses `git grep` (tracked files only).
    fn search_files(&self, query: &str) -> Result<Vec<String>>;

    /// Resolve a raw target path to a root-relative path, with a detailed
    /// error message on failure.
    ///
    /// The default requires a `//` prefix, rejects absolute paths and `..`
    /// traversal, and normalises backslashes. Override to support additional
    /// schemes.
    fn try_resolve_path(&self, raw: &str) -> Result<String, String> {
        default_resolve_path(raw)
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

pub(crate) fn default_resolve_path(raw: &str) -> Result<String, String> {
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

pub(crate) fn lenient_resolve_path(raw: &str) -> Result<String, String> {
    if raw.starts_with("http://") || raw.starts_with("https://") {
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
