use std::io::IsTerminal;
use std::path::PathBuf;
use std::process;

use clap::Parser;
use ifttt_lint::{changes, check, reports};

#[derive(Parser)]
#[command(
    name = "ifttt-lint",
    version,
    about = "Enforces atomic changes via LINT.IfChange/ThenChange directives"
)]
struct Cli {
    /// Git ref range (e.g. main...HEAD), diff file path, or - for stdin.
    /// With no argument: auto-detects git upstream and diffs against it.
    input: Option<String>,

    /// Project root for // paths. Defaults to git repo root or cwd.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Worker thread count (0 = auto).
    #[arg(short, long, default_value = "0")]
    threads: usize,

    /// Ignore target permanently (repeatable, glob syntax).
    #[arg(short, long)]
    ignore: Vec<String>,

    /// Scan mode: validate directive syntax in a directory.
    #[arg(short, long)]
    scan: Option<PathBuf>,

    /// Output format.
    #[arg(short, long, default_value = "pretty")]
    format: reports::Format,
}

/// The source of the diff and allow patterns.
struct DiffInput {
    diff: String,
    allows: Vec<String>,
}

fn main() {
    let cli = Cli::parse();

    // Configure rayon thread pool
    if cli.threads > 0 {
        if let Err(e) = rayon::ThreadPoolBuilder::new()
            .num_threads(cli.threads)
            .build_global()
        {
            eprintln!("warning: failed to configure thread pool: {e}");
        }
    }

    // Determine project root
    let mut root = match &cli.root {
        Some(r) => r.clone(),
        None => resolve_root(),
    };

    // Stage 1: Parse input
    // In scan mode, override root to the scan directory so file paths resolve correctly.
    let (changes, extra_allows, content_hint) = if let Some(scan_dir) = &cli.scan {
        root = scan_dir.clone();
        // from_directory reads each file once and returns the content alongside the
        // ChangeMap. Passing it to check() avoids reading every file a second time
        // during directive parsing.
        let (chgs, content) = changes::from_directory(scan_dir);
        (chgs, vec![], Some(content))
    } else {
        let diff_input = match resolve_input(&cli.input) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(2);
            }
        };

        let mut cursor = std::io::Cursor::new(diff_input.diff);
        let changes = match changes::from_diff(&mut cursor) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(2);
            }
        };

        (changes, diff_input.allows, None)
    };

    if changes.is_empty() {
        process::exit(0);
    }

    // Build ignore patterns: --ignore flags + IFTTT_ALLOW env + git commit allows
    let env_allows = match std::env::var("IFTTT_ALLOW") {
        Ok(v) => v,
        Err(std::env::VarError::NotPresent) => String::new(),
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("warning: IFTTT_ALLOW contains non-UTF-8 characters, ignoring");
            String::new()
        }
    };
    let env_allow_strings: Vec<String> = env_allows.split_whitespace().map(String::from).collect();
    let ignore_patterns: Vec<globset::GlobMatcher> = cli
        .ignore
        .iter()
        .chain(env_allow_strings.iter())
        .chain(extra_allows.iter())
        .filter_map(|pattern| match globset::Glob::new(pattern) {
            Ok(g) => Some(g.compile_matcher()),
            Err(e) => {
                eprintln!("warning: invalid ignore pattern '{pattern}': {e}");
                None
            }
        })
        .collect();

    // Stage 2: Check
    let result = check::check(&changes, &root, &ignore_patterns, content_hint.as_ref());

    // Stage 3: Format
    let output = reports::format(&result, cli.format, std::io::stderr().is_terminal());
    if !output.is_empty() {
        if matches!(cli.format, reports::Format::Json) {
            print!("{output}");
        } else {
            eprint!("{output}");
        }
        process::exit(1);
    }
}

/// Resolve the input argument into a diff string and allow patterns.
fn resolve_input(input: &Option<String>) -> Result<DiffInput, String> {
    match input.as_deref() {
        Some("-") => Ok(DiffInput {
            diff: read_stdin()?,
            allows: vec![],
        }),
        // `...` (three-dot) check must come before `..` (two-dot) since "..." also
        // contains "..". The three-dot form is the symmetric diff (`git diff A...B`);
        // the two-dot form is a standard log range (`git log A..B`).
        Some(arg) if arg.contains("...") || arg.contains("..") => {
            let (diff_range, log_range) = if arg.contains("...") {
                (arg.to_string(), three_dot_to_log_range(arg))
            } else {
                (arg.to_string(), arg.to_string())
            };
            let diff = git_diff(&diff_range)?;
            let allows = parse_allows_from_commits(&log_range);
            Ok(DiffInput { diff, allows })
        }
        Some(path) if std::path::Path::new(path).exists() => Ok(DiffInput {
            diff: std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read {path}: {e}"))?,
            allows: vec![],
        }),
        Some(arg) => Err(format!(
            "'{arg}' is not a file or git ref range (use BASE...HEAD syntax)"
        )),
        None => {
            if std::io::stdin().is_terminal() {
                let range = detect_git_range()?;
                let diff = git_diff(&range)?;
                let log_range = three_dot_to_log_range(&range);
                let allows = parse_allows_from_commits(&log_range);
                Ok(DiffInput { diff, allows })
            } else {
                Ok(DiffInput {
                    diff: read_stdin()?,
                    allows: vec![],
                })
            }
        }
    }
}

fn read_stdin() -> Result<String, String> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("failed to read stdin: {e}"))?;
    Ok(input)
}

fn git_diff(range: &str) -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(["diff", range])
        .output()
        .map_err(|e| format!("failed to run git diff: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff failed: {stderr}"));
    }

    String::from_utf8(output.stdout).map_err(|e| format!("git diff output is not UTF-8: {e}"))
}

fn parse_allows_from_commits(log_range: &str) -> Vec<String> {
    let output = std::process::Command::new("git")
        .args(["log", "--format=%B", log_range])
        .stderr(std::process::Stdio::null())
        .output();

    let Ok(output) = output else {
        return vec![];
    };
    if !output.status.success() {
        return vec![];
    }

    let body = String::from_utf8_lossy(&output.stdout);
    body.lines()
        .filter_map(|line| line.strip_prefix("IFTTT_ALLOW="))
        .flat_map(|s| s.split_whitespace())
        .map(String::from)
        .collect()
}

fn detect_git_range() -> Result<String, String> {
    if let Some(upstream) = git_rev_parse("@{u}") {
        let head = git_rev_parse("HEAD").unwrap_or_else(|| "HEAD".to_string());
        return Ok(format!("{upstream}...{head}"));
    }

    for base in ["origin/main", "origin/master"] {
        if git_ref_exists(base) {
            return Ok(format!("{base}...HEAD"));
        }
    }

    Err("no upstream branch found. Provide a ref range (e.g. main...HEAD) or pipe a diff".into())
}

fn resolve_root() -> PathBuf {
    if let Some(root) = detect_git_root() {
        return root;
    }
    std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("error: failed to determine project root: {e}");
        process::exit(2);
    })
}

fn detect_git_root() -> Option<PathBuf> {
    let path = git_rev_parse("--show-toplevel")?;
    Some(PathBuf::from(path))
}

fn git_rev_parse(arg: &str) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", arg])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8(output.stdout).ok()?;
        Some(path.trim().to_string())
    } else {
        None
    }
}

fn git_ref_exists(refname: &str) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", refname])
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Convert a three-dot symmetric-diff range (`base...head`) to a two-dot log
/// range (`base..head`) for use with `git log`.
///
/// Uses `rsplit_once` so that only the *range separator* (the rightmost `...`)
/// is converted, leaving any `...` that happens to appear inside a ref name
/// intact. Branch names containing `...` are invalid in git, but being
/// conservative here costs nothing.
fn three_dot_to_log_range(range: &str) -> String {
    match range.rsplit_once("...") {
        Some((base, head)) => format!("{base}..{head}"),
        None => range.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_dot_to_log_range_basic() {
        assert_eq!(
            three_dot_to_log_range("origin/main...HEAD"),
            "origin/main..HEAD"
        );
    }

    /// A ref name hypothetically containing "..." must not be corrupted: only
    /// the rightmost separator is converted. With the old `replace("...", "..")`
    /// this test fails because every "..." is replaced.
    #[test]
    fn three_dot_to_log_range_preserves_dots_in_base() {
        assert_eq!(
            three_dot_to_log_range("feat...ure...HEAD"),
            "feat...ure..HEAD"
        );
    }
}
