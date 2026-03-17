use std::path::PathBuf;
use std::process;

use clap::Parser;
use ifttt_lint::vcs::VcsProvider as _;

use ifttt_lint::{check, reports, vcs_git};

#[derive(Parser)]
#[command(
    name = "ifttt-lint",
    version,
    about = "Enforces atomic changes via LINT.IfChange/ThenChange directives"
)]
struct Cli {
    /// Git ref range to diff (e.g. main...HEAD).
    /// Default: staged changes (git diff --cached).
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

fn main() {
    let cli = Cli::parse();

    // Default to 2 threads: file I/O contention on most OSes makes higher
    // counts counterproductive (benchmarked on Chromium and TensorFlow).
    let threads = if cli.threads == 0 { 2 } else { cli.threads };
    if let Err(e) = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build_global()
    {
        eprintln!("warning: failed to configure thread pool: {e}");
    }

    let root = match vcs_git::GitVcsProvider::resolve_root() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(2);
        }
    };

    let vcs = vcs_git::GitVcsProvider::new(root, cli.diff, cli.strict, cli.files);

    let changes = match vcs.diff() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(2);
        }
    };

    // NO_IFTTT suppression preserves deleted-file markers (for reverse-lookup)
    // and the structural validity pass — only diff-based line data is suppressed.
    let suppression = match vcs.suppressions() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(2);
        }
    };
    let changes = if suppression.is_some() {
        changes.into_iter().filter(|(_, fc)| fc.deleted).collect()
    } else {
        changes
    };

    if changes.is_empty() && vcs.validate_files().is_empty() {
        process::exit(0);
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

    let result = check::check(&vcs, &changes, &ignore_patterns);
    let output = reports::format(
        &result,
        cli.format,
        std::io::IsTerminal::is_terminal(&std::io::stderr()),
    );
    if !output.is_empty() {
        if matches!(cli.format, reports::Format::Json) {
            print!("{output}");
        } else {
            eprint!("{output}");
        }
        process::exit(1);
    }
}
