use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};

use rayon::prelude::*;

// ─── Public types ───

/// Changed lines for a file, keyed by line number (1-based).
#[derive(Debug, Clone, Default)]
pub struct FileChanges {
    /// Lines added or modified in the new version (new-file line numbers).
    pub added_lines: HashSet<usize>,
    /// Lines removed from the old version (old-file line numbers).
    pub removed_lines: HashSet<usize>,
    /// New-file line positions where removals occurred. When a line is removed,
    /// the current new-file line counter marks where in the new file the removal
    /// effectively happened. Use this (not `removed_lines`) when comparing
    /// against new-file ranges (e.g. IfChange/ThenChange line numbers).
    pub removed_new_positions: HashSet<usize>,
    /// True when the file was deleted in this diff (new path was /dev/null).
    /// The file no longer exists on disk; no line data is populated.
    pub deleted: bool,
}

/// Map of relative file paths to their changes.
pub type ChangeMap = HashMap<String, FileChanges>;

// ─── Public API ───

/// Parse a unified diff and extract changed files with their line numbers.
pub fn from_diff(input: &mut dyn Read) -> Result<ChangeMap, String> {
    let mut content = String::new();
    input
        .read_to_string(&mut content)
        .map_err(|e| format!("failed to read diff: {e}"))?;

    if content.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let patches =
        patch::Patch::from_multiple(&content).map_err(|e| format!("failed to parse diff: {e}"))?;
    let mut result = HashMap::new();

    for p in patches {
        let new_path = strip_diff_prefix(&p.new.path);
        let old_path = strip_diff_prefix(&p.old.path);

        // Track deleted files so callers can do reverse-reference lookups.
        if new_path == "/dev/null" {
            if old_path != "/dev/null" {
                result.insert(
                    old_path.to_string(),
                    FileChanges {
                        deleted: true,
                        ..Default::default()
                    },
                );
            }
            continue;
        }

        let mut changes = FileChanges::default();

        for hunk in &p.hunks {
            let mut old_line = hunk.old_range.start as usize;
            let mut new_line = hunk.new_range.start as usize;

            for line in &hunk.lines {
                match line {
                    patch::Line::Add(_) => {
                        changes.added_lines.insert(new_line);
                        new_line += 1;
                    }
                    patch::Line::Remove(_) => {
                        changes.removed_lines.insert(old_line);
                        // Record the *new-file* position where this removal happened.
                        // Because a removal doesn't advance `new_line`, multiple
                        // consecutive removals all map to the same `new_line` value —
                        // they collapse to a single insertion point in the new file.
                        // This is intentional: range-overlap checks in check.rs compare
                        // against new-file line numbers (from directive parsing), so
                        // `removed_new_positions` must use the same coordinate space.
                        changes.removed_new_positions.insert(new_line);
                        old_line += 1;
                    }
                    patch::Line::Context(_) => {
                        old_line += 1;
                        new_line += 1;
                    }
                }
            }
        }

        // Track renames under old path too, including added lines so that
        // ThenChange targets still referencing the old path detect modifications.
        if old_path != new_path && old_path != "/dev/null" {
            result
                .entry(old_path.to_string())
                .or_insert_with(|| changes.clone());
        }

        let key = new_path.to_string();
        if let Some(existing) = result.get_mut(&key) {
            existing.added_lines.extend(&changes.added_lines);
            existing.removed_lines.extend(&changes.removed_lines);
            existing
                .removed_new_positions
                .extend(&changes.removed_new_positions);
        } else {
            result.insert(key, changes);
        }
    }

    Ok(result)
}

/// Walk a directory and produce a synthetic ChangeMap where every line in every
/// file is treated as "added" (scan mode = everything changed).
///
/// Also returns a content cache (`HashMap<String, String>`) keyed by relative
/// path. Callers can pass this cache to `check::check` so that the files are
/// not read a second time during directive parsing.
pub fn from_directory(dir: &Path) -> (ChangeMap, HashMap<String, String>) {
    let files = collect_files(dir);

    let entries: Vec<(String, String, usize)> = files
        .par_iter()
        .filter_map(|file_path| {
            let rel_path = file_path.strip_prefix(dir).unwrap_or(file_path);
            let rel_str = rel_path.to_string_lossy().to_string();

            let content = std::fs::read_to_string(file_path).ok()?;

            if is_binary(&content) {
                return None;
            }

            let line_count = content.lines().count();
            if line_count == 0 {
                return None;
            }

            Some((rel_str, content, line_count))
        })
        .collect();

    let mut result = HashMap::with_capacity(entries.len());
    let mut content_cache = HashMap::with_capacity(entries.len());

    for (rel_str, content, line_count) in entries {
        content_cache.insert(rel_str.clone(), content);
        result.insert(
            rel_str,
            FileChanges {
                added_lines: (1..=line_count).collect(),
                ..Default::default()
            },
        );
    }

    (result, content_cache)
}

// ─── Private helpers ───

/// Returns true if `content` appears to be a binary file (null byte in first 8 KB).
pub(crate) fn is_binary(content: &str) -> bool {
    content.as_bytes().iter().take(8192).any(|&b| b == 0)
}

/// Strip the standard `a/` or `b/` diff prefixes from a path.
fn strip_diff_prefix(path: &str) -> &str {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
}

/// Collect all files in a directory, respecting .gitignore.
fn collect_files(dir: &Path) -> Vec<PathBuf> {
    ignore::WalkBuilder::new(dir)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.into_path())
        .collect()
}

#[cfg(test)]
#[path = "changes_test.rs"]
mod tests;
