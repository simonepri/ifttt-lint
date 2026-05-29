use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use clap::{Parser, ValueEnum};

use crate::vcs::VcsProvider;
use crate::{check, reports, vcs_git, vcs_jj};

#[derive(Parser)]
#[command(
    name = "ifttt-lint",
    version,
    about = "Enforces atomic changes via LINT.IfChange/ThenChange directives"
)]
struct Cli {
    /// VCS ref range / revset to diff (e.g. `main...HEAD` for git, `main..@` for jj).
    #[arg(short, long)]
    diff: Option<String>,

    /// Worker thread count (0 = auto).
    #[arg(short, long, default_value = "0")]
    threads: usize,

    /// Require // prefix on all ThenChange paths.
    /// Use --strict=false to accept bare and single-/ paths.
    #[arg(long, default_value_t = true, num_args = 0..=1, default_missing_value = "true")]
    strict: bool,

    /// Ignore target permanently (repeatable, glob syntax).
    #[arg(short, long)]
    ignore: Vec<String>,

    /// Files to validate structurally: checks that every ThenChange target and
    /// label exists on disk, regardless of whether the file was modified.
    /// Intended for use with pre-commit's `pass_filenames: true`.
    files: Vec<PathBuf>,

    /// Output format.
    #[arg(short, long, default_value = "pretty")]
    format: reports::Format,

    /// VCS backend. Auto-detected from `.jj/` / `.git/` presence if omitted.
    #[arg(long, value_enum)]
    vcs: Option<VcsKind>,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum VcsKind {
    Git,
    Jj,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();

    let threads = if cli.threads == 0 {
        check::DEFAULT_THREADS
    } else {
        cli.threads
    };

    if cli.files.is_empty() && cli.diff.is_none() {
        eprintln!(
            "Nothing to check — pass FILES for structural validation or --diff RANGE for diff validation."
        );
        return ExitCode::SUCCESS;
    }

    let vcs_kind = resolve_vcs_kind(cli.vcs);
    if let Err(code) = ensure_binary_available(vcs_kind) {
        return code;
    }

    let root = match resolve_root(vcs_kind) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // In structural mode (files provided, no explicit --diff), skip the staged
    // diff entirely — co-change checking belongs to the push hook, not pre-commit.
    let structural_only = !cli.files.is_empty() && cli.diff.is_none();

    let vcs: Box<dyn VcsProvider> = match vcs_kind {
        VcsKind::Git => Box::new(vcs_git::GitVcsProvider::new(
            root, cli.diff, cli.strict, cli.files,
        )),
        VcsKind::Jj => Box::new(vcs_jj::JjVcsProvider::new(
            root, cli.diff, cli.strict, cli.files,
        )),
    };

    let changes = if structural_only {
        crate::vcs::ChangeMap::default()
    } else {
        match vcs.diff() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(2);
            }
        }
    };

    // NO_IFTTT suppression preserves deleted-file markers (for reverse-lookup)
    // and the structural validity pass — only diff-based line data is suppressed.
    let suppression = match vcs.suppressions() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let changes = if let Some(reason) = &suppression {
        eprintln!("info: IFTTT checks suppressed: {reason}");
        changes.into_iter().filter(|(_, fc)| fc.deleted).collect()
    } else {
        changes
    };

    if changes.is_empty() && vcs.validate_files().is_empty() {
        return ExitCode::SUCCESS;
    }

    let ignore_patterns: Vec<globset::GlobMatcher> = cli
        .ignore
        .iter()
        .filter_map(|pattern| {
            match globset::GlobBuilder::new(pattern)
                .literal_separator(true)
                .build()
            {
                Ok(g) => Some(g.compile_matcher()),
                Err(e) => {
                    eprintln!("warning: invalid ignore pattern '{pattern}': {e}");
                    None
                }
            }
        })
        .collect();

    let result = match check::check(vcs.as_ref(), &changes, &ignore_patterns, threads) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let output = reports::format(
        &result,
        cli.format,
        std::io::IsTerminal::is_terminal(&std::io::stderr()),
    );
    if output.is_empty() {
        return ExitCode::SUCCESS;
    }

    if matches!(cli.format, reports::Format::Json) {
        print!("{output}");
    } else {
        eprint!("{output}");
    }
    ExitCode::from(1)
}

/// Resolve the backend to use. Explicit `--vcs` wins; otherwise auto-detect
/// from the marker directories. Auto-detection scans the current working
/// directory and its ancestors — `.jj/` takes precedence over `.git/` so users
/// running jj on a colocated repo get jj's view by default.
fn resolve_vcs_kind(explicit: Option<VcsKind>) -> VcsKind {
    if let Some(kind) = explicit {
        return kind;
    }
    let Ok(cwd) = std::env::current_dir() else {
        return VcsKind::Git;
    };
    detect_vcs_from_ancestors(&cwd)
}

fn detect_vcs_from_ancestors(start: &Path) -> VcsKind {
    for dir in start.ancestors() {
        if dir.join(".jj").is_dir() {
            return VcsKind::Jj;
        }
        if dir.join(".git").exists() {
            return VcsKind::Git;
        }
    }
    VcsKind::Git
}

fn resolve_root(kind: VcsKind) -> anyhow::Result<PathBuf> {
    match kind {
        VcsKind::Git => vcs_git::GitVcsProvider::resolve_root(),
        VcsKind::Jj => vcs_jj::JjVcsProvider::resolve_root(),
    }
}

fn ensure_binary_available(kind: VcsKind) -> Result<(), ExitCode> {
    let (bin, install) = match kind {
        VcsKind::Git => ("git", "https://git-scm.com/downloads"),
        VcsKind::Jj => ("jj", "https://jj-vcs.dev/latest/install-and-setup/"),
    };
    let probe = Command::new(bin)
        .arg("--version")
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .status();
    if probe.is_ok_and(|s| s.success()) {
        return Ok(());
    }
    eprintln!("error: {bin} not found on PATH. Install: {install}");
    Err(ExitCode::from(2))
}

#[cfg(test)]
#[path = "cli_test.rs"]
mod tests;
