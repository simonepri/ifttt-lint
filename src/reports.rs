use std::fmt::Write;

use crate::check::{CheckResult, Diagnostic, Severity};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[non_exhaustive]
pub enum Format {
    Pretty,
    Json,
    Plain,
}

/// Returns empty string if no diagnostics.
pub fn format(result: &CheckResult, fmt: Format, color: bool) -> String {
    if result.is_empty() {
        return String::new();
    }

    match fmt {
        Format::Pretty => format_pretty(result, color),
        Format::Json => format_json(result),
        Format::Plain => format_plain(result),
    }
}

fn format_pretty(result: &CheckResult, color: bool) -> String {
    let mut out = String::new();

    let (bold_red, bold_yellow, reset) = if color {
        ("\x1b[1;31m", "\x1b[1;33m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    for d in result {
        let (color, label) = match d.severity {
            Severity::Error => (bold_red, "error"),
            Severity::Warning => (bold_yellow, "warning"),
        };
        writeln!(
            out,
            "{}: {color}{label}{reset}: {}",
            d.location(),
            d.message,
        )
        .unwrap();
    }

    out
}

fn format_json(result: &CheckResult) -> String {
    #[derive(serde::Serialize)]
    struct JsonOutput<'a> {
        diagnostics: &'a [Diagnostic],
    }

    let output = JsonOutput {
        diagnostics: result,
    };

    serde_json::to_string_pretty(&output).unwrap()
}

fn format_plain(result: &CheckResult) -> String {
    let mut out = String::new();

    for d in result {
        if let Some(target) = &d.target {
            writeln!(
                out,
                "{}: {} [ThenChange('{}') at line {}]",
                d.location(),
                d.message,
                target.raw,
                target.then_change_line,
            )
            .unwrap();
        } else {
            writeln!(out, "{}: {}", d.location(), d.message).unwrap();
        }
    }

    out
}

#[cfg(test)]
#[path = "reports_test.rs"]
mod tests;
