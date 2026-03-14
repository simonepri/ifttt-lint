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
            if open_len <= body.len() {
                let after_open = &body[open_len..];
                if skip.check_close(after_open) {
                    return ScanState::Normal;
                }
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
                return ScanState::InSkipRegion(skip_close);
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
    debug_assert!(quote.is_ascii(), "quote must be ASCII for UTF-8 safety");
    let mut pos = start + 1;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos = (pos + 2).min(bytes.len()); // skip escaped char
            continue;
        }
        if bytes[pos] == quote {
            return Some(pos + 1);
        }
        pos += 1;
    }
    None
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

/// Returns true when the body mentions a LINT directive as prose rather than
/// using it as a real directive.  E.g. `LINT.IfChange(label) is used for…`
/// or `LINT.ThenChange() marks the end…`.
///
/// Heuristic: after the complete directive pattern (including optional parens),
/// any remaining non-whitespace text means this is prose, not a directive.
fn is_prose_mention(body: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"LINT\.(?:IfChange|ThenChange)(?:\([^)]*\))?").unwrap());
    if let Some(m) = re.find(body) {
        let rest = &body[m.end()..];
        // If the next char is '(' the optional group didn't match because the
        // parens are unclosed — that's a real (possibly malformed) directive.
        !rest.starts_with('(') && !rest.trim().is_empty()
    } else {
        false
    }
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
                        message: format!("malformed label \"{raw}\": must start with a letter and contain only [a-zA-Z0-9_.\\-]"),
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

            if let Some(targets) = parse_then_change_targets(body) {
                directives.push(Directive::ThenChange { line, targets });
                continue;
            }

            let (joined, end) = collect_multiline(tokens, i);
            skip_until = end;

            if let Some(targets) = parse_then_change_targets(&joined) {
                directives.push(Directive::ThenChange { line, targets });
                continue;
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

fn parse_then_change_targets(body: &str) -> Option<Vec<Target>> {
    let caps = then_change_re().captures(body)?;
    let target_str = caps.get(1)?.as_str();
    let targets = parse_target_list(target_str);
    (!targets.is_empty()).then_some(targets)
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
    if let Some(colon_pos) = find_label_colon(trimmed) {
        let file_part = &trimmed[..colon_pos];
        let label_part = &trimmed[colon_pos + 1..];
        return Target {
            raw: trimmed.to_string(),
            file: Some(file_part.to_string()),
            label: Some(label_part.to_string()),
        };
    }
    Target {
        raw: trimmed.to_string(),
        file: Some(trimmed.to_string()),
        label: None,
    }
}

/// Find the colon that separates file path from label in a target string.
/// Returns `None` if no valid label separator exists.
///
/// Skips colons that look like Windows drive letters (`X:\…` or `X:/…` at
/// position 0 or right after a `//` prefix).
fn find_label_colon(s: &str) -> Option<usize> {
    let colon_pos = s.rfind(':')?;
    let label_part = &s[colon_pos + 1..];
    if label_part.is_empty() || !label_part.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    // Skip Windows drive-letter colons: a single ASCII letter immediately
    // before the colon, at the start of the path (possibly after `//`).
    let before = &s[..colon_pos];
    let stripped = before.strip_prefix("//").unwrap_or(before);
    if stripped.len() == 1 && stripped.as_bytes()[0].is_ascii_alphabetic() {
        let after_colon = s.as_bytes().get(colon_pos + 1);
        if after_colon == Some(&b'\\') || after_colon == Some(&b'/') {
            return None;
        }
    }
    Some(colon_pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_target_file_and_label() {
        let t = parse_target("//src/lib.rs:my_label");
        assert_eq!(t.file.as_deref(), Some("//src/lib.rs"));
        assert_eq!(t.label.as_deref(), Some("my_label"));
    }

    #[test]
    fn parse_target_file_only() {
        let t = parse_target("//src/lib.rs");
        assert_eq!(t.file.as_deref(), Some("//src/lib.rs"));
        assert_eq!(t.label, None);
    }

    #[test]
    fn parse_target_label_only() {
        let t = parse_target(":my_label");
        assert_eq!(t.file, None);
        assert_eq!(t.label.as_deref(), Some("my_label"));
    }

    #[test]
    fn parse_target_windows_drive_letter_no_label() {
        // `C:\foo\bar` should NOT treat `C:` as file + label separator.
        let t = parse_target("C:\\foo\\bar");
        assert_eq!(t.file.as_deref(), Some("C:\\foo\\bar"));
        assert_eq!(t.label, None);
    }

    #[test]
    fn parse_target_windows_drive_letter_with_prefix() {
        // `//C:\foo` should NOT split on the drive-letter colon.
        let t = parse_target("//C:\\foo");
        assert_eq!(t.file.as_deref(), Some("//C:\\foo"));
        assert_eq!(t.label, None);
    }

    #[test]
    fn parse_target_windows_drive_letter_forward_slash() {
        let t = parse_target("C:/foo/bar");
        assert_eq!(t.file.as_deref(), Some("C:/foo/bar"));
        assert_eq!(t.label, None);
    }

    #[test]
    fn parse_target_drive_letter_with_label() {
        // `C:\foo:label` — rightmost colon is the label separator.
        let t = parse_target("C:\\foo:label");
        assert_eq!(t.file.as_deref(), Some("C:\\foo"));
        assert_eq!(t.label.as_deref(), Some("label"));
    }

    #[test]
    fn parse_target_colon_in_numeric_suffix_not_label() {
        // A colon followed by a digit is not a label separator.
        let t = parse_target("//file.txt:42");
        assert_eq!(t.file.as_deref(), Some("//file.txt:42"));
        assert_eq!(t.label, None);
    }
}
