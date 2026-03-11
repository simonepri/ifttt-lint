use std::num::NonZeroUsize;
use std::path::Path;

use pest::Parser;
use pest_derive::Parser;

// ─── Public types ───

#[derive(Parser)]
#[grammar = "grammar.pest"]
pub struct DirectiveParser;

/// A parsed LINT directive with its line number (1-based).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    IfChange {
        line: NonZeroUsize,
        label: Option<String>,
    },
    ThenChange {
        line: NonZeroUsize,
        targets: Vec<Target>,
    },
    LabelStart {
        line: NonZeroUsize,
        name: String,
    },
    LabelEnd {
        line: NonZeroUsize,
    },
}

/// A ThenChange target: file path and optional label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub raw: String,
    pub file: Option<String>,
    pub label: Option<String>,
}

/// Error from directive parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveError {
    pub line: NonZeroUsize,
    pub message: String,
}

// ─── Public API ───

/// Parse all LINT directives from file content.
pub fn parse_directives(content: &str, file_path: &Path) -> (Vec<Directive>, Vec<DirectiveError>) {
    let mut directives = Vec::new();
    let mut errors = Vec::new();

    let ext = file_extension(file_path);
    let allowed_rules = comment_rules_for_extension(&ext);
    let all_lines: Vec<&str> = content.lines().collect();

    let mut skip_until = 0;

    for (line_idx, line) in all_lines.iter().enumerate() {
        if line_idx < skip_until {
            continue;
        }

        if !line.contains("LINT.") {
            continue;
        }

        // Use the grammar to extract comment body (strips prefix/suffix)
        let Some(comment_text) = extract_comment_body(line, allowed_rules) else {
            continue;
        };

        // Handle multi-line ThenChange: enter multi-line mode only when the
        // comment body is an actual (but incomplete) LINT.ThenChange directive.
        // Requiring the body to start with "LINT.ThenChange" prevents prose
        // comments like "// See LINT.ThenChange docs" from incorrectly consuming
        // the following lines into a multi-line collection pass.
        let directive_text = if comment_text.trim_start().starts_with("LINT.ThenChange")
            && DirectiveParser::parse(Rule::directive, &comment_text).is_err()
        {
            let (text, end) = collect_multiline_text(&all_lines, line_idx, allowed_rules);
            skip_until = end;
            text
        } else {
            comment_text
        };

        let line_num = NonZeroUsize::new(line_idx + 1).unwrap();

        match DirectiveParser::parse(Rule::directive, &directive_text) {
            Ok(pairs) => {
                for pair in pairs {
                    if let Some(d) = extract_directive(pair, line_num, &mut errors) {
                        directives.push(d);
                    }
                }
            }
            Err(_) => {
                if directive_text.trim_start().starts_with("LINT.") {
                    errors.push(DirectiveError {
                        line: line_num,
                        message: format!("malformed directive: {}", directive_text.trim()),
                    });
                }
            }
        }
    }

    (directives, errors)
}

// ─── Private helpers ───

/// Get the file extension (lowercase, without dot).
fn file_extension(path: &Path) -> String {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();

    if filename == "makefile" || filename == "gnumakefile" {
        return "makefile".to_string();
    }
    if filename == "dockerfile" {
        return "dockerfile".to_string();
    }

    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// Grammar rules for comment styles a file extension supports.
fn comment_rules_for_extension(ext: &str) -> &'static [Rule] {
    match ext {
        "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts" | "java" | "c" | "h" | "cc"
        | "cpp" | "cxx" | "hpp" | "hxx" | "cs" | "go" | "rs" | "swift" | "kt" | "kts" | "scala"
        | "dart" | "zig" | "v" | "groovy" | "gradle" => &[Rule::line_slash, Rule::line_block],

        "py" | "pyi" | "rb" | "sh" | "bash" | "zsh" | "fish" | "pl" | "pm" | "r" | "yaml"
        | "yml" | "toml" | "mk" | "dockerfile" | "bzl" | "star" | "hcl" | "tf" | "nix" | "conf"
        | "ini" | "env" => &[Rule::line_hash],

        "html" | "htm" | "xml" | "svg" | "md" | "mdx" | "jsp" | "erb" => &[Rule::line_html],

        "sql" | "lua" | "hs" | "ada" | "vhd" | "vhdl" | "pgsql" => &[Rule::line_dash],

        "lisp" | "cl" | "clj" | "cljs" | "cljc" | "el" | "scm" | "rkt" => &[Rule::line_semi],

        "tex" | "latex" | "m" | "erl" | "hrl" => &[Rule::line_pct],

        "f" | "f90" | "f95" | "f03" | "for" => &[Rule::line_bang],

        "vue" | "svelte" => &[Rule::line_slash, Rule::line_block, Rule::line_html],
        "php" => &[Rule::line_slash, Rule::line_block, Rule::line_hash],

        "makefile" => &[Rule::line_hash],

        _ => &[Rule::line_slash, Rule::line_block, Rule::line_hash],
    }
}

/// Extract the comment body text from a line using the grammar's line_* rules.
/// Returns the text after the comment prefix (and before any block suffix).
/// For block comments (`/* ... */`) and HTML comments (`<!-- ... -->`), the
/// closing syntax is stripped so it doesn't leak into directive parsing or
/// multi-line joining.
fn extract_comment_body(line: &str, allowed_rules: &[Rule]) -> Option<String> {
    for &rule in allowed_rules {
        if let Ok(pairs) = DirectiveParser::parse(rule, line) {
            for pair in pairs {
                for inner in pair.into_inner() {
                    if inner.as_rule() == Rule::comment_body {
                        let body = strip_comment_suffix(inner.as_str(), rule);
                        return Some(body.trim().to_string());
                    }
                }
            }
        }
    }
    None
}

/// Strip closing comment syntax (`*/` for block comments, `-->` for HTML)
/// from the raw body extracted by the grammar. This prevents the suffix
/// from leaking into multi-line joining or directive parsing.
fn strip_comment_suffix(body: &str, rule: Rule) -> &str {
    match rule {
        Rule::line_block => body
            .split_once("*/")
            .map(|(before, _)| before)
            .unwrap_or(body),
        Rule::line_html => body
            .split_once("-->")
            .map(|(before, _)| before)
            .unwrap_or(body),
        _ => body,
    }
}

/// Collect multi-line directive by joining continuation comment lines.
/// Returns the joined text and the index of the first line after the directive.
/// Uses the pest grammar to determine when the directive is complete:
/// after each joined line, attempts a parse — stops as soon as it succeeds.
fn collect_multiline_text(
    lines: &[&str],
    start_idx: usize,
    allowed_rules: &[Rule],
) -> (String, usize) {
    let first = match extract_comment_body(lines[start_idx], allowed_rules) {
        Some(t) => t,
        None => return (String::new(), start_idx + 1),
    };

    let mut joined = first;
    let mut end = start_idx + 1;

    if DirectiveParser::parse(Rule::directive, &joined).is_ok() {
        return (joined, end);
    }

    for line in &lines[start_idx + 1..] {
        if let Some(text) = extract_comment_body(line, allowed_rules) {
            joined.push(' ');
            joined.push_str(&text);
            end += 1;

            if DirectiveParser::parse(Rule::directive, &joined).is_ok() {
                return (joined, end);
            }
        } else {
            break;
        }
    }

    (joined, end)
}

/// Extract the label string from a directive pair containing a `quoted_label` child.
fn extract_quoted_label(pair: pest::iterators::Pair<'_, Rule>) -> Option<String> {
    pair.into_inner()
        .find(|p| p.as_rule() == Rule::quoted_label)
        .and_then(|p| p.into_inner().find(|p| p.as_rule() == Rule::label))
        .map(|p| p.as_str().to_string())
}

/// Extract a Directive from a pest parse pair.
fn extract_directive(
    pair: pest::iterators::Pair<'_, Rule>,
    line_num: NonZeroUsize,
    errors: &mut Vec<DirectiveError>,
) -> Option<Directive> {
    let body = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::directive_body)?;

    for directive_pair in body.into_inner() {
        match directive_pair.as_rule() {
            Rule::if_change => {
                let label = extract_quoted_label(directive_pair);
                return Some(Directive::IfChange {
                    line: line_num,
                    label,
                });
            }
            Rule::then_change => {
                let targets = extract_then_change_targets(directive_pair, errors, line_num);
                if !targets.is_empty() {
                    return Some(Directive::ThenChange {
                        line: line_num,
                        targets,
                    });
                }
            }
            Rule::label_start => {
                let name = extract_quoted_label(directive_pair).unwrap_or_default();
                return Some(Directive::LabelStart {
                    line: line_num,
                    name,
                });
            }
            Rule::label_end => {
                return Some(Directive::LabelEnd { line: line_num });
            }
            Rule::unknown_directive => {
                let name = directive_pair
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::directive_name)
                    .map(|p| p.as_str().to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                errors.push(DirectiveError {
                    line: line_num,
                    message: format!("unknown directive: LINT.{name}"),
                });
            }
            _ => {}
        }
    }
    None
}

/// Extract targets from a ThenChange directive.
fn extract_then_change_targets(
    pair: pest::iterators::Pair<'_, Rule>,
    errors: &mut Vec<DirectiveError>,
    line_num: NonZeroUsize,
) -> Vec<Target> {
    let mut targets = Vec::new();

    let quoted_targets = pair
        .into_inner()
        .filter(|p| matches!(p.as_rule(), Rule::target_single | Rule::target_array))
        .flat_map(|p| p.into_inner())
        .filter(|p| p.as_rule() == Rule::quoted_target);

    for qt in quoted_targets {
        match extract_quoted_target(qt) {
            Ok(t) => targets.push(t),
            Err(e) => errors.push(DirectiveError {
                line: line_num,
                message: e,
            }),
        }
    }

    targets
}

/// Walk the children of a `target_inner` parse node to extract the file path and label.
fn parse_target_inner(inner: pest::iterators::Pair<'_, Rule>) -> (Option<String>, Option<String>) {
    let mut file = None;
    let mut label = None;

    for child in inner.into_inner() {
        match child.as_rule() {
            Rule::target_label_ref => {
                label = child
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::label)
                    .map(|p| p.as_str().to_string());
            }
            Rule::target_file_label => {
                for part in child.into_inner() {
                    match part.as_rule() {
                        Rule::target_file_path => file = Some(part.as_str().to_string()),
                        Rule::label => label = Some(part.as_str().to_string()),
                        _ => {}
                    }
                }
            }
            Rule::target_file_only => {
                file = Some(child.as_str().to_string());
            }
            _ => {}
        }
    }

    (file, label)
}

/// Extract a Target from a parsed `quoted_target` pair.
/// The grammar's PUSH/POP/PEEK mechanism handles quote matching and target structure;
/// Rust only walks the parse tree and validates path security.
fn extract_quoted_target(pair: pest::iterators::Pair<'_, Rule>) -> Result<Target, String> {
    let mut file = None;
    let mut label = None;
    let mut raw = String::new();

    if let Some(inner) = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::target_inner)
    {
        raw = inner.as_str().to_string();
        (file, label) = parse_target_inner(inner);
    }

    // Security: reject path traversal and absolute paths after //
    if let Some(ref f) = file {
        validate_target_path(f)?;
    }

    Ok(Target { raw, file, label })
}

/// Validate that a target path doesn't escape the project root.
///
/// Rejects:
/// - `..` path traversal components (e.g. `//foo/../etc/passwd`)
/// - Absolute paths after the `//` prefix (e.g. `///etc/passwd`)
///
/// Normal filenames containing dots (e.g. `file..txt`) are permitted.
fn validate_target_path(file_path: &str) -> Result<(), String> {
    let rel = file_path.strip_prefix("//").unwrap_or(file_path);
    let rel_path = Path::new(rel);
    for component in rel_path.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err(format!(
                    "path traversal ('..') is not allowed in target: '{file_path}'"
                ));
            }
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return Err(format!(
                    "target path must be relative after '//': got '{file_path}'"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}
