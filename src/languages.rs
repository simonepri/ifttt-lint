// Language definitions for the ifttt-lint parser.
//
// Each Language describes comment syntax and skip patterns (strings, heredocs,
// raw literals) that the scanner uses to avoid false-positive directive matches
// inside non-comment tokens.
//
// Adding a new language:
//   1. Add a Language to LANGUAGES (matched top-to-bottom; first regex wins).
//   2. Pick skip patterns: STRING_SKIP, SkipPattern::Fixed { open, close }, or
//      SkipPattern::Dynamic(Struct::try_open). Reuse shared constants when
//      possible (e.g. TRIPLE_QUOTE_SKIP, HEREDOC_SKIP).
//   3. For new skip region types: create a struct with try_open/check_close/
//      find_close_offset and add it to skip_close_enum!.
//   4. Add a test in check_test.rs.

use std::sync::OnceLock;

use regex::{Regex, RegexSet};

#[derive(Debug)]
pub struct Language {
    #[allow(dead_code)]
    pub name: &'static str,
    /// Matched against the full filename, not just the extension.
    pub extensions: &'static str,
    pub line_comments: &'static [&'static str],
    pub block_comments: &'static [BlockComment],
    pub skip_patterns: &'static [SkipPattern],
}

#[derive(Debug)]
pub struct BlockComment {
    pub open: &'static str,
    pub close: &'static str,
    pub nestable: bool,
}

#[derive(Debug)]
pub enum SkipPattern {
    Fixed {
        open: &'static str,
        close: &'static str,
    },
    Dynamic(fn(&str) -> Option<(SkipClose, usize)>),
}

pub fn detect(path: &str) -> &'static Language {
    static SET: OnceLock<RegexSet> = OnceLock::new();
    let set = SET.get_or_init(|| RegexSet::new(LANGUAGES.iter().map(|l| l.extensions)).unwrap());
    // Match against the filename only so that directory components like
    // `src/build/` don't accidentally match patterns like `BUILD`.
    let filename = path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path);
    let matches: Vec<usize> = set.matches(filename).into_iter().collect();
    matches.first().map(|&i| &LANGUAGES[i]).unwrap_or(&FALLBACK)
}

pub static LANGUAGES: &[Language] = &[
    // ── // line + /* */ block ──
    Language {
        name: "C / C++",
        extensions: r"\.(c|h|cpp|cc|cxx|hpp|hxx|hh)$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: CPP_RAW_SKIP,
    },
    Language {
        name: "C#",
        extensions: r"\.cs$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: CS_STRING_SKIP,
    },
    Language {
        name: "Dart",
        extensions: r"\.dart$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: TRIPLE_QUOTE_SKIP,
    },
    Language {
        name: "Go",
        extensions: r"\.go$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: BACKTICK_FIXED_SKIP,
    },
    Language {
        name: "Groovy",
        extensions: r"\.(groovy|gradle)$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: GROOVY_STRING_SKIP,
    },
    Language {
        name: "Java",
        extensions: r"\.java$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: TRIPLE_DOUBLE_SKIP,
    },
    Language {
        name: "JavaScript",
        extensions: r"\.(jsx?|mjs|cjs)$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: BACKTICK_SKIP,
    },
    Language {
        name: "Kotlin",
        extensions: r"\.kts?$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: TRIPLE_DOUBLE_SKIP,
    },
    // `.m` is shared by Objective-C (`//`) and MATLAB (`%`).  We accept
    // both comment prefixes so directives work in either language — a false
    // positive would require `% LINT.IfChange` in Obj-C or vice-versa.
    Language {
        name: "Objective-C / MATLAB",
        extensions: r"\.mm?$",
        line_comments: SLASH_PCT_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "PHP",
        extensions: r"\.(php|phtml)$",
        line_comments: SLASH_HASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: PHP_HEREDOC_SKIP,
    },
    Language {
        name: "Protobuf",
        extensions: r"\.proto$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "Rust",
        extensions: r"\.rs$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK_NESTED,
        skip_patterns: RUST_RAW_SKIP,
    },
    Language {
        name: "Scala",
        extensions: r"\.(scala|sc)$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: TRIPLE_DOUBLE_SKIP,
    },
    Language {
        name: "SCSS",
        extensions: r"\.scss$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "Swift",
        extensions: r"\.swift$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: SWIFT_STRING_SKIP,
    },
    Language {
        name: "TypeScript",
        extensions: r"\.(tsx?|mts|cts)$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: BACKTICK_SKIP,
    },
    // ── # line ──
    Language {
        name: "CMake",
        extensions: r"(\.cmake$|(^|/)CMakeLists\.txt$)",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: LONG_BRACKET_SKIP,
    },
    Language {
        name: "Dockerfile",
        extensions: r"((^|/)Dockerfile(\.\w+)?$|\.dockerfile$)",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "Elixir",
        extensions: r"\.exs?$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: TRIPLE_QUOTE_SKIP,
    },
    Language {
        name: "GN",
        extensions: r"\.gni?$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "GraphQL",
        extensions: r"\.(graphql|gql)$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: TRIPLE_DOUBLE_SKIP,
    },
    Language {
        name: "Makefile",
        extensions: r"(\.(mk|mak)$|(^|/)(GNU)?[Mm]akefile$)",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "Nix",
        extensions: r"\.nix$",
        line_comments: HASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: NIX_INDENT_SKIP,
    },
    Language {
        name: "Perl",
        extensions: r"\.(pl|pm|t)$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: HEREDOC_PERCENT_SKIP,
    },
    Language {
        name: "PowerShell",
        extensions: r"\.(ps1|psm1|psd1)$",
        line_comments: HASH_LINE,
        block_comments: PS_BLOCK,
        skip_patterns: PS_HERESTRING_SKIP,
    },
    Language {
        name: "Python",
        extensions: r"\.(py|pyi|pyw)$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: TRIPLE_QUOTE_SKIP,
    },
    Language {
        name: "R",
        extensions: r"\.[rR]$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: R_RAW_SKIP,
    },
    Language {
        name: "Ruby",
        extensions: r"(\.(rb|rake|gemspec)$|(^|/)(Rakefile|Gemfile)$)",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: HEREDOC_PERCENT_SKIP,
    },
    Language {
        name: "Shell",
        extensions: r"\.(sh|bash|zsh|ksh)$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: HEREDOC_SKIP,
    },
    Language {
        name: "Starlark / Bazel",
        extensions: r"(\.bzl$|(^|/)(BUILD(\.bazel)?|WORKSPACE)$)",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: TRIPLE_QUOTE_SKIP,
    },
    Language {
        name: "Terraform / HCL",
        extensions: r"\.(tf|hcl|tfvars)$",
        line_comments: HASH_SLASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: HEREDOC_SKIP,
    },
    Language {
        name: "TOML",
        extensions: r"\.toml$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: TRIPLE_QUOTE_SKIP,
    },
    Language {
        name: "YAML",
        extensions: r"\.ya?ml$",
        line_comments: HASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    // ── ; line ──
    Language {
        name: "Lisp / Clojure",
        extensions: r"\.(clj[sc]?|edn|lisp|cl|el|scm|rkt)$",
        line_comments: SEMI_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    // ── -- line ──
    Language {
        name: "Haskell",
        extensions: r"\.hs$",
        line_comments: DASH_LINE,
        block_comments: BRACE_DASH_BLOCK_NESTED,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "Lua",
        extensions: r"\.lua$",
        line_comments: DASH_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: LONG_BRACKET_SKIP,
    },
    Language {
        name: "SQL",
        extensions: r"\.sql$",
        line_comments: DASH_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: DOLLAR_QUOTE_SKIP,
    },
    // ── % line ──
    Language {
        name: "LaTeX",
        extensions: r"\.(tex|sty|cls)$",
        line_comments: PCT_LINE,
        block_comments: NO_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    // ── Block-only ──
    Language {
        name: "CSS",
        extensions: r"\.css$",
        line_comments: NO_LINE,
        block_comments: SLASH_BLOCK,
        skip_patterns: STRING_SKIP,
    },
    Language {
        name: "HTML",
        extensions: r"\.(html?|xhtml)$",
        line_comments: NO_LINE,
        block_comments: HTML_BLOCK,
        skip_patterns: HTML_BLOCK_SKIP,
    },
    Language {
        name: "Markdown",
        extensions: r"\.(mdx?|markdown)$",
        line_comments: NO_LINE,
        block_comments: HTML_BLOCK,
        skip_patterns: FENCED_CODE_SKIP,
    },
    Language {
        name: "XML",
        extensions: r"\.(xml|xsd|xslt?|svg)$",
        line_comments: NO_LINE,
        block_comments: HTML_BLOCK,
        skip_patterns: HTML_BLOCK_SKIP,
    },
    // ── Multi-syntax (line + multiple block) ──
    Language {
        name: "Vue / Svelte",
        extensions: r"\.(vue|svelte)$",
        line_comments: SLASH_LINE,
        block_comments: SLASH_HTML_BLOCK,
        skip_patterns: BACKTICK_SKIP,
    },
    // ── {{/* … */}} block ──
    //
    // Go templates (including Helm `_helpers.tpl`) only have block comments;
    // directives are recognized when the open and close appear on the same
    // line (`{{/* LINT.IfChange */}}`) — matching the codebase's convention
    // of ignoring multi-line block comments.
    Language {
        name: "Go Template / Helm",
        extensions: r"\.(tpl|gotmpl|gohtml|tmpl)$",
        line_comments: NO_LINE,
        block_comments: GO_TEMPLATE_BLOCK,
        skip_patterns: STRING_SKIP,
    },
];

static FALLBACK: Language = Language {
    name: "Unknown",
    extensions: "",
    line_comments: &["//", "#"],
    block_comments: &[BlockComment {
        open: "/*",
        close: "*/",
        nestable: false,
    }],
    // Triple-quote strings are common enough across languages (Python, Kotlin,
    // Scala, Elixir, …) to be a safe default for unknown extensions.
    skip_patterns: TRIPLE_QUOTE_SKIP,
};

const SLASH_LINE: &[&str] = &["//"];
const SLASH_PCT_LINE: &[&str] = &["//", "%"];
const HASH_LINE: &[&str] = &["#"];
const HASH_SLASH_LINE: &[&str] = &["#", "//"];
const SLASH_HASH_LINE: &[&str] = &["//", "#"];
const DASH_LINE: &[&str] = &["--"];
const SEMI_LINE: &[&str] = &[";"];
const PCT_LINE: &[&str] = &["%"];
const NO_LINE: &[&str] = &[];

const SLASH_BLOCK: &[BlockComment] = &[BlockComment {
    open: "/*",
    close: "*/",
    nestable: false,
}];

const SLASH_BLOCK_NESTED: &[BlockComment] = &[BlockComment {
    open: "/*",
    close: "*/",
    nestable: true,
}];

const SLASH_HTML_BLOCK: &[BlockComment] = &[
    BlockComment {
        open: "/*",
        close: "*/",
        nestable: false,
    },
    BlockComment {
        open: "<!--",
        close: "-->",
        nestable: false,
    },
];

const BRACE_DASH_BLOCK_NESTED: &[BlockComment] = &[BlockComment {
    open: "{-",
    close: "-}",
    nestable: true,
}];

const HTML_BLOCK: &[BlockComment] = &[BlockComment {
    open: "<!--",
    close: "-->",
    nestable: false,
}];

const PS_BLOCK: &[BlockComment] = &[BlockComment {
    open: "<#",
    close: "#>",
    nestable: false,
}];

const GO_TEMPLATE_BLOCK: &[BlockComment] = &[BlockComment {
    open: "{{/*",
    close: "*/}}",
    nestable: false,
}];

const NO_BLOCK: &[BlockComment] = &[];

const STRING_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const BACKTICK_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(BacktickString::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const BACKTICK_FIXED_SKIP: &[SkipPattern] = &[
    SkipPattern::Fixed {
        open: "`",
        close: "`",
    },
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const CPP_RAW_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(CppRaw::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const CS_STRING_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(CsVerbatim::try_open),
    SkipPattern::Dynamic(CsRaw::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const DOLLAR_QUOTE_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(DollarQuote::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const FENCED_CODE_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(FencedCode::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const GROOVY_STRING_SKIP: &[SkipPattern] = &[
    SkipPattern::Fixed {
        open: "\"\"\"",
        close: "\"\"\"",
    },
    SkipPattern::Fixed {
        open: "'''",
        close: "'''",
    },
    SkipPattern::Fixed {
        open: "$/",
        close: "/$",
    },
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const HEREDOC_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(Heredoc::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const HEREDOC_PERCENT_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(Heredoc::try_open),
    SkipPattern::Dynamic(PercentLiteral::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const HTML_BLOCK_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(HtmlBlock::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const LONG_BRACKET_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(LongBracket::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const NIX_INDENT_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(NixIndent::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const PHP_HEREDOC_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(PhpHeredoc::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const PS_HERESTRING_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(PsHereString::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const R_RAW_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(RRaw::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const RUST_RAW_SKIP: &[SkipPattern] = &[
    SkipPattern::Dynamic(RustRaw::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const SWIFT_STRING_SKIP: &[SkipPattern] = &[
    SkipPattern::Fixed {
        open: "\"\"\"",
        close: "\"\"\"",
    },
    SkipPattern::Dynamic(SwiftExtended::try_open),
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const TRIPLE_QUOTE_SKIP: &[SkipPattern] = &[
    SkipPattern::Fixed {
        open: "\"\"\"",
        close: "\"\"\"",
    },
    SkipPattern::Fixed {
        open: "'''",
        close: "'''",
    },
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

const TRIPLE_DOUBLE_SKIP: &[SkipPattern] = &[
    SkipPattern::Fixed {
        open: "\"\"\"",
        close: "\"\"\"",
    },
    SkipPattern::Dynamic(DqString::try_open),
    SkipPattern::Dynamic(SqString::try_open),
];

/// Each variant wraps a struct with `check_close` and `find_close_offset`.
/// The macro generates the enum and dispatch — add new variants by creating
/// a struct and listing it in the invocation.
macro_rules! skip_close_enum {
    ($($Variant:ident),+ $(,)?) => {
        #[derive(Debug, Clone)]
        pub enum SkipClose {
            $($Variant($Variant),)+
        }

        impl SkipClose {
            pub fn check_close(&self, line: &str) -> bool {
                match self {
                    $(Self::$Variant(x) => x.check_close(line),)+
                }
            }

            /// None if no close on this line. Line-oriented closers
            /// return Some(after.len()) to consume the entire line.
            pub fn find_close_offset(&self, after: &str) -> Option<usize> {
                match self {
                    $(Self::$Variant(x) => x.find_close_offset(after),)+
                }
            }
        }
    }
}

skip_close_enum!(
    BacktickString,
    CsRaw,
    CsVerbatim,
    FencedCode,
    FixedClose,
    Heredoc,
    HtmlBlock,
    NixIndent,
    PercentLiteral,
    PhpHeredoc,
    PsHereString,
    SingleLineChar,
    UnescapedChar,
);

impl SkipClose {
    /// Whether an unclosed match should enter `InSkipRegion` (true) or
    /// just skip to end-of-line (false).
    pub fn spans_lines(&self) -> bool {
        !matches!(self, SkipClose::SingleLineChar(_))
    }

    pub fn advance(self, line: &str) -> Option<SkipClose> {
        match self {
            SkipClose::PercentLiteral(pl) => pl.advance(line),
            _ => (!self.check_close(line)).then_some(self),
        }
    }
}

/// JS / TS template literal backticks.
#[derive(Debug, Clone)]
pub struct BacktickString;

impl BacktickString {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        s.starts_with('`')
            .then_some((SkipClose::BacktickString(BacktickString), 1))
    }

    fn check_close(&self, line: &str) -> bool {
        find_unescaped_char(line, b'`').is_some()
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        find_unescaped_char(after, b'`')
    }
}

/// C++ raw strings (`R"delim(…)delim"`).
#[derive(Debug)]
pub struct CppRaw;

impl CppRaw {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^R"([^\s()\\]{0,16})\("#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let delimiter = caps.get(1)?.as_str();
        let close = format!("){delimiter}\"");
        Some((FixedClose::into_skip_close(close), m.end()))
    }
}

/// C# raw string literals (`"""…"""`, `""""…""""`, etc.).
#[derive(Debug, Clone)]
pub struct CsRaw {
    quote_count: usize,
}

impl CsRaw {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        if !s.starts_with("\"\"\"") {
            return None;
        }
        let count = s.bytes().take_while(|&b| b == b'"').count();
        Some((SkipClose::CsRaw(CsRaw { quote_count: count }), count))
    }

    fn check_close(&self, line: &str) -> bool {
        let bytes = line.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos] == b'"' {
                let count = bytes[pos..].iter().take_while(|&&b| b == b'"').count();
                if count >= self.quote_count {
                    return true;
                }
                pos += count;
                continue;
            }
            pos += 1;
        }
        false
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// C# verbatim strings (`@"…"`). `""` is escape; lone `"` closes.
#[derive(Debug, Clone)]
pub struct CsVerbatim;

impl CsVerbatim {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        if s.starts_with("@\"") {
            Some((SkipClose::CsVerbatim(CsVerbatim), 2))
        } else {
            None
        }
    }

    fn check_close(&self, line: &str) -> bool {
        let bytes = line.as_bytes();
        let mut pos = 0;
        while pos < bytes.len() {
            if bytes[pos] == b'"' {
                if pos + 1 < bytes.len() && bytes[pos + 1] == b'"' {
                    pos += 2;
                    continue;
                }
                return true;
            }
            pos += 1;
        }
        false
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// SQL dollar-quoted strings (`$$…$$`, `$tag$…$tag$`).
#[derive(Debug)]
pub struct DollarQuote;

impl DollarQuote {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^\$([A-Za-z_]\w*)?\$"#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let tag = caps.get(1).map(|c| c.as_str()).unwrap_or("");
        let close = format!("${tag}$");
        Some((FixedClose::into_skip_close(close), m.end()))
    }
}

/// Markdown / RST fenced code blocks (``` or ~~~).
#[derive(Debug, Clone)]
pub struct FencedCode {
    fence_char: char,
    fence_count: usize,
}

impl FencedCode {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        let trimmed = s.trim_start();
        let fence_char = trimmed.as_bytes().first()?;
        if *fence_char != b'`' && *fence_char != b'~' {
            return None;
        }
        let fence_count = trimmed.bytes().take_while(|&b| b == *fence_char).count();
        if fence_count < 3 {
            return None;
        }
        Some((
            SkipClose::FencedCode(FencedCode {
                fence_char: *fence_char as char,
                fence_count,
            }),
            s.len(),
        ))
    }

    fn check_close(&self, line: &str) -> bool {
        let trimmed = line.trim();
        let fc = self.fence_char as u8;
        if trimmed.is_empty() || trimmed.as_bytes()[0] != fc {
            return false;
        }
        let count = trimmed.bytes().take_while(|&b| b == fc).count();
        if count < self.fence_count {
            return false;
        }
        trimmed[count..].trim().is_empty()
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// Close strategy for `SkipPattern::Fixed` — literal substring match.
#[derive(Debug, Clone)]
pub struct FixedClose {
    pub close: String,
}

impl FixedClose {
    pub fn into_skip_close(close: String) -> SkipClose {
        SkipClose::FixedClose(FixedClose { close })
    }

    fn check_close(&self, line: &str) -> bool {
        line.contains(self.close.as_str())
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        after
            .find(self.close.as_str())
            .map(|p| p + self.close.len())
    }
}

/// Shell / Ruby / Perl heredocs (`<<EOF`, `<<~EOF`, `<<-'EOF'`).
#[derive(Debug, Clone)]
pub struct Heredoc {
    ident: String,
}

impl Heredoc {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^<<[-~]?\s*['"]?([A-Za-z_]\w*)['"]?"#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let ident = caps.get(1)?.as_str().to_string();
        Some((SkipClose::Heredoc(Heredoc { ident }), m.end()))
    }

    fn check_close(&self, line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.strip_suffix(';').unwrap_or(trimmed).trim() == self.ident
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// HTML `<script>`, `<style>`, and `<![CDATA[` blocks.
#[derive(Debug, Clone)]
pub struct HtmlBlock {
    close: String,
}

impl HtmlBlock {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        let b = s.as_bytes();
        let (close, advance) = if b
            .get(..9)
            .is_some_and(|p| p.eq_ignore_ascii_case(b"<![cdata["))
        {
            ("]]>".to_string(), 9)
        } else if b
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case(b"<script"))
        {
            if !b
                .get(7)
                .is_some_and(|&c| c == b'>' || c.is_ascii_whitespace())
            {
                return None;
            }
            ("</script>".to_string(), 7)
        } else if b
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case(b"<style"))
        {
            if !b
                .get(6)
                .is_some_and(|&c| c == b'>' || c.is_ascii_whitespace())
            {
                return None;
            }
            ("</style>".to_string(), 6)
        } else {
            return None;
        };
        Some((SkipClose::HtmlBlock(HtmlBlock { close }), advance))
    }

    fn check_close(&self, line: &str) -> bool {
        let close = self.close.as_bytes();
        line.as_bytes()
            .windows(close.len())
            .any(|w| w.eq_ignore_ascii_case(close))
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        let close = self.close.as_bytes();
        after
            .as_bytes()
            .windows(close.len())
            .position(|w| w.eq_ignore_ascii_case(close))
            .map(|p| p + close.len())
    }
}

/// Lua / CMake long bracket strings (`[==[…]==]`).
#[derive(Debug)]
pub struct LongBracket;

impl LongBracket {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        if !s.starts_with('[') {
            return None;
        }
        let level = s[1..].bytes().take_while(|&b| b == b'=').count();
        if s.as_bytes().get(1 + level) != Some(&b'[') {
            return None;
        }
        let open_len = 2 + level; // [=*[
        let close = format!("]{}]", "=".repeat(level));
        Some((FixedClose::into_skip_close(close), open_len))
    }
}

/// Nix indented strings (`''…''`).
#[derive(Debug, Clone)]
pub struct NixIndent;

impl NixIndent {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        if !s.starts_with("''") {
            return None;
        }
        let after = s.get(2..3).unwrap_or("");
        if after == "'" {
            return None;
        }
        Some((SkipClose::NixIndent(NixIndent), 2))
    }

    fn check_close(&self, line: &str) -> bool {
        let bytes = line.as_bytes();
        let mut pos = 0;
        while pos + 1 < bytes.len() {
            if bytes[pos] == b'\'' && bytes[pos + 1] == b'\'' {
                let next = bytes.get(pos + 2);
                match next {
                    Some(b'$') | Some(b'\'') | Some(b'\\') => {
                        pos += 3;
                        continue;
                    }
                    _ => return true,
                }
            }
            pos += 1;
        }
        false
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// Ruby / Perl percent-literals (`%q(…)`, `%w[…]`, `qq!…!`, etc.).
#[derive(Debug, Clone)]
pub struct PercentLiteral {
    close_char: char,
    paired: bool,
    depth: usize,
}

impl PercentLiteral {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^(?:%[qQwWiIrxs]?|qq?)([^\w\s])"#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let delim = caps.get(1)?.as_str().chars().next()?;
        let (close_char, paired) = match delim {
            '(' => (')', true),
            '[' => (']', true),
            '{' => ('}', true),
            '<' => ('>', true),
            c => (c, false),
        };
        Some((
            SkipClose::PercentLiteral(PercentLiteral {
                close_char,
                paired,
                depth: 0,
            }),
            m.end(),
        ))
    }

    fn check_close(&self, line: &str) -> bool {
        debug_assert!(
            self.close_char.is_ascii(),
            "delimiter must be ASCII for UTF-8 safety"
        );
        let bytes = line.as_bytes();
        let mut pos = 0;
        let mut d = self.depth;
        let open_char = self.open_char();

        while pos < bytes.len() {
            if bytes[pos] == b'\\' {
                pos = (pos + 2).min(bytes.len());
                continue;
            }

            if !self.paired {
                if bytes[pos] == self.close_char as u8 {
                    return true;
                }
                pos += 1;
                continue;
            }

            if open_char.is_some_and(|oc| bytes[pos] == oc as u8) {
                d += 1;
                pos += 1;
                continue;
            }

            if bytes[pos] == self.close_char as u8 {
                if d == 0 {
                    return true;
                }
                d -= 1;
                pos += 1;
                continue;
            }

            pos += 1;
        }
        false
    }

    fn advance(self, line: &str) -> Option<SkipClose> {
        if self.check_close(line) {
            return None;
        }

        if !self.paired {
            return Some(SkipClose::PercentLiteral(self));
        }

        Some(SkipClose::PercentLiteral(PercentLiteral {
            depth: self.track_depth(line),
            ..self
        }))
    }

    fn open_char(&self) -> Option<char> {
        match self.close_char {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            '>' => Some('<'),
            _ => None,
        }
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }

    fn track_depth(&self, line: &str) -> usize {
        debug_assert!(
            self.close_char.is_ascii(),
            "delimiter must be ASCII for UTF-8 safety"
        );
        let open_char = match self.open_char() {
            Some(c) => c,
            None => return self.depth,
        };
        let bytes = line.as_bytes();
        let mut pos = 0;
        let mut d = self.depth;
        while pos < bytes.len() {
            if bytes[pos] == b'\\' {
                pos = (pos + 2).min(bytes.len());
                continue;
            }

            if bytes[pos] == open_char as u8 {
                d += 1;
                pos += 1;
                continue;
            }

            if bytes[pos] == self.close_char as u8 {
                d = d.saturating_sub(1);
            }
            pos += 1;
        }
        d
    }
}

/// PHP heredocs (`<<<EOF`, `<<<'EOF'`).
#[derive(Debug, Clone)]
pub struct PhpHeredoc {
    ident: String,
}

impl PhpHeredoc {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^<<<\s*['"]?([A-Za-z_]\w*)['"]?"#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let ident = caps.get(1)?.as_str().to_string();
        Some((SkipClose::PhpHeredoc(PhpHeredoc { ident }), m.end()))
    }

    fn check_close(&self, line: &str) -> bool {
        let trimmed = line.trim();
        trimmed.strip_suffix(';').unwrap_or(trimmed).trim() == self.ident
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// PowerShell here-strings (`@"…"@`, `@'…'@`).
#[derive(Debug, Clone)]
pub struct PsHereString {
    close: String,
}

impl PsHereString {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^@([\"'])\s*$"#).unwrap());
        let caps = re.captures(s)?;
        let quote = caps.get(1)?.as_str().chars().next()?;
        let close = format!("{quote}@");
        Some((SkipClose::PsHereString(PsHereString { close }), s.len()))
    }

    fn check_close(&self, line: &str) -> bool {
        line.starts_with(&self.close)
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        Some(after.len())
    }
}

/// R raw strings (`r"(…)"`, `r"-(…)-"`, `r"{…}"`).
#[derive(Debug)]
pub struct RRaw;

impl RRaw {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^[rR]"(-*)([\(\[\{])"#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let dashes = caps.get(1)?.as_str();
        let open_bracket = caps.get(2)?.as_str().chars().next()?;
        let close_bracket = match open_bracket {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            _ => return None,
        };
        let close = format!("{close_bracket}{dashes}\"");
        Some((FixedClose::into_skip_close(close), m.end()))
    }
}

/// Rust raw strings (`r#"…"#`, `r##"…"##`).
#[derive(Debug)]
pub struct RustRaw;

impl RustRaw {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^r(#+)""#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let hashes = caps.get(1)?.as_str().len();
        let close = format!("\"{}", "#".repeat(hashes));
        Some((FixedClose::into_skip_close(close), m.end()))
    }
}

/// Swift extended string delimiters (`#"…"#`, `#"""…"""#`).
#[derive(Debug)]
pub struct SwiftExtended;

impl SwiftExtended {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        static RE: OnceLock<Regex> = OnceLock::new();
        let re = RE.get_or_init(|| Regex::new(r#"^(#+)("{1,3})"#).unwrap());
        let caps = re.captures(s)?;
        let m = caps.get(0)?;
        let hashes = caps.get(1)?.as_str().len();
        let quotes = caps.get(2)?.as_str();
        let close = format!("{}{}", quotes, "#".repeat(hashes));
        Some((FixedClose::into_skip_close(close), m.end()))
    }
}

/// Double-quoted string literal (`"…"`). Unclosed `"` enters a skip region
/// that spans lines (matches the multiline behavior of Rust, Ruby, Shell, etc.;
/// safe for single-line-only languages because unclosed `"` is rare in practice).
#[derive(Debug)]
pub struct DqString;

impl DqString {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        s.starts_with('"')
            .then_some((UnescapedChar::into_skip_close(b'"'), 1))
    }
}

/// Single-quoted string literal (`'…'`). Unclosed `'` skips to end-of-line
/// only — prevents Rust lifetime annotations (`'a`) or other non-string uses
/// from swallowing subsequent lines.
#[derive(Debug)]
pub struct SqString;

impl SqString {
    pub fn try_open(s: &str) -> Option<(SkipClose, usize)> {
        s.starts_with('\'')
            .then_some((SingleLineChar::into_skip_close(b'\''), 1))
    }
}

/// Close strategy for single-line string literals. Same matching logic as
/// `UnescapedChar`, but the scanner skips to end-of-line instead of entering
/// `InSkipRegion` when the close is not found on the current line.
#[derive(Debug, Clone)]
pub struct SingleLineChar {
    pub target: u8,
}

impl SingleLineChar {
    pub fn into_skip_close(target: u8) -> SkipClose {
        SkipClose::SingleLineChar(SingleLineChar { target })
    }

    fn check_close(&self, line: &str) -> bool {
        find_unescaped_char(line, self.target).is_some()
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        find_unescaped_char(after, self.target)
    }
}

/// Close strategy for unclosed string literals on code lines.
#[derive(Debug, Clone)]
pub struct UnescapedChar {
    pub target: u8,
}

impl UnescapedChar {
    pub fn into_skip_close(target: u8) -> SkipClose {
        SkipClose::UnescapedChar(UnescapedChar { target })
    }

    fn check_close(&self, line: &str) -> bool {
        find_unescaped_char(line, self.target).is_some()
    }

    fn find_close_offset(&self, after: &str) -> Option<usize> {
        find_unescaped_char(after, self.target)
    }
}

fn find_unescaped_char(s: &str, target: u8) -> Option<usize> {
    debug_assert!(target.is_ascii(), "target must be ASCII for UTF-8 safety");
    let bytes = s.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos] == b'\\' {
            pos = (pos + 2).min(bytes.len());
            continue;
        }
        if bytes[pos] == target {
            return Some(pos + 1);
        }
        pos += 1;
    }
    None
}
