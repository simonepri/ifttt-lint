use std::path::PathBuf;
use std::process;

use clap::Parser;
use ifttt_lint::{changes, check, reports};

mod git;

#[derive(Parser)]
#[command(
    name = "ifttt-lint",
    version,
    about = "Enforces atomic changes via LINT.IfChange/ThenChange directives"
)]
struct Cli {
    /// Diff source: git ref range (e.g. main...HEAD), path to a patch file, or - for stdin.
    /// Default: auto-detect git upstream (TTY) or read stdin (piped).
    #[arg(short, long)]
    diff: Option<String>,

    /// Project root for // paths. Defaults to git repo root or cwd.
    #[arg(long)]
    root: Option<PathBuf>,

    /// Worker thread count (0 = auto).
    #[arg(short, long, default_value = "0")]
    threads: usize,

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
    let root = cli.root.clone().unwrap_or_else(git::resolve_root);

    // Stage 1: Parse diff input.
    // When FILES are given without --diff, skip the diff pass: the caller only
    // wants structural validation and we avoid unnecessary git upstream detection.
    let skip_diff = cli.diff.is_none() && !cli.files.is_empty();
    let changes = if skip_diff {
        changes::ChangeMap::new()
    } else {
        let diff_input = match git::resolve_input(&cli.diff) {
            Ok(i) => i,
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(2);
            }
        };
        let mut cursor = std::io::Cursor::new(diff_input.diff);
        match changes::from_diff(&mut cursor) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("error: {e}");
                process::exit(2);
            }
        }
    };

    // Build file_list from positional args (root-relative paths).
    let file_list: Vec<String> = cli
        .files
        .iter()
        .filter_map(|p| {
            // Try to make path root-relative; fall back to the path as-is if it
            // is already relative (e.g. passed by pre-commit without an absolute prefix).
            // On Windows, strip_prefix may fail if separator styles differ between `p`
            // and `root`; the is_relative fallback handles that case and
            // normalize_path_str normalizes to forward slashes for cache key matching.
            if let Ok(rel) = p.strip_prefix(&root) {
                Some(ifttt_lint::check::normalize_path_str(
                    &rel.to_string_lossy(),
                ))
            } else if p.is_relative() {
                Some(ifttt_lint::check::normalize_path_str(&p.to_string_lossy()))
            } else {
                eprintln!(
                    "warning: '{}' is outside the project root and will be skipped",
                    p.display()
                );
                None
            }
        })
        .collect();

    // NO_IFTTT suppresses diff-based validation (added/removed line data) but
    // preserves deleted-file markers so the reverse-lookup pass still detects
    // surviving files that reference a deleted target.  The structural validity
    // pass (file_list) is also unaffected.
    let changes = if std::env::var("NO_IFTTT").is_ok() {
        changes
            .into_iter()
            .filter_map(
                |(path, fc)| {
                    if fc.deleted {
                        Some((path, fc))
                    } else {
                        None
                    }
                },
            )
            .collect()
    } else {
        changes
    };

    if changes.is_empty() && file_list.is_empty() {
        process::exit(0);
    }

    // Build ignore patterns from --ignore flags
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

    // Stage 2: Check
    let result = check::check(&changes, &root, &ignore_patterns, &file_list);

    // Stage 3: Format
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
