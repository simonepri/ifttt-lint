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
    let mut root = cli.root.clone().unwrap_or_else(git::resolve_root);

    // Stage 1: Parse input
    // In scan mode, override root to the scan directory so file paths resolve correctly.
    let (changes, content_hint) = if let Some(scan_dir) = &cli.scan {
        root = scan_dir.clone();
        // from_directory reads each file once and returns the content alongside the
        // ChangeMap. Passing it to check() avoids reading every file a second time
        // during directive parsing.
        let (chgs, content) = changes::from_directory(scan_dir);
        (chgs, Some(content))
    } else {
        let diff_input = match git::resolve_input(&cli.input) {
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

        (changes, None)
    };

    if changes.is_empty() || std::env::var("NO_IFTTT").is_ok() {
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
    let result = check::check(&changes, &root, &ignore_patterns, content_hint.as_ref());

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
