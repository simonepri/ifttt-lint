use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::Path;

use rayon::prelude::*;

use crate::changes::{ChangeMap, FileChanges};
use crate::grammar::{self, Directive, DirectiveError, Target};

// ─── Public types ───

/// A lint finding — an error detected during validation.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Finding {
    pub source_file: String,
    pub source_line: NonZeroUsize,
    pub source_label: Option<String>,
    pub target_raw: String,
    pub then_change_line: NonZeroUsize,
    pub message: String,
}

/// A parse-level error (malformed directives, duplicates, structural issues).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ParseError {
    pub file: String,
    pub line: NonZeroUsize,
    pub message: String,
}

impl Finding {
    pub fn source_location(&self) -> String {
        match &self.source_label {
            Some(label) => format!(
                "{}:{} (label '{}')",
                self.source_file, self.source_line, label
            ),
            None => format!("{}:{}", self.source_file, self.source_line),
        }
    }
}

impl ParseError {
    fn from_directive_error(err: &DirectiveError, file: &str) -> Self {
        Self {
            file: file.to_string(),
            line: err.line,
            message: err.message.clone(),
        }
    }
}

/// Result of validation.
#[derive(Debug, Default, serde::Serialize)]
pub struct CheckResult {
    pub findings: Vec<Finding>,
    pub parse_errors: Vec<ParseError>,
}

// ─── Public API ───

/// Run all validation: parse files, validate structure, check cross-file consistency.
///
/// Uses two parsing passes: first parses changed files (and any files in
/// `file_list`) to discover their directives, then parses target files referenced
/// by ThenChange directives (which are only known after the first pass completes).
///
/// `file_list` is an optional list of files (root-relative paths) to validate
/// structurally: for each listed file every ThenChange target is checked for
/// existence and label validity, regardless of whether the file appears in the
/// diff.  Files in the list that do not exist on disk are reported as parse
/// errors.  Pairs already triggered by the diff are skipped (the diff pass
/// handles those).  When `file_list` is non-empty the reverse-lookup pass
/// skips those files so findings are never emitted twice.
///
pub fn check(
    changes: &ChangeMap,
    root: &Path,
    ignore_patterns: &[globset::GlobMatcher],
    file_list: &[String],
) -> CheckResult {
    let mut result = CheckResult::default();

    // Pass 1: Parse changed files *and* file_list files to discover their directives.
    let changed_files: Vec<String> = changes.keys().cloned().collect();
    let seed_files: Vec<String> = changes
        .keys()
        .chain(
            file_list
                .iter()
                .filter(|f| !changes.contains_key(f.as_str())),
        )
        .cloned()
        .collect();
    let cache = parse_files(&seed_files, root, HashMap::new());

    // Pass 2: Parse target files referenced by ThenChange directives found in pass 1.
    let target_paths = collect_target_paths(&cache, &seed_files);
    let mut cache = parse_files(&target_paths, root, cache);

    // Report modified (non-deleted) files that are missing from disk.
    // Deleted files are expected to be absent.
    // Modified-but-missing indicates a stale patch or race condition — bad input.
    for file_str in &changed_files {
        if !cache.contains_key(file_str) {
            let fc = changes.get(file_str);
            if fc.is_some_and(|c| !c.deleted) {
                result.parse_errors.push(ParseError {
                    file: file_str.clone(),
                    line: NonZeroUsize::MIN,
                    message: "file referenced in diff but not found on disk".to_string(),
                });
            }
        }
    }

    // Report file_list entries that don't exist on disk.
    for file_str in file_list {
        if !cache.contains_key(file_str) {
            result.parse_errors.push(ParseError {
                file: file_str.clone(),
                line: NonZeroUsize::MIN,
                message: "file not found on disk".to_string(),
            });
        }
    }

    // Collect parse errors and validate structure for all parsed files
    for (path, parsed) in &cache {
        for err in &parsed.errors {
            result
                .parse_errors
                .push(ParseError::from_directive_error(err, path));
        }
        result
            .parse_errors
            .extend(validate_structure(&parsed.directives, path));
    }

    // Pre-compute sorted line indices for efficient range queries.
    // Only built for files that have directives in the cache; files without
    // LINT directives produced empty ParsedFiles and will never be triggered.
    let sorted_lines: HashMap<&str, SortedLines> = changes
        .iter()
        .filter(|(path, _)| cache.contains_key(path.as_str()))
        .map(|(path, fc)| (path.as_str(), SortedLines::from_changes(fc)))
        .collect();

    let file_list_set: HashSet<&str> = file_list.iter().map(|s| s.as_str()).collect();

    // Structural validity + diff-based passes share a ValidationContext that
    // borrows `cache`. Scoped so `cache` becomes mutable again for pass 3.
    {
        let ctx = ValidationContext {
            cache: &cache,
            sorted_lines: &sorted_lines,
            diff: changes,
            root,
            exists_cache: std::cell::RefCell::new(HashMap::new()),
        };

        // Structural validity pass: for each file in `file_list` validate that every
        // ThenChange target exists and every referenced label exists, independently of
        // whether the file was modified in the diff.  Pairs already triggered by the
        // diff are skipped to avoid duplicate findings.
        for file_str in file_list {
            let Some(parsed) = cache.get(file_str) else {
                continue;
            };
            let sorted = sorted_lines.get(file_str.as_str());
            let pairs = build_pairs(&parsed.directives);
            for pair in &pairs {
                if is_triggered(pair, sorted) {
                    continue; // diff pass handles triggered pairs
                }
                for target in &pair.targets {
                    if should_ignore(target, ignore_patterns) {
                        continue;
                    }
                    validate_target_exists(file_str, pair, target, &ctx, &mut result);
                }
            }
        }

        // Diff-based cross-file validation
        for file_str in &changed_files {
            let Some(parsed) = cache.get(file_str) else {
                continue;
            };

            let sorted = sorted_lines.get(file_str.as_str());
            let pairs = build_pairs(&parsed.directives);

            for pair in &pairs {
                if !is_triggered(pair, sorted) {
                    continue;
                }

                for target in &pair.targets {
                    if should_ignore(target, ignore_patterns) {
                        continue;
                    }
                    validate_target(file_str, pair, target, &ctx, &mut result);
                }
            }
        }
    }

    // Pass 3: reverse lookup — find surviving files that reference a deleted target.
    // Only runs when the diff contains file deletions; the two-stage content
    // pre-filter (`LINT.` then deleted path) keeps this cheap even when the
    // deleted file had no LINT directives — no surviving file will mention
    // that path in a ThenChange, so essentially zero work is done.
    // Files in `file_list` are excluded: their structural pass already catches
    // dangling references to deleted targets.
    let deleted_set: HashSet<&str> = changes
        .iter()
        .filter(|(_, fc)| fc.deleted)
        .map(|(path, _)| path.as_str())
        .collect();
    if !deleted_set.is_empty() {
        check_deleted_references(
            &deleted_set,
            &sorted_lines,
            &mut cache,
            root,
            ignore_patterns,
            &file_list_set,
            &mut result,
        );
    }

    // Sort for deterministic output (parallel passes like check_deleted_references
    // produce findings in arbitrary order).
    result.findings.sort_by(|a, b| {
        a.source_file
            .cmp(&b.source_file)
            .then(a.source_line.cmp(&b.source_line))
            .then(a.target_raw.cmp(&b.target_raw))
    });
    result.parse_errors.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.message.cmp(&b.message))
    });

    result
}

// ─── Path utilities ───

/// Normalize a path string to use forward slashes.
///
/// On Windows, paths may contain backslashes which would not match the
/// forward-slash keys stored in `ChangeMap` and `ParseCache`.
pub fn normalize_path_str(s: &str) -> String {
    s.replace('\\', "/")
}

/// Normalize a filesystem path to use forward slashes.
fn normalize_path(path: &Path) -> String {
    normalize_path_str(&path.to_string_lossy())
}

/// Walk all files in a directory in parallel, respecting .gitignore.
///
/// Uses `ignore::WalkBuilder::build_parallel()` so both the directory traversal
/// and the per-entry processing happen across multiple threads.
fn walk_files_parallel(dir: &Path) -> Vec<std::path::PathBuf> {
    let (tx, rx) = std::sync::mpsc::channel();
    ignore::WalkBuilder::new(dir)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .filter_entry(|entry| entry.file_name() != ".git")
        .build_parallel()
        .run(|| {
            let tx = tx.clone();
            Box::new(move |entry| {
                match entry {
                    Ok(e) => {
                        if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                            let _ = tx.send(e.into_path());
                        }
                    }
                    Err(e) => eprintln!("warning: error walking directory: {e}"),
                }
                ignore::WalkState::Continue
            })
        });
    drop(tx);
    rx.into_iter().collect()
}

// ─── File parsing ───

#[derive(Debug, Clone)]
struct ParsedFile {
    directives: Vec<Directive>,
    errors: Vec<DirectiveError>,
}

impl ParsedFile {
    fn empty() -> Self {
        Self {
            directives: vec![],
            errors: vec![],
        }
    }
}

type ParseCache = HashMap<String, ParsedFile>;

/// Parse directives from a list of files in parallel.
/// Skips files already in the cache.
/// Files that do not exist on disk are silently omitted from the cache so
/// that callers can detect their absence via `cache.contains_key`.
fn parse_files(paths: &[String], root: &Path, mut cache: ParseCache) -> ParseCache {
    let new_entries: Vec<(String, ParsedFile)> = paths
        .par_iter()
        .filter(|rel_path| !cache.contains_key(rel_path.as_str()))
        .filter_map(|rel_path| {
            let abs_path = root.join(rel_path);
            let parsed = parse_single_file(&abs_path, Path::new(rel_path))?;
            Some((rel_path.clone(), parsed))
        })
        .collect();

    cache.extend(new_entries);
    cache
}

/// Parse a single file, returning parsed directives or errors.
///
/// Returns `None` when the file does not exist on disk, allowing callers to
/// distinguish "missing" from "unreadable" or "empty".
fn parse_single_file(abs_path: &Path, rel_path: &Path) -> Option<ParsedFile> {
    let content = match std::fs::read_to_string(abs_path) {
        Ok(c) => c,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                return None;
            }
            if e.kind() == std::io::ErrorKind::IsADirectory
                || e.kind() == std::io::ErrorKind::PermissionDenied
            {
                return Some(ParsedFile::empty());
            }
            return Some(ParsedFile {
                directives: vec![],
                errors: vec![DirectiveError {
                    line: NonZeroUsize::MIN,
                    message: format!("failed to read file: {e}"),
                }],
            });
        }
    };

    // Skip binary files
    if crate::changes::is_binary(&content) {
        return Some(ParsedFile::empty());
    }

    // Quick check: skip files with no LINT directives
    if !content.contains("LINT.") {
        return Some(ParsedFile::empty());
    }

    let (directives, mut errors) = grammar::parse_directives(&content, rel_path);
    let rel_str = rel_path.to_string_lossy();
    errors.extend(validate_uniqueness(&directives, &rel_str));

    Some(ParsedFile { directives, errors })
}

/// Validate directive uniqueness within a file.
fn validate_uniqueness(directives: &[Directive], file_path: &str) -> Vec<DirectiveError> {
    let mut errors = Vec::new();
    let mut if_labels: HashMap<String, NonZeroUsize> = HashMap::new();
    let mut label_names: HashMap<String, NonZeroUsize> = HashMap::new();

    for d in directives {
        match d {
            Directive::IfChange {
                line,
                label: Some(label),
            } => {
                if let Some(prev_line) = if_labels.get(label) {
                    errors.push(DirectiveError {
                        line: *line,
                        message: format!(
                            "duplicate LINT.IfChange label '{label}' (first at {file_path}:{prev_line})"
                        ),
                    });
                } else {
                    if_labels.insert(label.clone(), *line);
                }
            }
            Directive::LabelStart { line, name } => {
                if let Some(prev_line) = label_names.get(name) {
                    errors.push(DirectiveError {
                        line: *line,
                        message: format!(
                            "duplicate LINT.Label '{name}' (first at {file_path}:{prev_line})"
                        ),
                    });
                } else {
                    label_names.insert(name.clone(), *line);
                }
            }
            _ => {}
        }
    }

    errors
}

/// Collect all unique target file paths referenced by ThenChange directives.
fn collect_target_paths(cache: &ParseCache, changed_files: &[String]) -> Vec<String> {
    changed_files
        .iter()
        .filter_map(|path| cache.get(path))
        .flat_map(|parsed| parsed.directives.iter())
        .filter_map(|d| {
            if let Directive::ThenChange { targets, .. } = d {
                Some(targets.iter())
            } else {
                None
            }
        })
        .flatten()
        .filter_map(|t| {
            t.file
                .as_deref()
                .map(|f| normalize_path_str(f.strip_prefix("//").unwrap_or(f)))
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect()
}

// ─── Structural validation ───

/// Validate structural correctness of directives within a file.
fn validate_structure(directives: &[Directive], file_path: &str) -> Vec<ParseError> {
    let mut errors = Vec::new();
    let mut label_stack: Vec<(String, NonZeroUsize)> = Vec::new();
    let mut pending_if: Option<NonZeroUsize> = None;

    for d in directives {
        match d {
            Directive::IfChange { line, .. } => {
                if let Some(prev_line) = pending_if {
                    errors.push(ParseError {
                        file: file_path.to_string(),
                        line: prev_line,
                        message: format!(
                            "LINT.IfChange without matching ThenChange (previous IfChange at line {prev_line})"
                        ),
                    });
                }
                pending_if = Some(*line);
            }
            Directive::ThenChange { line, .. } => {
                if pending_if.is_none() {
                    errors.push(ParseError {
                        file: file_path.to_string(),
                        line: *line,
                        message: "LINT.ThenChange without preceding IfChange".to_string(),
                    });
                }
                pending_if = None;
            }
            Directive::LabelStart { line, name } => {
                label_stack.push((name.clone(), *line));
            }
            Directive::LabelEnd { line } => {
                if label_stack.pop().is_none() {
                    errors.push(ParseError {
                        file: file_path.to_string(),
                        line: *line,
                        message: "LINT.EndLabel without matching Label".to_string(),
                    });
                }
            }
        }
    }

    for (name, line) in label_stack {
        errors.push(ParseError {
            file: file_path.to_string(),
            line,
            message: format!("LINT.Label('{name}') without matching EndLabel"),
        });
    }

    if let Some(line) = pending_if {
        errors.push(ParseError {
            file: file_path.to_string(),
            line,
            message: "LINT.IfChange without matching ThenChange".to_string(),
        });
    }

    errors
}

// ─── Sorted line index for range queries ───

/// Pre-sorted line numbers for efficient range-overlap checks via binary search.
struct SortedLines {
    added: Vec<usize>,
    removed_new_pos: Vec<usize>,
}

impl SortedLines {
    fn from_changes(changes: &FileChanges) -> Self {
        let mut added: Vec<usize> = changes.added_lines.iter().copied().collect();
        let mut removed_new_pos: Vec<usize> =
            changes.removed_new_positions.iter().copied().collect();
        added.sort_unstable();
        removed_new_pos.sort_unstable();
        Self {
            added,
            removed_new_pos,
        }
    }

    /// Returns true if any value in `sorted` falls within [lo, hi].
    fn any_in_range(sorted: &[usize], lo: usize, hi: usize) -> bool {
        if lo > hi {
            return false;
        }
        let start = sorted.partition_point(|&v| v < lo);
        start < sorted.len() && sorted[start] <= hi
    }
}

// ─── Cross-file validation ───

#[derive(Debug)]
struct DirectivePair {
    if_line: NonZeroUsize,
    if_label: Option<String>,
    then_line: NonZeroUsize,
    targets: Vec<Target>,
}

fn build_pairs(directives: &[Directive]) -> Vec<DirectivePair> {
    let mut pairs = Vec::new();
    let mut pending_if: Option<(NonZeroUsize, Option<String>)> = None;

    for d in directives {
        match d {
            Directive::IfChange { line, label } => {
                pending_if = Some((*line, label.clone()));
            }
            Directive::ThenChange { line, targets } => {
                if let Some((if_line, if_label)) = pending_if.take() {
                    pairs.push(DirectivePair {
                        if_line,
                        if_label,
                        then_line: *line,
                        targets: targets.clone(),
                    });
                }
            }
            _ => {}
        }
    }

    pairs
}

fn is_triggered(pair: &DirectivePair, sorted: Option<&SortedLines>) -> bool {
    let Some(sorted) = sorted else {
        return false;
    };

    // Added lines use `>=` because an addition on the IfChange line itself means
    // the directive block was touched and should trigger validation.
    if SortedLines::any_in_range(&sorted.added, pair.if_line.get(), pair.then_line.get()) {
        return true;
    }

    // Removed lines use `>` (if_line + 1) because a removal *at* the IfChange
    // line's position means content was removed before the guarded block (the
    // directive itself is on that line), so only removals strictly inside count.
    if SortedLines::any_in_range(
        &sorted.removed_new_pos,
        pair.if_line.get() + 1,
        pair.then_line.get(),
    ) {
        return true;
    }

    false
}

/// Check if a target should be ignored based on glob patterns.
///
/// Matching is attempted against:
/// 1. `target.raw` — the verbatim text inside quotes (e.g. `//foo.rs:label` or `:label`).
///    This includes the `//` prefix so patterns must account for it (e.g. `**/foo.rs`).
/// 2. The resolved file path with `//` stripped (e.g. `foo.rs`), if the target has a file
///    component. This is the most intuitive way to match: `ignore = ["generated/*"]`.
fn should_ignore(target: &Target, ignore_patterns: &[globset::GlobMatcher]) -> bool {
    ignore_patterns.iter().any(|pattern| {
        pattern.is_match(&target.raw)
            || target
                .file
                .as_deref()
                .is_some_and(|f| pattern.is_match(f.strip_prefix("//").unwrap_or(f)))
    })
}

struct ValidationContext<'a> {
    cache: &'a ParseCache,
    sorted_lines: &'a HashMap<&'a str, SortedLines>,
    diff: &'a ChangeMap,
    root: &'a Path,
    /// Cache for `root.join(path).exists()` calls. The same target file can appear
    /// in multiple ThenChange blocks across the changed-file set; caching avoids
    /// repeated filesystem stat calls for the same path.
    ///
    /// Uses `RefCell` because the structural validity loop is intentionally
    /// single-threaded (it mutates `result`). Do not move this into a parallel
    /// context without switching to a thread-safe interior-mutability primitive.
    exists_cache: std::cell::RefCell<HashMap<String, bool>>,
}

struct FindingCtx<'a> {
    source_file: &'a str,
    pair: &'a DirectivePair,
    target: &'a Target,
}

fn mk_finding(ctx: &FindingCtx<'_>, message: String) -> Finding {
    let FindingCtx {
        source_file,
        pair,
        target,
    } = ctx;
    Finding {
        source_file: source_file.to_string(),
        source_line: pair.if_line,
        source_label: pair.if_label.clone(),
        target_raw: target.raw.clone(),
        then_change_line: pair.then_line,
        message,
    }
}

/// Resolve a target to its root-relative path and optional label.
///
/// Returns `None` when no validation is needed (bare `(None, None)` target).
/// Same-file label references `(None, Some(label))` resolve to
/// `(source_file, Some(label))` so that label existence can be validated
/// by the caller.
fn resolve_target(target: &Target, source_file: &str) -> Option<(String, Option<String>)> {
    match (&target.file, &target.label) {
        (Some(file), label) => {
            let rel = file.strip_prefix("//").unwrap_or(file);
            Some((normalize_path_str(rel), label.clone()))
        }
        // Same-file label reference (:label) — resolves to the source file in
        // both structural and diff modes so that label existence is validated.
        (None, Some(label)) => Some((source_file.to_string(), Some(label.clone()))),
        (None, None) => None,
    }
}

/// Check for same-file reference misuse (`//path/to/self` instead of `:label`).
///
/// Returns `true` when a finding was emitted and the caller should stop.
fn check_same_file_reference(
    fctx: &FindingCtx<'_>,
    target_str: &str,
    target_label: &Option<String>,
    result: &mut CheckResult,
) -> bool {
    // Only flag when the target was written with an explicit file component
    // (`//path`). The `:label` shorthand resolves to source_file in the diff
    // pass and is the correct form.
    if fctx.target.file.is_none() || target_str != fctx.source_file {
        return false;
    }
    let message = match target_label {
        None => "self-referencing ThenChange without label is meaningless".to_string(),
        Some(label) => format!(
            "use ':{label}' syntax for same-file label references \
             (replace '{raw}' with ':{label}')",
            raw = fctx.target.raw,
        ),
    };
    result.findings.push(mk_finding(fctx, message));
    true
}

/// Check that the target file exists on disk, using a cache to avoid repeated
/// stat calls. Returns `true` when the file exists.
fn check_file_exists(
    fctx: &FindingCtx<'_>,
    target_str: &str,
    ctx: &ValidationContext<'_>,
    result: &mut CheckResult,
) -> bool {
    let file_exists = ctx.cache.contains_key(target_str)
        || *ctx
            .exists_cache
            .borrow_mut()
            .entry(target_str.to_string())
            .or_insert_with(|| ctx.root.join(target_str).exists());
    if !file_exists {
        let message = match &fctx.target.label {
            Some(label) => format!("target file not found: {target_str} (label '{label}')"),
            None => format!("target file not found: {target_str}"),
        };
        result.findings.push(mk_finding(fctx, message));
    }
    file_exists
}

/// Validate that a ThenChange target exists structurally (file present on disk,
/// label defined) without checking whether the target was modified.
///
/// Used by the structural validity pass for `file_list` files.
fn validate_target_exists(
    source_file: &str,
    pair: &DirectivePair,
    target: &Target,
    ctx: &ValidationContext<'_>,
    result: &mut CheckResult,
) {
    let Some((target_str, target_label)) = resolve_target(target, source_file) else {
        return;
    };
    let fctx = FindingCtx {
        source_file,
        pair,
        target,
    };
    if check_same_file_reference(&fctx, &target_str, &target_label, result) {
        return;
    }
    // Same-file `:label` references don't need a file-existence check — the
    // source file is guaranteed to be in the cache since we just parsed it.
    if target_str == source_file && target.file.is_none() {
        if let Some(label_name) = &target_label {
            if find_label_range(&target_str, label_name, ctx.cache).is_none() {
                result.findings.push(mk_finding(
                    &fctx,
                    format!("label '{label_name}' not found in {target_str}"),
                ));
            }
        }
        return;
    }
    if !check_file_exists(&fctx, &target_str, ctx, result) {
        return;
    }
    if let Some(label_name) = &target_label {
        if find_label_range(&target_str, label_name, ctx.cache).is_none() {
            result.findings.push(mk_finding(
                &fctx,
                format!("label '{label_name}' not found in {target_str}"),
            ));
        }
    }
}

fn validate_target(
    source_file: &str,
    pair: &DirectivePair,
    target: &Target,
    ctx: &ValidationContext<'_>,
    result: &mut CheckResult,
) {
    let Some((target_str, target_label)) = resolve_target(target, source_file) else {
        return;
    };
    let fctx = FindingCtx {
        source_file,
        pair,
        target,
    };
    if check_same_file_reference(&fctx, &target_str, &target_label, result) {
        return;
    }
    if !check_file_exists(&fctx, &target_str, ctx, result) {
        return;
    }
    let target_sorted = ctx.sorted_lines.get(target_str.as_str());
    check_target_modified(
        &fctx,
        &target_str,
        target_label.as_deref(),
        target_sorted,
        ctx,
        result,
    );
}

fn check_target_modified(
    fctx: &FindingCtx<'_>,
    target_str: &str,
    target_label: Option<&str>,
    target_sorted: Option<&SortedLines>,
    ctx: &ValidationContext<'_>,
    result: &mut CheckResult,
) {
    if let Some(label_name) = target_label {
        let label_range = find_label_range(target_str, label_name, ctx.cache);
        match label_range {
            Some(range) => {
                // Added lines must fall within [start, end] (the label's content lines).
                // Removed positions use end+1 because a removal right after the last
                // content line (i.e. on the EndLabel line) still indicates the label
                // region was modified.
                let has_change_in_range = target_sorted
                    .map(|s| {
                        SortedLines::any_in_range(&s.added, range.start, range.end)
                            || SortedLines::any_in_range(
                                &s.removed_new_pos,
                                range.start,
                                range.end + 1,
                            )
                    })
                    .unwrap_or(false);

                if !has_change_in_range {
                    result.findings.push(mk_finding(
                        fctx,
                        format!("target {target_str}:{label_name} was not modified"),
                    ));
                }
            }
            None => {
                result.findings.push(mk_finding(
                    fctx,
                    format!("label '{label_name}' not found in {target_str}"),
                ));
            }
        }
    } else {
        let has_changes = ctx
            .diff
            .get(target_str)
            .map(|c| !c.added_lines.is_empty() || !c.removed_lines.is_empty())
            .unwrap_or(false);

        if !has_changes {
            result.findings.push(mk_finding(
                fctx,
                format!("target {target_str} was not modified"),
            ));
        }
    }
}

#[derive(Debug, Clone)]
struct LabelRange {
    start: usize,
    end: usize,
}

/// Find the line range of a named label within a parsed file.
///
/// Two label forms are supported:
/// - `LINT.Label("name")` / `LINT.EndLabel`: the range covers the lines between them.
/// - `LINT.IfChange("name")` / `LINT.ThenChange`: the range covers the lines between them.
///
/// Returns `None` if the label is not found in the file's parsed directives.
fn find_label_range(file_path: &str, label_name: &str, cache: &ParseCache) -> Option<LabelRange> {
    let parsed = cache.get(file_path)?;

    let mut label_stack: Vec<(&str, usize)> = Vec::new();
    let mut if_content_start: Option<usize> = None;

    for d in &parsed.directives {
        match d {
            Directive::LabelStart { line, name } => {
                label_stack.push((name, line.get() + 1));
            }
            Directive::LabelEnd { line } => {
                if let Some((name, start)) = label_stack.pop() {
                    if name == label_name {
                        return Some(LabelRange {
                            start,
                            end: line.get() - 1,
                        });
                    }
                }
            }
            Directive::IfChange { line, label } => {
                // Only track IfChange blocks whose label matches.
                // Stores the first content line (line after IfChange).
                // Non-matching IfChange directives must not clear a previously
                // matched if_content_start (e.g. in malformed files with
                // consecutive IfChange directives).
                if label.as_ref().map(|l| l.as_str()) == Some(label_name) {
                    if_content_start = Some(line.get() + 1);
                }
            }
            Directive::ThenChange { line, .. } => {
                if let Some(start) = if_content_start.take() {
                    return Some(LabelRange {
                        start,
                        end: line.get() - 1,
                    });
                }
            }
        }
    }

    None
}

/// Scan all surviving files in `root` for `ThenChange` directives that reference
/// a deleted file, emitting a "target file not found" finding for each.
///
/// This runs a single walk of the repo (via `collect_files`), not one per
/// deleted path. Each file is read at most once, and two cheap substring checks
/// (`LINT.` then any deleted path) skip the vast majority of files before the
/// full PEG parse. Files already in `cache` (parsed in earlier passes) reuse
/// their cached directives and skip the disk read entirely.
///
/// The walk uses `ignore::WalkBuilder` which respects `.gitignore` but does NOT
/// leverage git's pre-built index for content grep — it reads file content
/// directly. For typical repos the two-stage substring filter keeps this fast;
/// in very large monorepos the cost is proportional to the number of files
/// containing `LINT.` directives.
///
/// This pass triggers whenever the diff deletes (or renames) any file. If the
/// deleted file had no LINT directives, the substring filter ensures essentially
/// zero extra work: no surviving file will mention that path in a `ThenChange`.
///
/// Pairs that were already triggered by the diff are skipped; the main
/// validation pass already handles those.
fn check_deleted_references(
    deleted: &HashSet<&str>,
    sorted_lines: &HashMap<&str, SortedLines>,
    cache: &mut ParseCache,
    root: &Path,
    ignore_patterns: &[globset::GlobMatcher],
    file_list_set: &HashSet<&str>,
    result: &mut CheckResult,
) {
    let all_files = walk_files_parallel(root);

    type ScanEntry = (Vec<Finding>, Option<(String, ParsedFile)>);
    let results: Vec<ScanEntry> = all_files
        .par_iter()
        .filter_map(|file_path| {
            let rel = file_path.strip_prefix(root).ok()?;
            let rel_str = normalize_path(rel);

            // Don't scan deleted files themselves.
            if deleted.contains(rel_str.as_str()) {
                return None;
            }

            // Skip file_list files — their structural pass already handles them.
            if file_list_set.contains(rel_str.as_str()) {
                return None;
            }

            // Use cached directives when available; otherwise read from disk.
            // Build pairs before moving directives into ParsedFile to avoid cloning.
            let pairs;
            let new_entry: Option<(String, ParsedFile)>;
            if let Some(parsed) = cache.get(&rel_str) {
                if parsed.directives.is_empty() {
                    return None;
                }
                pairs = build_pairs(&parsed.directives);
                new_entry = None;
            } else {
                let content = std::fs::read_to_string(file_path).ok()?;

                if crate::changes::is_binary(&content) {
                    return None;
                }

                // Check LINT. first: the cheapest single-string filter, eliminates
                // the vast majority of files before the costlier per-deleted-path search.
                if !content.contains("LINT.") {
                    return None;
                }

                // Smart skip: none of the deleted paths appear in this file's content.
                if !deleted.iter().any(|d| content.contains(*d)) {
                    return None;
                }

                let (directives, errors) = grammar::parse_directives(&content, rel);
                pairs = build_pairs(&directives);
                new_entry = Some((rel_str.clone(), ParsedFile { directives, errors }));
            };
            let sorted = sorted_lines.get(rel_str.as_str());

            let mut findings = Vec::new();
            for pair in &pairs {
                // Skip pairs already triggered by the diff; the main pass handles them.
                if is_triggered(pair, sorted) {
                    continue;
                }
                for target in &pair.targets {
                    if should_ignore(target, ignore_patterns) {
                        continue;
                    }
                    let Some(file) = &target.file else { continue };
                    let target_str = normalize_path_str(file.strip_prefix("//").unwrap_or(file));
                    if !deleted.contains(target_str.as_str()) {
                        continue;
                    }
                    let message = match &target.label {
                        Some(label) => {
                            format!("target file not found: {target_str} (label '{label}')")
                        }
                        None => format!("target file not found: {target_str}"),
                    };
                    findings.push(mk_finding(
                        &FindingCtx {
                            source_file: &rel_str,
                            pair,
                            target,
                        },
                        message,
                    ));
                }
            }
            Some((findings, new_entry))
        })
        .collect();

    for (findings, new_entry) in results {
        result.findings.extend(findings);
        if let Some((path, parsed)) = new_entry {
            cache.insert(path, parsed);
        }
    }
}

#[cfg(test)]
#[path = "check_test.rs"]
mod tests;
