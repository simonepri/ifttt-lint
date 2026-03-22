use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Mutex;

use rayon::prelude::*;

use crate::parser::{Directive, DirectiveError, Target};
use crate::vcs::{ChangeMap, FileChanges, VcsProvider};

/// Variant order defines sort priority (errors before warnings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TargetInfo {
    pub label: Option<String>,
    pub raw: String,
    pub then_change_line: NonZeroUsize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Diagnostic {
    pub file: String,
    pub line: NonZeroUsize,
    pub severity: Severity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<TargetInfo>,
}

impl Diagnostic {
    pub fn location(&self) -> String {
        format!("{}:{}", self.file, self.line)
    }
}

pub type CheckResult = Vec<Diagnostic>;

struct ValidationScope {
    validate_files: Vec<String>,
    changed_files: Vec<String>,
    seed_files: Vec<String>,
    file_list_set: HashSet<String>,
}

pub fn check(
    vcs: &dyn VcsProvider,
    changes: &ChangeMap,
    ignore_patterns: &[globset::GlobMatcher],
) -> CheckResult {
    let scope = build_validation_scope(vcs.validate_files(), changes);
    let mut result: CheckResult = Vec::new();
    let mut cache = parse_initial_cache(vcs, &scope);

    collect_seed_parse_errors(&scope, changes, &cache, vcs, &mut result);

    let sorted_lines = build_sorted_lines(changes, &cache);
    run_validation(
        &scope,
        &cache,
        &sorted_lines,
        changes,
        vcs,
        ignore_patterns,
        &mut result,
    );
    run_reverse_lookup_pass(
        &scope,
        &sorted_lines,
        changes,
        &mut cache,
        vcs,
        ignore_patterns,
        &mut result,
    );

    sort_result(&mut result);
    result
}

fn build_validation_scope(vcs_validate_files: &[String], changes: &ChangeMap) -> ValidationScope {
    let validate_files = if !vcs_validate_files.is_empty() {
        vcs_validate_files.to_vec()
    } else if changes.is_empty() {
        Vec::new()
    } else {
        changes
            .iter()
            .filter(|(_, fc)| !fc.deleted)
            .map(|(path, _)| path.clone())
            .collect()
    };
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
    let file_list_set = validate_files.iter().cloned().collect();

    ValidationScope {
        validate_files,
        changed_files,
        seed_files,
        file_list_set,
    }
}

fn parse_initial_cache(vcs: &dyn VcsProvider, scope: &ValidationScope) -> ParseCache {
    let cache = parse_files(&scope.seed_files, vcs, HashMap::new());
    let target_paths = collect_target_paths(vcs, &cache, &scope.seed_files);
    parse_files(&target_paths, vcs, cache)
}

fn collect_seed_parse_errors(
    scope: &ValidationScope,
    changes: &ChangeMap,
    cache: &ParseCache,
    vcs: &dyn VcsProvider,
    result: &mut CheckResult,
) {
    for file_str in &scope.changed_files {
        if cache.contains_key(file_str) {
            continue;
        }

        if changes.get(file_str).is_some_and(|c| !c.deleted) {
            result.push(mk_error(
                file_str,
                NonZeroUsize::MIN,
                "file referenced in diff but not found on disk".to_string(),
            ));
        }
    }

    for file_str in &scope.validate_files {
        if cache.contains_key(file_str) {
            continue;
        }

        result.push(mk_error(
            file_str,
            NonZeroUsize::MIN,
            "file not found on disk".to_string(),
        ));
    }

    for path in &scope.seed_files {
        let Some(parsed) = cache.get(path) else {
            continue;
        };
        for err in &parsed.errors {
            result.push(mk_error(path, err.line, err.message.clone()));
        }
        result.extend(validate_structure(&parsed.directives, path, vcs));
    }
}

fn build_sorted_lines<'a>(
    changes: &'a ChangeMap,
    cache: &ParseCache,
) -> HashMap<&'a str, SortedLines> {
    changes
        .iter()
        .filter(|(path, _)| cache.contains_key(path.as_str()))
        .map(|(path, fc)| (path.as_str(), SortedLines::from_changes(fc)))
        .collect()
}

/// Merged structural + diff pass. For each validate_file, iterates all
/// IfChange/ThenChange pairs — triggered pairs get diff checks (was the
/// target modified?), non-triggered pairs get structural checks (does the
/// target/label exist?).
fn run_validation(
    scope: &ValidationScope,
    cache: &ParseCache,
    sorted_lines: &HashMap<&str, SortedLines>,
    changes: &ChangeMap,
    vcs: &dyn VcsProvider,
    ignore_patterns: &[globset::GlobMatcher],
    result: &mut CheckResult,
) {
    let ctx = CheckContext {
        cache,
        sorted_lines,
        diff: changes,
        vcs,
        exists_cache: Mutex::new(HashMap::new()),
    };

    let diagnostics: Vec<CheckResult> = scope
        .validate_files
        .par_iter()
        .filter_map(|file_str| {
            let parsed = cache.get(file_str)?;
            let sorted = sorted_lines.get(file_str.as_str());
            let pairs = build_pairs(&parsed.directives);
            let mut file_result: CheckResult = Vec::new();
            for (pair, target, triggered) in
                active_targets(&pairs, sorted, file_str, ctx.vcs, ignore_patterns)
            {
                match resolve_target(file_str, pair, target, &ctx) {
                    TargetResolution::Resolved { path, label } => {
                        let dctx = DiagnosticCtx {
                            source_file: file_str,
                            pair,
                            target,
                        };
                        if triggered {
                            let target_sorted = ctx.sorted_lines.get(path.as_str());
                            file_result.extend(check_target_synced(
                                &dctx,
                                &path,
                                label.as_deref(),
                                target_sorted,
                                &ctx,
                            ));
                        } else {
                            file_result.extend(check_label_exists(
                                &dctx,
                                &path,
                                label.as_deref(),
                                ctx.cache,
                            ));
                        }
                    }
                    TargetResolution::SameFileWarning(d)
                    | TargetResolution::Missing(d)
                    | TargetResolution::Error(d) => file_result.push(d),
                    TargetResolution::NoTarget | TargetResolution::PriorError => {}
                }
            }
            (!file_result.is_empty()).then_some(file_result)
        })
        .collect();

    result.extend(diagnostics.into_iter().flatten());
}

fn run_reverse_lookup_pass(
    scope: &ValidationScope,
    sorted_lines: &HashMap<&str, SortedLines>,
    changes: &ChangeMap,
    cache: &mut ParseCache,
    vcs: &dyn VcsProvider,
    ignore_patterns: &[globset::GlobMatcher],
    result: &mut CheckResult,
) {
    let deleted_set: HashSet<&str> = changes
        .iter()
        .filter(|(_, fc)| fc.deleted)
        .map(|(path, _)| path.as_str())
        .collect();
    let label_sets = build_label_sets(changes, cache, sorted_lines);

    if deleted_set.is_empty() && label_sets.is_empty() {
        return;
    }

    let stale_ctx = StaleReferenceCtx {
        deleted: &deleted_set,
        label_sets: &label_sets,
        sorted_lines,
        vcs,
        ignore_patterns,
        file_list_set: &scope.file_list_set,
    };
    check_stale_references(&stale_ctx, cache, result);
}

fn sort_result(result: &mut CheckResult) {
    result.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(a.file.cmp(&b.file))
            .then(a.line.cmp(&b.line))
            .then(a.message.cmp(&b.message))
    });
}

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
type CacheInsert = Option<(String, ParsedFile)>;

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
        let Directive::IfChange {
            line,
            label: Some(label),
        } = d
        else {
            continue;
        };

        let Some(prev_line) = if_labels.get(label) else {
            if_labels.insert(label.clone(), *line);
            continue;
        };

        errors.push(DirectiveError {
            line: *line,
            message: format!(
                "duplicate LINT.IfChange label {label} (first at {file_path}:{prev_line})"
            ),
        });
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

        for directive in &parsed.directives {
            let Directive::ThenChange { targets, .. } = directive else {
                continue;
            };

            for target in targets {
                let Some(file) = target.file.as_deref() else {
                    continue;
                };
                let Some(path) = resolve_target_path(file, source, vcs) else {
                    continue;
                };

                paths.push(path);
            }
        }
    }
    paths.sort_unstable();
    paths.dedup();
    paths
}

fn validate_structure(
    directives: &[Directive],
    file_path: &str,
    vcs: &dyn VcsProvider,
) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    let mut pending_if: Option<NonZeroUsize> = None;

    for d in directives {
        match d {
            Directive::IfChange { line, .. } => {
                if let Some(prev_line) = pending_if {
                    errors.push(mk_error(
                        file_path,
                        prev_line,
                        format!(
                            "LINT.IfChange without matching ThenChange (previous IfChange at line {prev_line})"
                        ),
                    ));
                }
                pending_if = Some(*line);
            }
            Directive::ThenChange { line, targets } => {
                if pending_if.is_none() {
                    errors.push(mk_error(
                        file_path,
                        *line,
                        "LINT.ThenChange without preceding IfChange".to_string(),
                    ));
                }
                for target in targets {
                    let Some(file) = &target.file else {
                        continue;
                    };
                    let Err(message) = vcs.try_resolve_path(file) else {
                        continue;
                    };
                    errors.push(mk_error(file_path, *line, message));
                }
                pending_if = None;
            }
        }
    }

    if let Some(line) = pending_if {
        errors.push(mk_error(
            file_path,
            line,
            "LINT.IfChange without matching ThenChange".to_string(),
        ));
    }

    errors
}

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
                let Some((if_line, if_label)) = pending_if.take() else {
                    continue;
                };

                pairs.push(DirectivePair {
                    if_line,
                    if_label,
                    then_line: *line,
                    targets: targets.clone(),
                });
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
                .and_then(|f| resolve_target_path(f, source_file, vcs))
                .is_some_and(|f| pattern.is_match(f))
    })
}

/// Yields `(pair, target, triggered)` for each non-ignored target across all pairs.
fn active_targets<'a>(
    pairs: &'a [DirectivePair],
    sorted: Option<&SortedLines>,
    source_file: &str,
    vcs: &dyn VcsProvider,
    ignore_patterns: &[globset::GlobMatcher],
) -> Vec<(&'a DirectivePair, &'a Target, bool)> {
    let mut result = Vec::new();
    for pair in pairs {
        let triggered = is_triggered(pair, sorted);
        for target in &pair.targets {
            if should_ignore(vcs, target, source_file, ignore_patterns) {
                continue;
            }
            result.push((pair, target, triggered));
        }
    }
    result
}

#[derive(Debug, Clone, Copy)]
enum ExistsStatus {
    Exists,
    Missing,
    /// A VCS error was recorded for this path; don't retry or emit a duplicate error.
    PriorError,
}

struct CheckContext<'a> {
    cache: &'a ParseCache,
    sorted_lines: &'a HashMap<&'a str, SortedLines>,
    diff: &'a ChangeMap,
    vcs: &'a dyn VcsProvider,
    /// Cache for `vcs.file_exists()` — avoids repeated stat calls for the same path.
    exists_cache: Mutex<HashMap<String, ExistsStatus>>,
}

struct DiagnosticCtx<'a> {
    source_file: &'a str,
    pair: &'a DirectivePair,
    target: &'a Target,
}

fn target_diagnostic(ctx: &DiagnosticCtx<'_>, severity: Severity, message: String) -> Diagnostic {
    let DiagnosticCtx {
        source_file,
        pair,
        target,
    } = ctx;
    Diagnostic {
        file: source_file.to_string(),
        line: pair.if_line,
        severity,
        message,
        target: Some(TargetInfo {
            label: pair.if_label.clone(),
            raw: target.raw.clone(),
            then_change_line: pair.then_line,
        }),
    }
}

fn target_warning(ctx: &DiagnosticCtx<'_>, message: String) -> Diagnostic {
    target_diagnostic(ctx, Severity::Warning, message)
}

fn target_error(ctx: &DiagnosticCtx<'_>, message: String) -> Diagnostic {
    target_diagnostic(ctx, Severity::Error, message)
}

fn mk_error(file: &str, line: NonZeroUsize, message: String) -> Diagnostic {
    Diagnostic {
        file: file.to_string(),
        line,
        severity: Severity::Error,
        message,
        target: None,
    }
}

/// Resolve a target's file path through VCS and relativize to the source.
fn resolve_target_path(file: &str, source_file: &str, vcs: &dyn VcsProvider) -> Option<String> {
    let resolved = vcs.resolve_path(file)?;
    Some(resolve_relative_to_source(resolved, file, source_file))
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

fn missing_target_message(target_str: &str, label: Option<&str>) -> String {
    match label {
        Some(label) => format!("target file not found: {target_str} (label {label})"),
        None => format!("target file not found: {target_str}"),
    }
}

/// Pure resolution result — no side effects, caller dispatches.
enum TargetResolution {
    Resolved {
        path: String,
        label: Option<String>,
    },
    SameFileWarning(Diagnostic),
    Missing(Diagnostic),
    Error(Diagnostic),
    /// Nothing to validate — bare target without file or label.
    NoTarget,
    /// A VCS error was already recorded for this path; skip silently.
    PriorError,
}

/// Resolve a ThenChange target, check same-file references, and verify the
/// target file exists. Pure — returns a `TargetResolution`, never mutates
/// external state (aside from the thread-safe exists_cache).
fn resolve_target(
    source_file: &str,
    pair: &DirectivePair,
    target: &Target,
    ctx: &CheckContext<'_>,
) -> TargetResolution {
    // Step 1: Path resolution.
    let (target_str, target_label) = match (&target.file, &target.label) {
        (Some(file), label) => {
            let Some(path) = resolve_target_path(file, source_file, ctx.vcs) else {
                return TargetResolution::PriorError;
            };
            (path, label.clone())
        }
        (None, Some(label)) => (source_file.to_string(), Some(label.clone())),
        (None, None) => return TargetResolution::NoTarget,
    };

    let fctx = DiagnosticCtx {
        source_file,
        pair,
        target,
    };

    // Step 2: Same-file reference check (strict mode only).
    if target.file.is_some() && target_str == source_file && ctx.vcs.is_strict() {
        let message = match &target_label {
            None => "self-referencing ThenChange without label is meaningless".to_string(),
            Some(label) => format!(
                "use :{label} syntax for same-file label references \
                 (replace {raw} with :{label})",
                raw = target.raw,
            ),
        };
        return TargetResolution::SameFileWarning(target_warning(&fctx, message));
    }

    // Step 3: File existence check.
    if ctx.cache.contains_key(target_str.as_str()) {
        return TargetResolution::Resolved {
            path: target_str,
            label: target_label,
        };
    }

    // Fast path: check exists_cache (short lock, no I/O).
    {
        let cache = ctx.exists_cache.lock().unwrap();
        if let Some(&cached) = cache.get(target_str.as_str()) {
            return match cached {
                ExistsStatus::PriorError => TargetResolution::PriorError,
                ExistsStatus::Exists => TargetResolution::Resolved {
                    path: target_str,
                    label: target_label,
                },
                ExistsStatus::Missing => TargetResolution::Missing(target_error(
                    &fctx,
                    missing_target_message(&target_str, target.label.as_deref()),
                )),
            };
        }
    }

    // Slow path: stat the file without holding the lock.
    match ctx.vcs.file_exists(&target_str) {
        Ok(exists) => {
            let status = if exists {
                ExistsStatus::Exists
            } else {
                ExistsStatus::Missing
            };
            ctx.exists_cache
                .lock()
                .unwrap()
                .insert(target_str.clone(), status);
            if exists {
                TargetResolution::Resolved {
                    path: target_str,
                    label: target_label,
                }
            } else {
                TargetResolution::Missing(target_error(
                    &fctx,
                    missing_target_message(&target_str, target.label.as_deref()),
                ))
            }
        }
        Err(e) => {
            ctx.exists_cache
                .lock()
                .unwrap()
                .insert(target_str.clone(), ExistsStatus::PriorError);
            TargetResolution::Error(mk_error(
                &target_str,
                NonZeroUsize::MIN,
                format!("failed to check file existence: {e}"),
            ))
        }
    }
}

/// Structural check: does the target label exist?
fn check_label_exists(
    fctx: &DiagnosticCtx<'_>,
    path: &str,
    label: Option<&str>,
    cache: &ParseCache,
) -> Option<Diagnostic> {
    let label_name = label?;
    if find_label_range(path, label_name, cache).is_some() {
        return None;
    }
    Some(target_error(
        fctx,
        format!("label {label_name} not found in {path}"),
    ))
}

fn check_target_synced(
    fctx: &DiagnosticCtx<'_>,
    target_str: &str,
    target_label: Option<&str>,
    target_sorted: Option<&SortedLines>,
    ctx: &CheckContext<'_>,
) -> Option<Diagnostic> {
    let Some(label_name) = target_label else {
        let has_changes = ctx
            .diff
            .get(target_str)
            .map(|c| !c.added_lines.is_empty() || !c.removed_lines.is_empty())
            .unwrap_or(false);

        if has_changes {
            return None;
        }

        return Some(target_warning(
            fctx,
            format!("changes in this block may need to be reflected in {target_str}"),
        ));
    };

    if !ctx.cache.contains_key(target_str) {
        return Some(target_warning(
            fctx,
            format!("changes in this block may need to be reflected in {target_str}:{label_name}"),
        ));
    }

    let Some(range) = find_label_range(target_str, label_name, ctx.cache) else {
        return Some(target_error(
            fctx,
            format!("label {label_name} not found in {target_str}"),
        ));
    };

    // Removed positions use end+1: a removal on the ThenChange line still
    // indicates the label region was modified.
    let has_change_in_range = target_sorted
        .map(|s| {
            SortedLines::any_in_range(&s.added, range.start, range.end)
                || SortedLines::any_in_range(&s.removed_new_pos, range.start, range.end + 1)
        })
        .unwrap_or(false);
    if has_change_in_range {
        return None;
    }

    Some(target_warning(
        fctx,
        format!("changes in this block may need to be reflected in {target_str}:{label_name}"),
    ))
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
                if label.as_ref().map(|l| l.as_str()) == Some(label_name) {
                    if_content_start = Some(line.get() + 1);
                } else if let Some(start) = if_content_start.take() {
                    // Malformed: a non-matching IfChange appeared before
                    // the matching ThenChange. Cap the range here rather
                    // than extending into a different pair's content.
                    // validate_structure catches this separately.
                    return Some(LabelRange {
                        start,
                        end: line.get() - 1,
                    });
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

struct StaleReferenceCtx<'a> {
    deleted: &'a HashSet<&'a str>,
    label_sets: &'a HashMap<String, HashSet<String>>,
    sorted_lines: &'a HashMap<&'a str, SortedLines>,
    vcs: &'a dyn VcsProvider,
    ignore_patterns: &'a [globset::GlobMatcher],
    file_list_set: &'a HashSet<String>,
}

/// Find surviving files whose ThenChange targets reference a deleted file
/// or a stale label. Uses a two-stage substring filter (`LINT.` then
/// deleted/label-tracked path) to avoid parsing the vast majority of files.
fn check_stale_references(
    ctx: &StaleReferenceCtx<'_>,
    cache: &mut ParseCache,
    result: &mut CheckResult,
) {
    let candidates = match ctx.vcs.search_files("LINT.") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("warning: reverse-lookup skipped: {e}");
            return;
        }
    };

    let scan_results = scan_stale_references(&candidates, ctx, cache);

    for (diagnostics, new_entry) in scan_results {
        result.extend(diagnostics);
        if let Some((path, parsed)) = new_entry {
            cache.insert(path, parsed);
        }
    }
}

/// Parallel scan phase — reads from cache but never mutates it.
fn scan_stale_references(
    candidates: &[String],
    ctx: &StaleReferenceCtx<'_>,
    cache: &ParseCache,
) -> Vec<(Vec<Diagnostic>, CacheInsert)> {
    candidates
        .par_iter()
        .filter_map(|rel_str| {
            if ctx.deleted.contains(rel_str.as_str())
                || ctx.file_list_set.contains(rel_str.as_str())
            {
                return None;
            }

            let (pairs, new_entry) = stale_reference_scan_input(rel_str, ctx, cache)?;
            let sorted = ctx.sorted_lines.get(rel_str.as_str());

            let mut diagnostics = Vec::new();
            for (pair, target, triggered) in
                active_targets(&pairs, sorted, rel_str, ctx.vcs, ctx.ignore_patterns)
            {
                if triggered {
                    continue;
                }
                let Some(file) = &target.file else { continue };
                let Some(target_str) = resolve_target_path(file, rel_str, ctx.vcs) else {
                    continue;
                };

                let dctx = DiagnosticCtx {
                    source_file: rel_str,
                    pair,
                    target,
                };

                // Deleted file reference.
                if ctx.deleted.contains(target_str.as_str()) {
                    diagnostics.push(target_error(
                        &dctx,
                        missing_target_message(&target_str, target.label.as_deref()),
                    ));
                    continue;
                }

                // Stale label reference.
                let Some(label_name) = &target.label else {
                    continue;
                };
                let Some(valid_labels) = ctx.label_sets.get(&target_str) else {
                    continue;
                };
                if valid_labels.contains(label_name) {
                    continue;
                }

                diagnostics.push(target_error(
                    &dctx,
                    format!("label {label_name} not found in {target_str}"),
                ));
            }
            Some((diagnostics, new_entry))
        })
        .collect()
}

fn stale_reference_scan_input(
    rel_str: &str,
    ctx: &StaleReferenceCtx<'_>,
    cache: &ParseCache,
) -> Option<(Vec<DirectivePair>, CacheInsert)> {
    if let Some(parsed) = cache.get(rel_str) {
        if parsed.directives.is_empty() {
            return None;
        }

        return Some((build_pairs(&parsed.directives), None));
    }

    let content = ctx.vcs.read_file(rel_str).ok()??;
    if crate::vcs::is_binary(&content) {
        return None;
    }

    let mentions_deleted = ctx.deleted.iter().any(|d| content.contains(*d));
    let mentions_label_file = ctx.label_sets.keys().any(|f| content.contains(f.as_str()));
    if !mentions_deleted && !mentions_label_file {
        return None;
    }

    let (directives, errors) = crate::parser::parse(&content, rel_str);
    let pairs = build_pairs(&directives);
    let new_entry = Some((rel_str.to_string(), ParsedFile { directives, errors }));
    Some((pairs, new_entry))
}

#[cfg(test)]
#[path = "check_test.rs"]
mod tests;
