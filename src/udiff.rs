use std::collections::HashMap;
use std::io::Read;

use patchkit::unified::{HunkLine, PlainOrBinaryPatch};

use crate::vcs::{ChangeMap, FileChanges};

/// Parse a unified diff into per-file change sets.
///
/// Parsing proper is delegated to the `patchkit` crate, which consumes hunk
/// bodies by counting lines against the `@@ -a,b +c,d @@` header — so removed
/// content that renders as `--- …` (e.g. a deleted `-- SQL comment`) can't be
/// mistaken for the next file's header — and drops sections it can't model
/// (mode-only changes, pure renames, binary files) as junk.
///
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

    let entries =
        patchkit::unified::parse_patches(raw.split_inclusive('\n').map(|l| l.as_bytes().to_vec()));
    let mut result = HashMap::new();

    for entry in entries {
        let patch = match entry.map_err(|e| format!("failed to parse diff: {e}"))? {
            PlainOrBinaryPatch::Plain(patch) => patch,
            // Binary blobs can't carry LINT directives; nothing to track.
            PlainOrBinaryPatch::Binary(_) => continue,
        };
        // A `---`/`+++` pair without hunks describes no line changes.
        if patch.hunks.is_empty() {
            continue;
        }

        // `patchkit` returns header paths verbatim, still carrying git's
        // `core.quotePath` quoting; decode before normalizing.
        let new_path = normalize(&unquote_path(&String::from_utf8_lossy(&patch.mod_name)));
        let old_path = normalize(&unquote_path(&String::from_utf8_lossy(&patch.orig_name)));

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

        for hunk in &patch.hunks {
            let mut old_line = hunk.orig_pos;
            let mut new_line = hunk.mod_pos;

            for line in &hunk.lines {
                match line {
                    HunkLine::InsertLine(_) => {
                        changes.added_lines.insert(new_line);
                        new_line += 1;
                    }
                    HunkLine::RemoveLine(_) => {
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
                    HunkLine::ContextLine(_) => {
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

/// Decode a git C-quoted path (`core.quotePath` escapes special and non-ASCII
/// bytes); bare paths pass through unchanged. Octal escapes are raw bytes of
/// the underlying path, so unescaping works at the byte level before decoding
/// back to UTF-8. The closing quote and anything after it are dropped.
fn unquote_path(path: &str) -> String {
    let Some(quoted) = path.strip_prefix('"') else {
        return path.to_string();
    };
    let mut bytes = Vec::with_capacity(quoted.len());
    let mut rest = quoted.bytes().peekable();
    while let Some(byte) = rest.next() {
        match byte {
            b'"' => break,
            b'\\' => {
                let Some(escaped) = rest.next() else { break };
                match escaped {
                    b'a' => bytes.push(0x07),
                    b'b' => bytes.push(0x08),
                    b't' => bytes.push(b'\t'),
                    b'n' => bytes.push(b'\n'),
                    b'v' => bytes.push(0x0B),
                    b'f' => bytes.push(0x0C),
                    b'r' => bytes.push(b'\r'),
                    b'0'..=b'7' => {
                        // Octal escapes are at most three digits; a digit right
                        // after a complete escape is a literal character.
                        let mut value = u32::from(escaped - b'0');
                        for _ in 0..2 {
                            let Some(digit) = rest.next_if(|b| matches!(b, b'0'..=b'7')) else {
                                break;
                            };
                            value = value * 8 + u32::from(digit - b'0');
                        }
                        // Git emits exactly one byte per octal escape; saturate
                        // rather than panic on out-of-range handwritten input.
                        bytes.push(u8::try_from(value).unwrap_or(u8::MAX));
                    }
                    other => bytes.push(other), // `\"`, `\\`, and unknown escapes
                }
            }
            other => bytes.push(other),
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
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
