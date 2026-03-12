use std::io::IsTerminal;
use std::path::PathBuf;

#[derive(Debug)]
pub struct DiffInput {
    pub diff: String,
}

/// Resolve the input argument into a diff string.
///
/// Recognised `--diff` values:
/// - `-`     — read diff from stdin
/// - `A...B` / `A..B` — git ref range
/// - `path`  — read diff from a file on disk
/// - `None`  — auto-detect git upstream (TTY) or read stdin (piped)
pub fn resolve_input(input: &Option<String>) -> Result<DiffInput, String> {
    match input.as_deref() {
        None => resolve_auto(),
        Some("-") => Ok(DiffInput {
            diff: read_stdin()?,
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
            let diff = diff(&diff_range)?;
            parse_no_ifttt_from_commits(&log_range);
            Ok(DiffInput { diff })
        }
        Some(path) if std::path::Path::new(path).exists() => Ok(DiffInput {
            diff: std::fs::read_to_string(path)
                .map_err(|e| format!("failed to read {path}: {e}"))?,
        }),
        Some(arg) => Err(format!(
            "'{arg}' is not a file or git ref range (use BASE...HEAD syntax)"
        )),
    }
}

/// Auto-detect: git upstream if TTY, stdin if piped.
fn resolve_auto() -> Result<DiffInput, String> {
    if std::io::stdin().is_terminal() {
        let range = detect_range()?;
        let d = diff(&range)?;
        let log_range = three_dot_to_log_range(&range);
        parse_no_ifttt_from_commits(&log_range);
        Ok(DiffInput { diff: d })
    } else {
        let stdin_content = read_stdin()?;
        if stdin_content.is_empty() {
            // No diff piped (e.g. pre-commit hook where stdin is /dev/null).
            // Fall back to staged changes so the hook works out of the box.
            Ok(DiffInput {
                diff: staged_diff()?,
            })
        } else {
            Ok(DiffInput {
                diff: stdin_content,
            })
        }
    }
}

pub fn resolve_root() -> PathBuf {
    if let Some(root) = detect_root() {
        return root;
    }
    std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("error: failed to determine project root: {e}");
        std::process::exit(2);
    })
}

fn read_stdin() -> Result<String, String> {
    use std::io::Read;
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("failed to read stdin: {e}"))?;
    Ok(input)
}

fn staged_diff() -> Result<String, String> {
    let output = std::process::Command::new("git")
        .args(["diff", "--cached"])
        .output()
        .map_err(|e| format!("failed to run git diff --cached: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git diff --cached failed: {stderr}"));
    }

    String::from_utf8(output.stdout)
        .map_err(|e| format!("git diff --cached output is not UTF-8: {e}"))
}

fn diff(range: &str) -> Result<String, String> {
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

fn parse_no_ifttt_from_commits(log_range: &str) {
    let Ok(output) = std::process::Command::new("git")
        .args(["log", "--format=%B", log_range])
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let log = String::from_utf8_lossy(&output.stdout);
    if let Some(line) = log.lines().find(|l| l.starts_with("NO_IFTTT=")) {
        let reason = line.trim_start_matches("NO_IFTTT=");
        std::env::set_var("NO_IFTTT", reason);
    }
}

fn detect_range() -> Result<String, String> {
    if let Some(upstream) = rev_parse("@{u}") {
        let head = rev_parse("HEAD").unwrap_or_else(|| "HEAD".to_string());
        return Ok(format!("{upstream}...{head}"));
    }

    for base in ["origin/main", "origin/master"] {
        if ref_exists(base) {
            return Ok(format!("{base}...HEAD"));
        }
    }

    Err("no upstream branch found. Provide a ref range (e.g. main...HEAD) or pipe a diff".into())
}

fn detect_root() -> Option<PathBuf> {
    let path = rev_parse("--show-toplevel")?;
    Some(PathBuf::from(path))
}

fn rev_parse(arg: &str) -> Option<String> {
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

fn ref_exists(refname: &str) -> bool {
    std::process::Command::new("git")
        .args(["rev-parse", "--verify", refname])
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "git_test.rs"]
mod tests;

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
