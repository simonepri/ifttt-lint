// Core parser: extracts LINT.IfChange / LINT.ThenChange directives from source
// files using language definitions from `crate::languages`.
//
// Two-phase design:
//   1. Scanner — line-oriented state machine that extracts comment bodies
//   2. Directive parser — regex-based extraction of directives from comments

use std::num::NonZeroUsize;
use std::sync::OnceLock;

use regex::Regex;

use crate::languages::{
    self, BlockComment, FixedClose, Language, LongBracket, SkipClose, SkipPattern,
};

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub raw: String,
    pub file: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectiveError {
    pub line: NonZeroUsize,
    pub message: String,
}

/// Parse a file and return all directives and errors.
pub fn parse(content: &str, file_path: &str) -> (Vec<Directive>, Vec<DirectiveError>) {
    let lang = languages::detect(file_path);
    let tokens = scan(content, lang);
    parse_directives(&tokens)
}

#[derive(Debug, Clone)]
struct CommentToken {
    line: NonZeroUsize,
    body: String,
}

enum ScanState {
    Normal,
    InBlockComment {
        close: &'static str,
        nestable: bool,
        open: &'static str,
        depth: usize,
    },
    InSkipRegion(SkipClose),
}

fn scan(content: &str, lang: &Language) -> Vec<CommentToken> {
    let mut tokens = Vec::new();
    let mut state = ScanState::Normal;

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = NonZeroUsize::new(line_idx + 1).unwrap();
        state = scan_line(line, line_num, state, lang, &mut tokens);
    }

    tokens
}

fn scan_line(
    line: &str,
    line_num: NonZeroUsize,
    state: ScanState,
    lang: &Language,
    tokens: &mut Vec<CommentToken>,
) -> ScanState {
    match state {
        ScanState::Normal => scan_normal(line, line_num, lang, tokens),
        ScanState::InBlockComment {
            close,
            nestable,
            open,
            depth,
        } => scan_block_comment(line, close, nestable, open, depth),
        ScanState::InSkipRegion(skip) => scan_skip_region(line, skip),
    }
}

fn scan_normal(
    line: &str,
    line_num: NonZeroUsize,
    lang: &Language,
    tokens: &mut Vec<CommentToken>,
) -> ScanState {
    let trimmed = line.trim_start();

    for &prefix in lang.line_comments {
        let Some(body) = trimmed.strip_prefix(prefix) else {
            continue;
        };

        // Special: Lua "--" + "[=*[" -> LongBracket skip
        // Special: CMake "#" + "[=*[" -> LongBracket skip
        if let Some((skip, open_len)) = LongBracket::try_open(body) {
            let after_open = &body[open_len..];
            if skip.check_close(after_open) {
                return ScanState::Normal;
            }
            return ScanState::InSkipRegion(skip);
        }

        push_token(tokens, body.trim(), line_num);
        return ScanState::Normal;
    }

    for bc in lang.block_comments {
        let Some(after_open) = trimmed.strip_prefix(bc.open) else {
            continue;
        };
        if let Some(close_pos) = after_open.find(bc.close) {
            let body = &after_open[..close_pos];
            let stripped = strip_block_decoration(body);
            push_token(tokens, stripped.trim(), line_num);
            return ScanState::Normal;
        }
        return ScanState::InBlockComment {
            close: bc.close,
            nestable: bc.nestable,
            open: bc.open,
            depth: 0,
        };
    }

    scan_for_openers(line, lang)
}

fn scan_block_comment(
    line: &str,
    close: &'static str,
    nestable: bool,
    open: &'static str,
    mut depth: usize,
) -> ScanState {
    let mut pos = 0;
    let bytes = line.as_bytes();

    while pos < bytes.len() {
        if nestable && line[pos..].starts_with(open) {
            depth += 1;
            pos += open.len();
            continue;
        }
        if line[pos..].starts_with(close) {
            if depth == 0 {
                // Directives must appear alone on their comment line.
                // Anything after the block-comment close on the same line
                // is intentionally not scanned.
                return ScanState::Normal;
            }
            depth -= 1;
            pos += close.len();
            continue;
        }
        // Advance by the byte length of the current UTF-8 character
        pos += line[pos..].chars().next().map_or(1, |c| c.len_utf8());
    }

    ScanState::InBlockComment {
        close,
        nestable,
        open,
        depth,
    }
}

fn scan_skip_region(line: &str, skip: SkipClose) -> ScanState {
    match skip.advance(line) {
        None => ScanState::Normal,
        Some(next) => ScanState::InSkipRegion(next),
    }
}

fn scan_for_openers(line: &str, lang: &Language) -> ScanState {
    let bytes = line.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        let rest = &line[pos..];

        if let Some((skip_close, advance)) = try_skip_pattern_open(rest, lang) {
            if matches!(&skip_close, SkipClose::FencedCode(_)) && pos > 0 {
                pos += line[pos..].chars().next().map_or(1, |c| c.len_utf8());
                continue;
            }
            let after = &line[pos + advance..];
            if !skip_close.check_close(after) {
                if skip_close.spans_lines() {
                    return ScanState::InSkipRegion(skip_close);
                }
                // Single-line pattern (e.g., unclosed string literal): skip to EOL.
                pos = bytes.len();
                continue;
            }
            pos += advance + skip_close.find_close_offset(after).unwrap_or(after.len());
            continue;
        }

        if let Some((bc, advance)) = try_block_comment_open(rest, lang) {
            let after = &line[pos + advance..];
            if after.contains(bc.close) {
                pos += advance + after.find(bc.close).unwrap() + bc.close.len();
                continue;
            }
            return ScanState::InBlockComment {
                close: bc.close,
                nestable: bc.nestable,
                open: bc.open,
                depth: 0,
            };
        }

        if line_comment_starts_at(rest, lang) {
            return ScanState::Normal;
        }

        // Advance by the byte length of the current UTF-8 character
        pos += line[pos..].chars().next().map_or(1, |c| c.len_utf8());
    }

    ScanState::Normal
}

fn try_block_comment_open<'a>(s: &str, lang: &'a Language) -> Option<(&'a BlockComment, usize)> {
    for bc in lang.block_comments {
        if s.starts_with(bc.open) {
            return Some((bc, bc.open.len()));
        }
    }
    None
}

fn line_comment_starts_at(s: &str, lang: &Language) -> bool {
    lang.line_comments
        .iter()
        .any(|&prefix| s.starts_with(prefix))
}

fn try_skip_pattern_open(s: &str, lang: &Language) -> Option<(SkipClose, usize)> {
    let mut best: Option<(SkipClose, usize)> = None;
    for pat in lang.skip_patterns {
        let result = match pat {
            SkipPattern::Fixed { open, close } => s
                .starts_with(open)
                .then_some((FixedClose::into_skip_close(close.to_string()), open.len())),
            SkipPattern::Dynamic(opener) => opener(s),
        };
        if let Some((sc, len)) = result {
            if best.as_ref().is_some_and(|(_, best_len)| len <= *best_len) {
                continue;
            }
            best = Some((sc, len));
        }
    }
    best
}

fn push_token(tokens: &mut Vec<CommentToken>, body: &str, line: NonZeroUsize) {
    if !body.is_empty() {
        tokens.push(CommentToken {
            line,
            body: body.to_string(),
        });
    }
}

fn strip_block_decoration(body: &str) -> &str {
    let trimmed = body.trim();
    trimmed
        .strip_prefix('*')
        .map(|s| s.trim())
        .unwrap_or(trimmed)
}

fn if_change_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"LINT\.IfChange(?:\(([a-zA-Z][a-zA-Z0-9_.\-]*)\))?(?:\W|$)").unwrap()
    })
}

fn if_change_label_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"LINT\.IfChange\(([^)]+)\)").unwrap())
}

fn malformed_label_message(raw: &str) -> String {
    format!("malformed label \"{raw}\": must start with a letter and contain only [a-zA-Z0-9_.\\-]")
}

fn is_valid_label(label: &str) -> bool {
    let mut chars = label.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }

    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

fn then_change_empty_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"LINT\.ThenChange\(\)").unwrap())
}

fn then_change_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Greedy `.+` matches up to the *last* `)`, so paths with nested parens
    // (e.g. `//path(1).rs`) are captured whole. is_prose_mention() filters
    // false captures where the outer `)` belongs to surrounding prose.
    RE.get_or_init(|| Regex::new(r"LINT\.ThenChange\((.+)\)").unwrap())
}

fn unknown_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"LINT\.(\w+)").unwrap())
}

/// Check whether a comment body looks like a directive line.
fn is_directive_body(body: &str) -> bool {
    body.trim_start().starts_with("LINT.")
}

/// Returns true when the body mentions a LINT directive as prose rather than
/// using it as a real directive.  E.g. `LINT.IfChange(label) is used for…`
/// or `LINT.ThenChange() marks the end…`.
///
/// Heuristic: after the complete directive pattern (including balanced parens),
/// any remaining non-whitespace text means this is prose, not a directive.
fn is_prose_mention(body: &str) -> bool {
    let after_name = body
        .find("LINT.IfChange")
        .map(|i| i + "LINT.IfChange".len())
        .or_else(|| {
            body.find("LINT.ThenChange")
                .map(|i| i + "LINT.ThenChange".len())
        });
    let Some(after_name) = after_name else {
        return false;
    };
    let rest = &body[after_name..];
    if !rest.starts_with('(') {
        return !rest.trim().is_empty();
    }
    // Walk balanced parens — handles nested parens in paths like //path(1).rs.
    // Invariant: depth ≥ 1 throughout the loop because rest.starts_with('(')
    // (checked above), so the first character increments depth to 1 before any
    // ')' can decrement it.  depth -= 1 can therefore never underflow.
    let mut depth = 0usize;
    for (i, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return !rest[i + 1..].trim().is_empty();
                }
            }
            _ => {}
        }
    }
    // Unclosed paren — treat as a real (possibly malformed) directive, not prose.
    false
}

fn parse_directives(tokens: &[CommentToken]) -> (Vec<Directive>, Vec<DirectiveError>) {
    let mut directives = Vec::new();
    let mut errors = Vec::new();
    let mut skip_until = 0;

    for (i, token) in tokens.iter().enumerate() {
        if i < skip_until {
            continue;
        }
        if !is_directive_body(&token.body) {
            continue;
        }
        if is_prose_mention(&token.body) {
            continue;
        }

        let body = &token.body;
        let line = token.line;

        if let Some(caps) = if_change_re().captures(body) {
            let label = caps.get(1).map(|m| m.as_str().to_string());
            // The regex makes the label group optional, so it matches even when
            // parens contain an invalid label (e.g. `LINT.IfChange(123bad)`).
            // Detect that case by re-checking with the broader label regex.
            if label.is_none() {
                if let Some(bad) = if_change_label_re().captures(body) {
                    let raw = bad.get(1).unwrap().as_str();
                    errors.push(DirectiveError {
                        line,
                        message: malformed_label_message(raw),
                    });
                    continue;
                }
            }
            directives.push(Directive::IfChange { line, label });
            continue;
        }

        if body.contains("LINT.ThenChange") {
            if then_change_empty_re().is_match(body) {
                directives.push(Directive::ThenChange {
                    line,
                    targets: vec![],
                });
                continue;
            }

            match parse_then_change_targets(body) {
                Ok(Some(targets)) => {
                    directives.push(Directive::ThenChange { line, targets });
                    continue;
                }
                Err(message) => {
                    errors.push(DirectiveError { line, message });
                    continue;
                }
                Ok(None) => {}
            }

            let (joined, end) = collect_multiline(tokens, i);
            skip_until = end;

            match parse_then_change_targets(&joined) {
                Ok(Some(targets)) => {
                    directives.push(Directive::ThenChange { line, targets });
                    continue;
                }
                Err(message) => {
                    errors.push(DirectiveError { line, message });
                    continue;
                }
                Ok(None) => {}
            }

            errors.push(DirectiveError {
                line,
                message: format!("malformed directive: {}", body.trim()),
            });
            continue;
        }

        if let Some(caps) = unknown_re().captures(body) {
            let name = caps.get(1).unwrap().as_str();
            if name != "IfChange" && name != "ThenChange" {
                errors.push(DirectiveError {
                    line,
                    message: format!("unknown directive: LINT.{name}"),
                });
                continue;
            }
        }

        errors.push(DirectiveError {
            line,
            message: format!("malformed directive: {}", body.trim()),
        });
    }

    (directives, errors)
}

fn collect_multiline(tokens: &[CommentToken], start: usize) -> (String, usize) {
    let mut joined = tokens[start].body.clone();
    let mut end = start + 1;

    while end < tokens.len() && tokens[end].line.get() == tokens[end - 1].line.get() + 1 {
        joined.push(' ');
        joined.push_str(&tokens[end].body);
        end += 1;
        if then_change_re().is_match(&joined) {
            break;
        }
    }

    (joined, end)
}

fn parse_then_change_targets(body: &str) -> Result<Option<Vec<Target>>, String> {
    let Some(caps) = then_change_re().captures(body) else {
        return Ok(None);
    };
    let Some(target_str) = caps.get(1).map(|m| m.as_str()) else {
        return Ok(None);
    };
    let targets = parse_target_list(target_str)?;
    Ok((!targets.is_empty()).then_some(targets))
}

fn parse_target_list(s: &str) -> Result<Vec<Target>, String> {
    s.split(',')
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .map(parse_target)
        .collect()
}

fn parse_target(raw: &str) -> Result<Target, String> {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix(':') {
        if !is_valid_label(rest) {
            return Err(malformed_label_message(rest));
        }

        return Ok(Target {
            raw: trimmed.to_string(),
            file: None,
            label: Some(rest.to_string()),
        });
    }
    if let Some(colon_pos) = find_label_colon(trimmed) {
        let file_part = &trimmed[..colon_pos];
        let label_part = &trimmed[colon_pos + 1..];
        if !is_valid_label(label_part) {
            return Err(malformed_label_message(label_part));
        }

        return Ok(Target {
            raw: trimmed.to_string(),
            file: Some(file_part.to_string()),
            label: Some(label_part.to_string()),
        });
    }
    Ok(Target {
        raw: trimmed.to_string(),
        file: Some(trimmed.to_string()),
        label: None,
    })
}

/// Find the colon that separates file path from label in a target string.
/// Returns `None` if no valid label separator exists.
///
/// Splits on the last `/` first to isolate the basename, then searches
/// for a label colon only within the basename.  This naturally handles
/// colons in directory components (e.g. `host:8080/`) and Windows drive
/// letters (`C:\`, `C:/`).
fn find_label_colon(s: &str) -> Option<usize> {
    let basename_start = s.rfind('/').map(|p| p + 1).unwrap_or(0);
    let basename = &s[basename_start..];
    let local_pos = basename.rfind(':')?;
    let colon_pos = basename_start + local_pos;
    let label_part = &s[colon_pos + 1..];
    if label_part.is_empty() || !label_part.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(colon_pos)
}

#[cfg(test)]
#[path = "parser_test.rs"]
mod tests;
