use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Mutex;

use rayon::prelude::*;

use crate::parser::{Directive, DirectiveError, Target};
use crate::vcs::{ChangeMap, FileChanges, VcsProvider};

// ─── Public types ───

#[derive(Debug, Clone, serde::Serialize)]
pub struct Finding {
    pub source_file: String,
    pub source_line: NonZeroUsize,
    pub source_label: Option<String>,
    pub target_raw: String,
    pub then_change_line: NonZeroUsize,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ParseError {
    pub file: String,
    pub line: NonZeroUsize,
    pub message: String,
}

impl Finding {
    pub fn source_location(&self) -> String {
        format!("{}:{}", self.source_file, self.source_line)
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

#[derive(Debug, Default, serde::Serialize)]
pub struct CheckResult {
    pub findings: Vec<Finding>,
    pub parse_errors: Vec<ParseError>,
}

// ─── Public API ───

pub fn check(
    vcs: &dyn VcsProvider,
    changes: &ChangeMap,
    ignore_patterns: &[globset::GlobMatcher],
) -> CheckResult {
    let mut result = CheckResult::default();
    let vcs_validate_files = vcs.validate_files();

    // When no file list was provided but a diff is present, auto-populate
    // the structural validation set from changed (non-deleted) files.
    let derived_validate_files: Option<Vec<String>> =
        if vcs_validate_files.is_empty() && !changes.is_empty() {
            Some(
                changes
                    .iter()
                    .filter(|(_, fc)| !fc.deleted)
                    .map(|(path, _)| path.clone())
                    .collect(),
            )
        } else {
            None
        };
    let validate_files: &[String] = derived_validate_files
        .as_deref()
        .unwrap_or(vcs_validate_files);

    // Pass 1: parse changed files + validate_files.
    let changed_files: Vec<String> = changes.keys().cloned().collect();
    let seed_files: Vec<String> = changes
        .keys()
        .chain(
            validate_files
                .iter()
                .filter(|f| !changes.contains_key(f.as_str())),
        )
        .cloned()
        .collect();
    let cache = parse_files(&seed_files, vcs, HashMap::new());

    // Pass 2: parse target files referenced by ThenChange directives.
    let target_paths = collect_target_paths(vcs, &cache, &seed_files);
    let mut cache = parse_files(&target_paths, vcs, cache);

    // Modified-but-missing indicates a stale patch or race condition.
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

    for file_str in validate_files {
        if !cache.contains_key(file_str) {
            result.parse_errors.push(ParseError {
                file: file_str.clone(),
                line: NonZeroUsize::MIN,
                message: "file not found on disk".to_string(),
            });
        }
    }

    for (path, parsed) in &cache {
        for err in &parsed.errors {
            result
                .parse_errors
                .push(ParseError::from_directive_error(err, path));
        }
        result
            .parse_errors
            .extend(validate_structure(&parsed.directives, path, vcs));
    }

    // Pre-compute sorted line indices for efficient range queries.
    let sorted_lines: HashMap<&str, SortedLines> = changes
        .iter()
        .filter(|(path, _)| cache.contains_key(path.as_str()))
        .map(|(path, fc)| (path.as_str(), SortedLines::from_changes(fc)))
        .collect();

    let file_list_set: HashSet<&str> = validate_files.iter().map(|s| s.as_str()).collect();

    // Scoped so `cache` becomes mutable again for pass 3.
    {
        let ctx = ValidationContext {
            cache: &cache,
            sorted_lines: &sorted_lines,
            diff: changes,
            vcs,
            exists_cache: Mutex::new(HashMap::new()),
        };

        // Structural validity pass (triggered pairs are left to the diff pass).
        let structural: Vec<CheckResult> = validate_files
            .par_iter()
            .filter_map(|file_str| {
                let parsed = cache.get(file_str)?;
                let sorted = sorted_lines.get(file_str.as_str());
                let pairs = build_pairs(&parsed.directives);
                let mut file_result = CheckResult::default();
                for pair in &pairs {
                    if is_triggered(pair, sorted) {
                        continue; // diff pass handles triggered pairs
                    }
                    for target in &pair.targets {
                        if should_ignore(ctx.vcs, target, file_str, ignore_patterns) {
                            continue;
                        }
                        let r = validate_target_exists(file_str, pair, target, &ctx);
                        file_result.findings.extend(r.findings);
                        file_result.parse_errors.extend(r.parse_errors);
                    }
                }
                if file_result.findings.is_empty() && file_result.parse_errors.is_empty() {
                    None
                } else {
                    Some(file_result)
                }
            })
            .collect();
        for r in structural {
            result.findings.extend(r.findings);
            result.parse_errors.extend(r.parse_errors);
        }

        // Diff-based cross-file validation (scoped to file list when present).
        // Note: when `derived_validate_files` is active, `file_list_set` is
        // non-empty (all changed non-deleted files), so the filter below takes
        // the allow-list branch — but its outcome is equivalent to the all-pass
        // branch since every changed file is already in the set.
        let diff_results: Vec<CheckResult> = changed_files
            .par_iter()
            .filter(|f| file_list_set.is_empty() || file_list_set.contains(f.as_str()))
            .filter_map(|file_str| {
                let parsed = cache.get(file_str)?;
                let sorted = sorted_lines.get(file_str.as_str());
                let pairs = build_pairs(&parsed.directives);
                let mut file_result = CheckResult::default();
                for pair in &pairs {
                    if !is_triggered(pair, sorted) {
                        continue;
                    }
                    for target in &pair.targets {
                        if should_ignore(ctx.vcs, target, file_str, ignore_patterns) {
                            continue;
                        }
                        let r = validate_target(file_str, pair, target, &ctx);
                        file_result.findings.extend(r.findings);
                        file_result.parse_errors.extend(r.parse_errors);
                    }
                }
                if file_result.findings.is_empty() && file_result.parse_errors.is_empty() {
                    None
                } else {
                    Some(file_result)
                }
            })
            .collect();
        for r in diff_results {
            result.findings.extend(r.findings);
            result.parse_errors.extend(r.parse_errors);
        }
    }

    // Pass 3: reverse lookup — find surviving files that reference a deleted
    // target or a stale label.
    let deleted_set: HashSet<&str> = changes
        .iter()
        .filter(|(_, fc)| fc.deleted)
        .map(|(path, _)| path.as_str())
        .collect();

    let label_sets = build_label_sets(changes, &cache, &sorted_lines);

    if !deleted_set.is_empty() || !label_sets.is_empty() {
        check_stale_references(
            &deleted_set,
            &label_sets,
            &sorted_lines,
            &mut cache,
            vcs,
            ignore_patterns,
            &file_list_set,
            &mut result,
        );
    }

    // Sort for deterministic output (parallel passes produce arbitrary order).
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

/// Files that do not exist are omitted from the cache so callers can
/// detect their absence via `cache.contains_key`.
fn parse_files(paths: &[String], vcs: &dyn VcsProvider, mut cache: ParseCache) -> ParseCache {
    let new_entries: Vec<(String, ParsedFile)> = paths
        .par_iter()
        .filter(|rel_path| !cache.contains_key(rel_path.as_str()))
        .filter_map(|rel_path| {
            let parsed = parse_single_file(rel_path, vcs)?;
            Some((rel_path.clone(), parsed))
        })
        .collect();

    cache.extend(new_entries);
    cache
}

/// Returns `None` when the file does not exist (vs "unreadable" or "empty").
fn parse_single_file(rel_path: &str, vcs: &dyn VcsProvider) -> Option<ParsedFile> {
    let content = match vcs.read_file(rel_path) {
        Ok(Some(c)) => c,
        Ok(None) => return None,
        Err(e) => {
            return Some(ParsedFile {
                directives: vec![],
                errors: vec![DirectiveError {
                    line: NonZeroUsize::MIN,
                    message: format!("failed to read file: {e}"),
                }],
            });
        }
    };

    if crate::vcs::is_binary(&content) {
        return Some(ParsedFile::empty());
    }

    if !content.contains("LINT.") {
        return Some(ParsedFile::empty());
    }

    let (directives, mut errors) = crate::parser::parse(&content, rel_path);
    errors.extend(validate_uniqueness(&directives, rel_path));

    Some(ParsedFile { directives, errors })
}

fn validate_uniqueness(directives: &[Directive], file_path: &str) -> Vec<DirectiveError> {
    let mut errors = Vec::new();
    let mut if_labels: HashMap<String, NonZeroUsize> = HashMap::new();

    for d in directives {
        if let Directive::IfChange {
            line,
            label: Some(label),
        } = d
        {
            if let Some(prev_line) = if_labels.get(label) {
                errors.push(DirectiveError {
                    line: *line,
                    message: format!(
                        "duplicate LINT.IfChange label {label} (first at {file_path}:{prev_line})"
                    ),
                });
            } else {
                if_labels.insert(label.clone(), *line);
            }
        }
    }

    errors
}

fn collect_target_paths(
    vcs: &dyn VcsProvider,
    cache: &ParseCache,
    source_files: &[String],
) -> Vec<String> {
    let mut paths = Vec::new();
    for source in source_files {
        let Some(parsed) = cache.get(source) else {
            continue;
        };
        for d in &parsed.directives {
            if let Directive::ThenChange { targets, .. } = d {
                for t in targets {
                    if let Some(file) = t.file.as_deref() {
                        if let Some(resolved) = vcs.resolve_path(file) {
                            paths.push(resolve_relative_to_source(resolved, file, source));
                        }
                    }
                }
            }
        }
    }
    paths.sort_unstable();
    paths.dedup();
    paths
}

// ─── Structural validation ───

fn validate_structure(
    directives: &[Directive],
    file_path: &str,
    vcs: &dyn VcsProvider,
) -> Vec<ParseError> {
    let mut errors = Vec::new();
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
            Directive::ThenChange { line, targets } => {
                if pending_if.is_none() {
                    errors.push(ParseError {
                        file: file_path.to_string(),
                        line: *line,
                        message: "LINT.ThenChange without preceding IfChange".to_string(),
                    });
                }
                for target in targets {
                    if let Some(file) = &target.file {
                        if let Err(message) = vcs.try_resolve_path(file) {
                            errors.push(ParseError {
                                file: file_path.to_string(),
                                line: *line,
                                message,
                            });
                        }
                    }
                }
                pending_if = None;
            }
        }
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
        }
    }

    pairs
}

fn is_triggered(pair: &DirectivePair, sorted: Option<&SortedLines>) -> bool {
    let Some(sorted) = sorted else {
        return false;
    };

    let content_start = pair.if_line.get() + 1;
    let content_end = pair.then_line.get() - 1;

    // Brand-new pair: IfChange was added but not removed at its position.
    // The block is being established for the first time, not modifying prior
    // state — content lines may appear in `sorted.added` because the whole
    // block is new, not because they were changed. A label rename is
    // distinguishable because it both adds and removes the IfChange line.
    let if_change_added =
        SortedLines::any_in_range(&sorted.added, pair.if_line.get(), pair.if_line.get());
    let if_change_removed = SortedLines::any_in_range(
        &sorted.removed_new_pos,
        pair.if_line.get(),
        pair.if_line.get(),
    );
    if if_change_added && !if_change_removed {
        return false;
    }

    // Content additions between directives (exclusive of directive lines).
    if SortedLines::any_in_range(&sorted.added, content_start, content_end) {
        return true;
    }

    // Content removals between directives.
    // If ThenChange was replaced (then_line ∈ added), cap at content_end —
    // a removed_new_pos at then_line belongs to the ThenChange replacement,
    // not a content removal. Otherwise include then_line (content removal
    // right before ThenChange collapses there).
    let then_replaced =
        SortedLines::any_in_range(&sorted.added, pair.then_line.get(), pair.then_line.get());
    let removal_end = if then_replaced {
        content_end
    } else {
        pair.then_line.get()
    };
    if SortedLines::any_in_range(&sorted.removed_new_pos, content_start, removal_end) {
        return true;
    }

    false
}

/// Matches against both `target.raw` (verbatim text) and the resolved file path,
/// so `--ignore "generated/*"` matches `//generated/api.rs`.
fn should_ignore(
    vcs: &dyn VcsProvider,
    target: &Target,
    source_file: &str,
    ignore_patterns: &[globset::GlobMatcher],
) -> bool {
    ignore_patterns.iter().any(|pattern| {
        pattern.is_match(&target.raw)
            || target
                .file
                .as_deref()
                .and_then(|f| {
                    let resolved = vcs.resolve_path(f)?;
                    Some(resolve_relative_to_source(resolved, f, source_file))
                })
                .is_some_and(|f| pattern.is_match(f))
    })
}

/// Result of a cached `file_exists` lookup.
///
/// - `Some(true)` — file exists
/// - `Some(false)` — file confirmed missing
/// - `None` — a VCS error was recorded for this path; don't retry or emit
///   a duplicate `ParseError`
type ExistsStatus = Option<bool>;

struct ValidationContext<'a> {
    cache: &'a ParseCache,
    sorted_lines: &'a HashMap<&'a str, SortedLines>,
    diff: &'a ChangeMap,
    vcs: &'a dyn VcsProvider,
    /// Cache for `vcs.file_exists()` — avoids repeated stat calls for the same path.
    exists_cache: Mutex<HashMap<String, ExistsStatus>>,
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

/// When `--strict=false` allows bare filenames (no `/`), resolve them relative
/// to the source file's directory. Paths that already contain a `/` are left
/// as-is (they are root-relative by convention).
fn resolve_relative_to_source(resolved: String, raw: &str, source_file: &str) -> String {
    if raw.contains('/') {
        return resolved;
    }
    match Path::new(source_file).parent() {
        Some(parent) if !parent.as_os_str().is_empty() => {
            let joined = parent.join(&resolved);
            joined.to_string_lossy().replace('\\', "/")
        }
        _ => resolved,
    }
}

/// Returns `None` when no validation is needed (unrecognised scheme or bare target).
/// Same-file `:label` references resolve to `(source_file, Some(label))`.
fn resolve_target(
    vcs: &dyn VcsProvider,
    target: &Target,
    source_file: &str,
) -> Option<(String, Option<String>)> {
    match (&target.file, &target.label) {
        (Some(file), label) => {
            let resolved = vcs.resolve_path(file)?;
            let resolved = resolve_relative_to_source(resolved, file, source_file);
            Some((resolved, label.clone()))
        }
        (None, Some(label)) => Some((source_file.to_string(), Some(label.clone()))),
        (None, None) => None,
    }
}

/// Returns `Some(finding)` when target is a same-file reference that should
/// not be processed further.
fn check_same_file_reference(
    fctx: &FindingCtx<'_>,
    target_str: &str,
    target_label: &Option<String>,
    ctx: &ValidationContext<'_>,
) -> Option<Finding> {
    if fctx.target.file.is_none() || target_str != fctx.source_file {
        return None;
    }
    // In non-strict mode, don't warn about same-file references using explicit
    // paths — codebases like Chromium commonly use `//file.h:label` form.
    if !ctx.vcs.is_strict() {
        return None;
    }
    let message = match target_label {
        None => "self-referencing ThenChange without label is meaningless".to_string(),
        Some(label) => format!(
            "use :{label} syntax for same-file label references \
             (replace {raw} with :{label})",
            raw = fctx.target.raw,
        ),
    };
    Some(mk_finding(fctx, message))
}

enum ExistsCheck {
    Exists,
    Missing(Finding),
    Error(ParseError),
    PriorError,
}

/// Check whether a target file exists, using a thread-safe cache.
fn check_file_exists(
    fctx: &FindingCtx<'_>,
    target_str: &str,
    ctx: &ValidationContext<'_>,
) -> ExistsCheck {
    if ctx.cache.contains_key(target_str) {
        return ExistsCheck::Exists;
    }

    // Fast path: check cache (short lock, no I/O).
    {
        let cache = ctx.exists_cache.lock().unwrap();
        if let Some(&cached) = cache.get(target_str) {
            return match cached {
                None => ExistsCheck::PriorError,
                Some(true) => ExistsCheck::Exists,
                Some(false) => {
                    let message = match &fctx.target.label {
                        Some(label) => {
                            format!("target file not found: {target_str} (label {label})")
                        }
                        None => format!("target file not found: {target_str}"),
                    };
                    ExistsCheck::Missing(mk_finding(fctx, message))
                }
            };
        }
    }

    // Slow path: stat the file without holding the lock.
    match ctx.vcs.file_exists(target_str) {
        Ok(exists) => {
            ctx.exists_cache
                .lock()
                .unwrap()
                .insert(target_str.to_string(), Some(exists));
            if exists {
                ExistsCheck::Exists
            } else {
                let message = match &fctx.target.label {
                    Some(label) => {
                        format!("target file not found: {target_str} (label {label})")
                    }
                    None => format!("target file not found: {target_str}"),
                };
                ExistsCheck::Missing(mk_finding(fctx, message))
            }
        }
        Err(e) => {
            ctx.exists_cache
                .lock()
                .unwrap()
                .insert(target_str.to_string(), None);
            ExistsCheck::Error(ParseError {
                file: target_str.to_string(),
                line: NonZeroUsize::MIN,
                message: format!("failed to check file existence: {e}"),
            })
        }
    }
}

fn validate_target_exists(
    source_file: &str,
    pair: &DirectivePair,
    target: &Target,
    ctx: &ValidationContext<'_>,
) -> CheckResult {
    let mut result = CheckResult::default();
    let Some((target_str, target_label)) = resolve_target(ctx.vcs, target, source_file) else {
        return result;
    };
    let fctx = FindingCtx {
        source_file,
        pair,
        target,
    };
    if let Some(finding) = check_same_file_reference(&fctx, &target_str, &target_label, ctx) {
        result.findings.push(finding);
        return result;
    }
    // Same-file `:label` references don't need a file-existence check — the
    // source file is guaranteed to be in the cache since we just parsed it.
    if target_str == source_file && target.file.is_none() {
        if let Some(label_name) = &target_label {
            if find_label_range(&target_str, label_name, ctx.cache).is_none() {
                result.findings.push(mk_finding(
                    &fctx,
                    format!("label {label_name} not found in {target_str}"),
                ));
            }
        }
        return result;
    }
    match check_file_exists(&fctx, &target_str, ctx) {
        ExistsCheck::Exists => {}
        ExistsCheck::Missing(f) => {
            result.findings.push(f);
            return result;
        }
        ExistsCheck::Error(e) => {
            result.parse_errors.push(e);
            return result;
        }
        ExistsCheck::PriorError => return result,
    }
    if let Some(label_name) = &target_label {
        if find_label_range(&target_str, label_name, ctx.cache).is_none() {
            result.findings.push(mk_finding(
                &fctx,
                format!("label {label_name} not found in {target_str}"),
            ));
        }
    }
    result
}

fn validate_target(
    source_file: &str,
    pair: &DirectivePair,
    target: &Target,
    ctx: &ValidationContext<'_>,
) -> CheckResult {
    let mut result = CheckResult::default();
    let Some((target_str, target_label)) = resolve_target(ctx.vcs, target, source_file) else {
        return result;
    };
    let fctx = FindingCtx {
        source_file,
        pair,
        target,
    };
    if let Some(finding) = check_same_file_reference(&fctx, &target_str, &target_label, ctx) {
        result.findings.push(finding);
        return result;
    }
    match check_file_exists(&fctx, &target_str, ctx) {
        ExistsCheck::Exists => {}
        ExistsCheck::Missing(f) => {
            result.findings.push(f);
            return result;
        }
        ExistsCheck::Error(e) => {
            result.parse_errors.push(e);
            return result;
        }
        ExistsCheck::PriorError => return result,
    }
    let target_sorted = ctx.sorted_lines.get(target_str.as_str());
    if let Some(finding) = check_target_modified(
        &fctx,
        &target_str,
        target_label.as_deref(),
        target_sorted,
        ctx,
    ) {
        result.findings.push(finding);
    }
    result
}

fn check_target_modified(
    fctx: &FindingCtx<'_>,
    target_str: &str,
    target_label: Option<&str>,
    target_sorted: Option<&SortedLines>,
    ctx: &ValidationContext<'_>,
) -> Option<Finding> {
    if let Some(label_name) = target_label {
        let label_range = find_label_range(target_str, label_name, ctx.cache);
        match label_range {
            Some(range) => {
                // Removed positions use end+1: a removal on the ThenChange line
                // still indicates the label region was modified.
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
                    return Some(mk_finding(
                        fctx,
                        format!("changes in this block may need to be reflected in {target_str}:{label_name}"),
                    ));
                }
            }
            None => {
                return Some(mk_finding(
                    fctx,
                    format!("label {label_name} not found in {target_str}"),
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
            return Some(mk_finding(
                fctx,
                format!("changes in this block may need to be reflected in {target_str}"),
            ));
        }
    }
    None
}

#[derive(Debug, Clone)]
struct LabelRange {
    start: usize,
    end: usize,
}

/// Returns the content line range between `LINT.IfChange("name")` and its
/// `LINT.ThenChange`, or `None` if the label is not found.
fn find_label_range(file_path: &str, label_name: &str, cache: &ParseCache) -> Option<LabelRange> {
    let parsed = cache.get(file_path)?;

    let mut if_content_start: Option<usize> = None;

    for d in &parsed.directives {
        match d {
            Directive::IfChange { line, label } => {
                // Non-matching IfChange must not clear a previously matched
                // if_content_start (handles malformed consecutive IfChange).
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

/// Build the set of current IfChange labels for each changed file that may
/// have had labels added, renamed, or removed. Uses owned strings to avoid
/// borrowing cache across the mutable pass.
///
/// Includes a file when:
///   - It has no IfChange directives (all labeled pairs may have been removed).
///   - An IfChange line appears in `sorted.added` (label may be new or renamed).
///   - Lines were removed and IfChange directives exist (a labeled pair may
///     have been partially removed — we can't see deleted directives).
///
/// Skips files where IfChange directives exist, no IfChange was added, and
/// nothing was removed — labels are intact, no stale references introduced.
fn build_label_sets(
    changes: &ChangeMap,
    cache: &ParseCache,
    sorted_lines: &HashMap<&str, SortedLines>,
) -> HashMap<String, HashSet<String>> {
    changes
        .iter()
        .filter(|(_, fc)| !fc.deleted)
        .filter_map(|(path, _)| {
            let parsed = cache.get(path)?;
            let has_if_change = parsed
                .directives
                .iter()
                .any(|d| matches!(d, Directive::IfChange { .. }));
            if has_if_change {
                // sorted_lines is keyed by all non-deleted changed files that
                // are in cache — same preconditions as reaching here. The `?`
                // is a defensive fallback; it should never return None.
                let sorted = sorted_lines.get(path.as_str())?;
                let any_if_added = parsed.directives.iter().any(|d| {
                    if let Directive::IfChange { line, .. } = d {
                        SortedLines::any_in_range(&sorted.added, line.get(), line.get())
                    } else {
                        false
                    }
                });
                // Skip only when no IfChange was added AND nothing was removed.
                // If lines were removed, a labeled pair may have been deleted —
                // we cannot see it in the parsed output, so include the file.
                if !any_if_added && sorted.removed_new_pos.is_empty() {
                    return None;
                }
            }
            let labels: HashSet<String> = parsed
                .directives
                .iter()
                .filter_map(|d| match d {
                    Directive::IfChange { label: Some(l), .. } => Some(l.clone()),
                    _ => None,
                })
                .collect();
            Some((path.clone(), labels))
        })
        .collect()
}

/// Find surviving files whose ThenChange targets reference a deleted file
/// or a stale label. Uses a two-stage substring filter (`LINT.` then
/// deleted/label-tracked path) to avoid parsing the vast majority of files.
#[allow(clippy::too_many_arguments)]
fn check_stale_references(
    deleted: &HashSet<&str>,
    label_sets: &HashMap<String, HashSet<String>>,
    sorted_lines: &HashMap<&str, SortedLines>,
    cache: &mut ParseCache,
    vcs: &dyn VcsProvider,
    ignore_patterns: &[globset::GlobMatcher],
    file_list_set: &HashSet<&str>,
    result: &mut CheckResult,
) {
    let candidates = match vcs.search_files("LINT.") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: reverse-lookup skipped: {e}");
            return;
        }
    };

    let scan_results = scan_stale_references(
        &candidates,
        deleted,
        label_sets,
        sorted_lines,
        cache,
        vcs,
        ignore_patterns,
        file_list_set,
    );

    for (findings, new_entry) in scan_results {
        result.findings.extend(findings);
        if let Some((path, parsed)) = new_entry {
            cache.insert(path, parsed);
        }
    }
}

type ScanEntry = (Vec<Finding>, Option<(String, ParsedFile)>);

/// Parallel scan phase — reads from cache but never mutates it.
#[allow(clippy::too_many_arguments)]
fn scan_stale_references(
    candidates: &[String],
    deleted: &HashSet<&str>,
    label_sets: &HashMap<String, HashSet<String>>,
    sorted_lines: &HashMap<&str, SortedLines>,
    cache: &ParseCache,
    vcs: &dyn VcsProvider,
    ignore_patterns: &[globset::GlobMatcher],
    file_list_set: &HashSet<&str>,
) -> Vec<ScanEntry> {
    candidates
        .par_iter()
        .filter_map(|rel_str| {
            if deleted.contains(rel_str.as_str()) {
                return None;
            }
            // Structural pass already handles file_list files.
            if file_list_set.contains(rel_str.as_str()) {
                return None;
            }

            let pairs;
            let new_entry: Option<(String, ParsedFile)>;
            if let Some(parsed) = cache.get(rel_str) {
                if parsed.directives.is_empty() {
                    return None;
                }
                pairs = build_pairs(&parsed.directives);
                new_entry = None;
            } else {
                let content = vcs.read_file(rel_str).ok()??;

                if crate::vcs::is_binary(&content) {
                    return None;
                }

                // Quick content filter: skip files that don't mention any
                // deleted path or any file with tracked labels.
                let mentions_deleted = deleted.iter().any(|d| content.contains(*d));
                let mentions_label_file = label_sets.keys().any(|f| content.contains(f.as_str()));
                if !mentions_deleted && !mentions_label_file {
                    return None;
                }

                let (directives, errors) = crate::parser::parse(&content, rel_str);
                pairs = build_pairs(&directives);
                new_entry = Some((rel_str.clone(), ParsedFile { directives, errors }));
            };
            let sorted = sorted_lines.get(rel_str.as_str());

            let mut findings = Vec::new();
            for pair in &pairs {
                if is_triggered(pair, sorted) {
                    continue;
                }
                for target in &pair.targets {
                    if should_ignore(vcs, target, rel_str, ignore_patterns) {
                        continue;
                    }
                    let Some(file) = &target.file else { continue };
                    let Some(resolved) = vcs.resolve_path(file) else {
                        continue;
                    };
                    let target_str = resolve_relative_to_source(resolved, file, rel_str);

                    // Deleted file reference.
                    if deleted.contains(target_str.as_str()) {
                        let message = match &target.label {
                            Some(label) => {
                                format!("target file not found: {target_str} (label {label})")
                            }
                            None => format!("target file not found: {target_str}"),
                        };
                        findings.push(mk_finding(
                            &FindingCtx {
                                source_file: rel_str,
                                pair,
                                target,
                            },
                            message,
                        ));
                        continue;
                    }

                    // Stale label reference.
                    if let Some(label_name) = &target.label {
                        if let Some(valid_labels) = label_sets.get(&target_str) {
                            if !valid_labels.contains(label_name) {
                                findings.push(mk_finding(
                                    &FindingCtx {
                                        source_file: rel_str,
                                        pair,
                                        target,
                                    },
                                    format!("label {label_name} not found in {target_str}"),
                                ));
                            }
                        }
                    }
                }
            }
            Some((findings, new_entry))
        })
        .collect()
}

#[cfg(test)]
#[path = "check_test.rs"]
mod tests;
