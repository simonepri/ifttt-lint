use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;

use crate::vcs::VcsProvider as _;
use crate::{check, reports, vcs_git};

#[derive(Parser)]
#[command(
    name = "ifttt-lint",
    version,
    about = "Enforces atomic changes via LINT.IfChange/ThenChange directives"
)]
struct Cli {
    /// Git ref range to diff (e.g. main...HEAD).
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

    let root = match vcs_git::GitVcsProvider::resolve_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    // In structural mode (files provided, no explicit --diff), skip the staged
    // diff entirely — co-change checking belongs to the push hook, not pre-commit.
    let structural_only = !cli.files.is_empty() && cli.diff.is_none();

    let vcs = vcs_git::GitVcsProvider::new(root, cli.diff, cli.strict, cli.files);

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
    let changes = if suppression.is_some() {
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
        .filter_map(|pattern| match globset::Glob::new(pattern) {
            Ok(g) => Some(g.compile_matcher()),
            Err(e) => {
                eprintln!("warning: invalid ignore pattern '{pattern}': {e}");
                None
            }
        })
        .collect();

    let result = match check::check(&vcs, &changes, &ignore_patterns, threads) {
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

#[cfg(test)]
#[path = "cli_test.rs"]
mod tests;
