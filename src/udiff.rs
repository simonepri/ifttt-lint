use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Read;

use crate::vcs::{ChangeMap, FileChanges};

/// `normalize` is applied to paths before keying into the `ChangeMap`, so
/// VCS-specific prefixes (e.g. git's `a/`/`b/`) are stripped at insertion
/// time and rename collisions merge naturally.
pub fn parse(
    input: &mut dyn Read,
    normalize: impl Fn(&str) -> String,
) -> Result<ChangeMap, String> {
    let mut raw = String::new();
    input
        .read_to_string(&mut raw)
        .map_err(|e| format!("failed to read diff: {e}"))?;

    let content = strip_bodiless_sections(&raw);
    let content = strip_no_newline_markers(&content);

    if content.trim().is_empty() {
        return Ok(HashMap::new());
    }

    let patches =
        patch::Patch::from_multiple(&content).map_err(|e| format!("failed to parse diff: {e}"))?;
    let mut result = HashMap::new();

    for p in patches {
        let new_path = normalize(&p.new.path);
        let old_path = normalize(&p.old.path);

        // Track deleted files so callers can do reverse-reference lookups.
        if new_path == "/dev/null" {
            if old_path == "/dev/null" {
                continue;
            }
            // Use entry() rather than insert() so that if this path already
            // has accumulated changes from an earlier patch (e.g. rename→delete
            // in the same diff), those line sets are preserved. insert() would
            // overwrite them, making the ordering of patches observable.
            result
                .entry(old_path)
                .and_modify(|fc: &mut FileChanges| fc.deleted = true)
                .or_insert_with(FileChanges::deleted);
            continue;
        }

        let mut changes = FileChanges::default();

        for hunk in &p.hunks {
            let mut old_line = usize::try_from(hunk.old_range.start)
                .map_err(|_| "diff hunk line number exceeds platform limit".to_string())?;
            let mut new_line = usize::try_from(hunk.new_range.start)
                .map_err(|_| "diff hunk line number exceeds platform limit".to_string())?;

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
        // Use merge_changes so that if this path already has accumulated
        // changes from an earlier patch (e.g. modify then rename in one diff),
        // those changes are preserved rather than overwritten.
        if old_path != new_path && old_path != "/dev/null" {
            merge_changes(&mut result, old_path, changes.clone());
        }

        merge_changes(&mut result, new_path, changes);
    }

    Ok(result)
}

/// Strip the `a/` / `b/` prefixes that git's `--git` unified-diff puts on
/// every path. `jj diff --git` follows the same convention, so both backends
/// pass this as their `normalize` callback. The strip is a no-op for paths
/// that don't carry the prefix.
pub(crate) fn strip_diff_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

/// Drop diff sections the unified-diff parser can't model.
///
/// `patch::Patch::from_multiple` only parses a file section that carries `@@`
/// hunks. Git and jj emit several kinds of section that have none:
///
/// - binary blobs — a `Binary files a/x and b/x differ` summary, or a
///   `GIT binary patch` block under `--binary`;
/// - metadata-only changes — a `chmod` shows up as `old mode`/`new mode`, a
///   pure rename or copy as `rename from`/`rename to`, and an empty new or
///   deleted file as a lone `new file mode`/`deleted file mode`.
///
/// The parser absorbs a hunkless section as preamble only when another hunked
/// patch follows it; one trailing the final patch (or standing alone) leaves it
/// with input it rejects outright, panicking. None of these sections carry
/// changed `LINT` directive lines, so excising them is lossless for our purposes.
fn strip_bodiless_sections(content: &str) -> Cow<'_, str> {
    // `out` stays `None` — and the borrow is returned untouched — until the
    // first section is dropped, at which point it's seeded with everything kept
    // so far. Kept lines are reproduced byte-for-byte from the original slice.
    let mut out: Option<String> = None;
    let mut flush = |start: usize, end: usize, keep: bool| {
        if let Some(s) = out.as_mut() {
            if keep {
                s.push_str(&content[start..end]);
            }
        } else if !keep {
            out = Some(content[..start].to_string());
        }
    };

    // A section runs from one `diff --git` line to the next. Content before the
    // first such line is preamble and is always kept.
    let mut section_start = 0;
    let mut pos = 0;
    let mut in_file_section = false;
    let mut section_has_hunk = false;
    for line in content.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        if line.starts_with("diff --git ") {
            flush(
                section_start,
                line_start,
                !in_file_section || section_has_hunk,
            );
            section_start = line_start;
            in_file_section = true;
            section_has_hunk = false;
        } else if line.starts_with("@@ ") {
            section_has_hunk = true;
        }
    }
    flush(
        section_start,
        content.len(),
        !in_file_section || section_has_hunk,
    );

    match out {
        Some(stripped) => Cow::Owned(stripped),
        None => Cow::Borrowed(content),
    }
}

/// Drop `\ No newline at end of file` markers. The parser tolerates one only at
/// the very end of a patch, but git emits one mid-hunk whenever a change toggles
/// the trailing newline of the last line, leaving the parser with the unconsumed
/// line that follows. The marker is metadata, not file content, so removing it is
/// lossless for line-number tracking.
fn strip_no_newline_markers(content: &str) -> Cow<'_, str> {
    const MARKER: &str = "\\ No newline at end of file";
    if !content.contains(MARKER) {
        return Cow::Borrowed(content);
    }

    let mut out = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) != MARKER {
            out.push_str(line);
        }
    }
    Cow::Owned(out)
}

fn merge_changes(result: &mut ChangeMap, path: String, changes: FileChanges) {
    let Some(existing) = result.get_mut(&path) else {
        result.insert(path, changes);
        return;
    };

    existing.added_lines.extend(&changes.added_lines);
    existing.removed_lines.extend(&changes.removed_lines);
    existing
        .removed_new_positions
        .extend(&changes.removed_new_positions);
}

#[cfg(test)]
#[path = "udiff_test.rs"]
mod tests;
