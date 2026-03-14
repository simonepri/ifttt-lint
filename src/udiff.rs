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
        let new_path = normalize(&p.new.path);
        let old_path = normalize(&p.old.path);

        // Track deleted files so callers can do reverse-reference lookups.
        if new_path == "/dev/null" {
            if old_path == "/dev/null" {
                continue;
            }
            result.insert(old_path, FileChanges::deleted());
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
        if old_path != new_path && old_path != "/dev/null" {
            result.entry(old_path).or_insert_with(|| changes.clone());
        }

        merge_changes(&mut result, new_path, changes);
    }

    Ok(result)
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
