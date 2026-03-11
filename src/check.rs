use std::collections::HashMap;
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
/// Uses two parsing passes: first parses changed files to discover their directives,
/// then parses target files referenced by ThenChange directives (which are only known
/// after the first pass completes).
///
/// `content_hint` is an optional map of relative path → pre-loaded file content.
/// When provided (e.g. from `changes::from_directory`), files present in the map
/// are not read from disk a second time, avoiding a redundant I/O round-trip in
/// scan mode.
pub fn check(
    changes: &ChangeMap,
    root: &Path,
    ignore_patterns: &[globset::GlobMatcher],
    content_hint: Option<&HashMap<String, String>>,
) -> CheckResult {
    let mut result = CheckResult::default();

    // Pass 1: Parse changed files to discover their directives.
    let changed_files: Vec<String> = changes.keys().cloned().collect();
    let cache = parse_files(&changed_files, root, HashMap::new(), content_hint);

    // Pass 2: Parse target files referenced by ThenChange directives found in pass 1.
    // Target files are typically NOT in the content_hint (they weren't part of the
    // original scan set), so they fall back to the normal disk read path.
    let target_paths = collect_target_paths(&cache, &changed_files);
    let cache = parse_files(&target_paths, root, cache, content_hint);

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

    // Validate cross-file consistency
    let ctx = ValidationContext {
        cache: &cache,
        sorted_lines: &sorted_lines,
        diff: changes,
        root,
        exists_cache: std::cell::RefCell::new(HashMap::new()),
    };

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

    result
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
fn parse_files(
    paths: &[String],
    root: &Path,
    mut cache: ParseCache,
    content_hint: Option<&HashMap<String, String>>,
) -> ParseCache {
    let new_entries: Vec<(String, ParsedFile)> = paths
        .par_iter()
        .filter(|rel_path| !cache.contains_key(rel_path.as_str()))
        .map(|rel_path| {
            let abs_path = root.join(rel_path);
            let preloaded = content_hint.and_then(|h| h.get(rel_path.as_str()).map(String::as_str));
            let parsed = parse_single_file(&abs_path, Path::new(rel_path), preloaded);
            (rel_path.clone(), parsed)
        })
        .collect();

    cache.extend(new_entries);
    cache
}

/// Parse a single file, returning parsed directives or errors.
///
/// When `preloaded` is `Some`, its content is used directly and no disk read
/// is performed. This avoids a redundant I/O round-trip in scan mode, where
/// `changes::from_directory` has already read every file once.
fn parse_single_file(abs_path: &Path, rel_path: &Path, preloaded: Option<&str>) -> ParsedFile {
    let content = if let Some(c) = preloaded {
        c.to_string()
    } else {
        match std::fs::read_to_string(abs_path) {
            Ok(c) => c,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::IsADirectory
                    || e.kind() == std::io::ErrorKind::PermissionDenied
                {
                    return ParsedFile::empty();
                }
                return ParsedFile {
                    directives: vec![],
                    errors: vec![DirectiveError {
                        line: NonZeroUsize::MIN,
                        message: format!("failed to read file: {e}"),
                    }],
                };
            }
        }
    };

    // Skip binary files
    if crate::changes::is_binary(&content) {
        return ParsedFile::empty();
    }

    // Quick check: skip files with no LINT directives
    if !content.contains("LINT.") {
        return ParsedFile::empty();
    }

    let (directives, mut errors) = grammar::parse_directives(&content, rel_path);
    let rel_str = rel_path.to_string_lossy();
    errors.extend(validate_uniqueness(&directives, &rel_str));

    ParsedFile { directives, errors }
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
                .map(|f| f.strip_prefix("//").unwrap_or(f).to_string())
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

fn validate_target(
    source_file: &str,
    pair: &DirectivePair,
    target: &Target,
    ctx: &ValidationContext<'_>,
    result: &mut CheckResult,
) {
    let (target_str, target_label) = match (&target.file, &target.label) {
        (Some(file), label) => {
            let rel = file.strip_prefix("//").unwrap_or(file);
            (rel.to_string(), label.clone())
        }
        (None, Some(label)) => (source_file.to_string(), Some(label.clone())),
        (None, None) => return,
    };

    // Same-file reference check.
    //
    // When the resolved target file equals the source file, require the caller to
    // use the short `:label` syntax instead of `//path/to/file:label`.
    //
    // `(None, Some(label))` — already `:label` form, resolved to source_file above — is fine.
    // `(Some(_), _)` with a matching path — the `//` form for the same file — is always an error:
    //   • without a label: a file referencing itself wholesale is meaningless.
    //   • with a label: must use `:label` instead of `//same-file:label`.
    if target.file.is_some() && target_str == source_file {
        let message = match &target_label {
            None => "self-referencing ThenChange without label is meaningless".to_string(),
            Some(label) => format!(
                "use ':{label}' syntax for same-file label references \
                 (replace '{raw}' with ':{label}')",
                raw = target.raw,
            ),
        };
        result.findings.push(mk_finding(
            &FindingCtx {
                source_file,
                pair,
                target,
            },
            message,
        ));
        return;
    }

    // Check if target file exists, using a cache to avoid repeated stat calls
    // for the same path when multiple ThenChange blocks reference the same target.
    let file_exists = ctx.cache.contains_key(&target_str)
        || *ctx
            .exists_cache
            .borrow_mut()
            .entry(target_str.clone())
            .or_insert_with(|| ctx.root.join(&target_str).exists());
    if !file_exists {
        result.findings.push(mk_finding(
            &FindingCtx {
                source_file,
                pair,
                target,
            },
            format!("target file not found: {target_str}"),
        ));
        return;
    }

    let target_sorted = ctx.sorted_lines.get(target_str.as_str());
    check_target_modified(
        &FindingCtx {
            source_file,
            pair,
            target,
        },
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

#[cfg(test)]
#[path = "check_test.rs"]
mod tests;
