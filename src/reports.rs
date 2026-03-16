use std::fmt::Write;

use crate::check::{CheckResult, Finding, ParseError};

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum Format {
    Pretty,
    Json,
    Plain,
}

/// Returns empty string if no errors.
pub fn format(result: &CheckResult, fmt: Format, color: bool) -> String {
    let total = result.parse_errors.len() + result.findings.len();
    if total == 0 {
        return String::new();
    }

    match fmt {
        Format::Pretty => format_pretty(result, color),
        Format::Json => format_json(result),
        Format::Plain => format_plain(result),
    }
}

// ─── Private helpers ───

fn format_pretty(result: &CheckResult, color: bool) -> String {
    let mut out = String::new();

    let (bold_red, bold_yellow, reset) = if color {
        ("\x1b[1;31m", "\x1b[1;33m", "\x1b[0m")
    } else {
        ("", "", "")
    };

    for err in &result.parse_errors {
        writeln!(
            out,
            "{}:{}: {bold_red}error{reset}: {}",
            err.file, err.line, err.message,
        )
        .unwrap();
    }

    for finding in &result.findings {
        writeln!(
            out,
            "{}: {bold_yellow}warning{reset}: {}",
            finding.source_location(),
            finding.message,
        )
        .unwrap();
    }

    out
}

fn format_json(result: &CheckResult) -> String {
    #[derive(serde::Serialize)]
    struct JsonOutput<'a> {
        parse_errors: &'a [ParseError],
        findings: &'a [Finding],
    }

    let output = JsonOutput {
        parse_errors: &result.parse_errors,
        findings: &result.findings,
    };

    serde_json::to_string_pretty(&output).unwrap()
}

fn format_plain(result: &CheckResult) -> String {
    let mut out = String::new();

    for err in &result.parse_errors {
        writeln!(out, "{}:{}: {}", err.file, err.line, err.message).unwrap();
    }

    for finding in &result.findings {
        writeln!(
            out,
            "{}: {} [ThenChange('{}') at line {}]",
            finding.source_location(),
            finding.message,
            finding.target_raw,
            finding.then_change_line,
        )
        .unwrap();
    }

    out
}

#[cfg(test)]
#[path = "reports_test.rs"]
mod tests;
