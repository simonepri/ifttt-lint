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
    self, BlockComment, FixedClose, Language, LongBracket, SkipClose, SkipPattern, UnescapedChar,
};

// ─── Public types ───

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

// ─── Phase 1: Scanner ───

#[derive(Debug, Clone)]
struct CommentToken {
    line: NonZeroUsize,
    body: String,
}

enum ScanState {
    Normal,
    InBlockComment {
        close: String,
        nestable: bool,
        open: String,
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
        } => scan_block_comment(line, &close, nestable, &open, depth),
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

    // 1. Try line comment prefixes (longest first for correct matching)
    for &prefix in lang.line_comments {
        if let Some(body) = trimmed.strip_prefix(prefix) {
            // Special: Lua "--" + "[=*[" -> LongBracket skip
            // Special: CMake "#" + "[=*[" -> LongBracket skip
            if let Some((skip, open_len)) = LongBracket::try_open(body) {
                if open_len <= body.len() {
                    let after_open = &body[open_len..];
                    if skip.check_close(after_open) {
                        // Single-line long bracket comment — skip (not a directive line)
                        return ScanState::Normal;
                    }
                }
                return ScanState::InSkipRegion(skip);
            }

            push_token(tokens, body.trim(), line_num);
            return ScanState::Normal;
        }
    }

    // 2. Check block comments at line start (all languages, not just block-only).
    //    Single-line block comments yield tokens; multi-line enter InBlockComment.
    for bc in lang.block_comments {
        if let Some(after_open) = trimmed.strip_prefix(bc.open) {
            if let Some(close_pos) = after_open.find(bc.close) {
                let body = &after_open[..close_pos];
                let stripped = strip_block_decoration(body);
                push_token(tokens, stripped.trim(), line_num);
                return ScanState::Normal;
            }
            // Block comment opens but doesn't close on this line
            return ScanState::InBlockComment {
                close: bc.close.to_string(),
                nestable: bc.nestable,
                open: bc.open.to_string(),
                depth: 0,
            };
        }
    }

    // 3. Scan for multi-line openers on non-comment lines
    scan_for_openers(line, lang)
}

fn scan_block_comment(
    line: &str,
    close: &str,
    nestable: bool,
    open: &str,
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
                // Resume scanning the rest of the line would be needed for
                // correctness, but directives only appear at line start so
                // returning Normal is sufficient.
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
        close: close.to_string(),
        nestable,
        open: open.to_string(),
        depth,
    }
}

fn scan_skip_region(line: &str, skip: SkipClose) -> ScanState {
    match skip.advance(line) {
        None => ScanState::Normal,
        Some(next) => ScanState::InSkipRegion(next),
    }
}

// ─── Opener scanning ───

fn scan_for_openers(line: &str, lang: &Language) -> ScanState {
    let bytes = line.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        let rest = &line[pos..];

        // Try skip pattern openers (longer/more-specific first)
        if let Some((skip_close, advance)) = try_skip_pattern_open(rest, lang) {
            // FencedCode is a line-level construct; reject mid-line matches.
            if matches!(&skip_close, SkipClose::FencedCode(_)) && pos > 0 {
                pos += line[pos..].chars().next().map_or(1, |c| c.len_utf8());
                continue;
            }
            // Check if closer is on the same line
            let after = &line[pos + advance..];
            if !skip_close.check_close(after) {
                return ScanState::InSkipRegion(skip_close);
            }
            // Closer found on same line — skip past it
            pos += advance + skip_close.find_close_offset(after).unwrap_or(after.len());
            continue;
        }

        // Try block comment openers
        if let Some((bc, advance)) = try_block_comment_open(rest, lang) {
            let after = &line[pos + advance..];
            if after.contains(bc.close) {
                // Single-line block comment — skip past closer
                pos += advance + after.find(bc.close).unwrap() + bc.close.len();
                continue;
            }
            return ScanState::InBlockComment {
                close: bc.close.to_string(),
                nestable: bc.nestable,
                open: bc.open.to_string(),
                depth: 0,
            };
        }

        // Try line comment prefix -> rest of line is comment, no opener possible
        if line_comment_starts_at(rest, lang) {
            return ScanState::Normal;
        }

        // Skip string literals
        if bytes[pos] == b'"' {
            match skip_single_line_string(bytes, pos, b'"') {
                Some(end) => {
                    pos = end;
                    continue;
                }
                None => return ScanState::InSkipRegion(UnescapedChar::into_skip_close(b'"')),
            }
        }
        if bytes[pos] == b'\'' {
            match skip_single_line_string(bytes, pos, b'\'') {
                Some(end) => pos = end,
                None => pos = bytes.len(), // unclosed single-quote, skip rest
            }
            continue;
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

/// Skip a string literal on a single line. Returns `Some(end)` if the closing
/// quote is found (end = position after the quote), `None` if unclosed.
fn skip_single_line_string(bytes: &[u8], start: usize, quote: u8) -> Option<usize> {
    let mut pos = start + 1;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos += 2; // skip escaped char
            continue;
        }
        if bytes[pos] == quote {
            return Some(pos + 1);
        }
        pos += 1;
    }
    None
}

// ─── Skip pattern open detection ───

fn try_skip_pattern_open(s: &str, lang: &Language) -> Option<(SkipClose, usize)> {
    let mut best: Option<(SkipClose, usize)> = None;
    for pat in lang.skip_patterns {
        let result = match pat {
            SkipPattern::Fixed { open, close } => {
                if s.starts_with(open) {
                    Some((FixedClose::into_skip_close(close.to_string()), open.len()))
                } else {
                    None
                }
            }
            SkipPattern::Dynamic(opener) => opener(s),
        };
        if let Some((sc, len)) = result {
            if best.is_none() || len > best.as_ref().unwrap().1 {
                best = Some((sc, len));
            }
        }
    }
    best
}

// ─── Token helpers ───

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

// ─── Phase 2: Directive parsing ───

fn if_change_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"LINT\.IfChange(?:\(([a-zA-Z][a-zA-Z0-9_.\-]*)\))?(?:\W|$)").unwrap()
    })
}

fn then_change_empty_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"LINT\.ThenChange\(\)").unwrap())
}

fn then_change_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"LINT\.ThenChange\(([^)]+)\)").unwrap())
}

fn unknown_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"LINT\.(\w+)").unwrap())
}

/// Check whether a comment body looks like a directive line.
fn is_directive_body(body: &str) -> bool {
    body.trim_start().starts_with("LINT.")
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

        let body = &token.body;
        let line = token.line;

        // Try IfChange
        if let Some(caps) = if_change_re().captures(body) {
            let label = caps.get(1).map(|m| m.as_str().to_string());
            directives.push(Directive::IfChange { line, label });
            continue;
        }

        // Try ThenChange
        if body.contains("LINT.ThenChange") {
            // Empty ThenChange — valid closure with no targets.
            if then_change_empty_re().is_match(body) {
                directives.push(Directive::ThenChange {
                    line,
                    targets: vec![],
                });
                continue;
            }

            if let Some(caps) = then_change_re().captures(body) {
                let target_str = caps.get(1).unwrap().as_str();
                let targets = parse_target_list(target_str);
                if !targets.is_empty() {
                    directives.push(Directive::ThenChange { line, targets });
                }
                continue;
            }

            // Multi-line ThenChange: accumulate consecutive tokens
            let (joined, end) = collect_multiline(tokens, i);
            skip_until = end;

            if let Some(caps) = then_change_re().captures(&joined) {
                let target_str = caps.get(1).unwrap().as_str();
                let targets = parse_target_list(target_str);
                if !targets.is_empty() {
                    directives.push(Directive::ThenChange { line, targets });
                }
                continue;
            }

            // Still didn't parse — report as malformed
            errors.push(DirectiveError {
                line,
                message: format!("malformed directive: {}", body.trim()),
            });
            continue;
        }

        // Try unknown directive
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

        // Starts with LINT. but nothing matched
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

fn parse_target_list(s: &str) -> Vec<Target> {
    s.split(',')
        .map(|entry| entry.trim())
        .filter(|entry| !entry.is_empty())
        .map(parse_target)
        .collect()
}

fn parse_target(raw: &str) -> Target {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix(':') {
        let label = rest.to_string();
        return Target {
            raw: trimmed.to_string(),
            file: None,
            label: Some(label),
        };
    }
    // Split on last ':'
    if let Some(colon_pos) = trimmed.rfind(':') {
        let file_part = &trimmed[..colon_pos];
        let label_part = &trimmed[colon_pos + 1..];
        if !label_part.is_empty() && label_part.starts_with(|c: char| c.is_ascii_alphabetic()) {
            return Target {
                raw: trimmed.to_string(),
                file: Some(file_part.to_string()),
                label: Some(label_part.to_string()),
            };
        }
    }
    Target {
        raw: trimmed.to_string(),
        file: Some(trimmed.to_string()),
        label: None,
    }
}
