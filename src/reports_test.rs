use std::num::NonZeroUsize;

use super::*;
use crate::check::{Diagnostic, Severity, TargetInfo};
use yare::parameterized;

fn result_with_error() -> CheckResult {
    vec![Diagnostic {
        file: "test.rs".to_string(),
        line: NonZeroUsize::new(1).unwrap(),
        severity: Severity::Error,
        message: "test error".to_string(),
        target: None,
    }]
}

fn result_with_finding() -> CheckResult {
    vec![Diagnostic {
        file: "src/a.rs".to_string(),
        line: NonZeroUsize::new(1).unwrap(),
        severity: Severity::Warning,
        message: "changes in this block may need to be reflected in src/b.rs".to_string(),
        target: Some(TargetInfo {
            label: None,
            raw: "//src/b.rs".to_string(),
            then_change_line: NonZeroUsize::new(3).unwrap(),
        }),
    }]
}

#[parameterized(
    empty_pretty = { Format::Pretty, Vec::new(), false },
    empty_json = { Format::Json, Vec::new(), false },
    empty_plain = { Format::Plain, Vec::new(), false },
    error_pretty = { Format::Pretty, result_with_error(), true },
    error_json = { Format::Json, result_with_error(), true },
    error_plain = { Format::Plain, result_with_error(), true },
    finding_pretty = { Format::Pretty, result_with_finding(), true },
    finding_json = { Format::Json, result_with_finding(), true },
    finding_plain = { Format::Plain, result_with_finding(), true },
)]
fn format_has_output(fmt: Format, result: CheckResult, has_output: bool) {
    let output = format(&result, fmt, false);
    assert_eq!(!output.is_empty(), has_output);
}

#[test]
fn pretty_no_color() {
    let output = format(&result_with_error(), Format::Pretty, false);
    assert!(output.contains("test.rs:1: error: test error"));
    assert!(!output.contains("\x1b["));
}

#[test]
fn pretty_with_color() {
    let output = format(&result_with_error(), Format::Pretty, true);
    assert!(output.contains("\x1b[1;31merror\x1b[0m"));
}

#[test]
fn json_contains_structure() {
    let output = format(&result_with_finding(), Format::Json, false);
    assert!(output.contains("\"diagnostics\""));
    assert!(output.contains("may need to be reflected"));
}

#[test]
fn plain_format() {
    let output = format(&result_with_error(), Format::Plain, false);
    assert!(output.contains("test.rs:1: test error"));
}

#[test]
fn finding_with_label_omits_label_from_location() {
    let result: CheckResult = vec![Diagnostic {
        file: "src/a.rs".to_string(),
        line: NonZeroUsize::new(1).unwrap(),
        severity: Severity::Warning,
        message: "test".to_string(),
        target: Some(TargetInfo {
            label: Some("my_label".to_string()),
            raw: "//src/b.rs".to_string(),
            then_change_line: NonZeroUsize::new(3).unwrap(),
        }),
    }];
    let output = format(&result, Format::Pretty, false);
    assert!(output.contains("src/a.rs:1: warning: test"));
    assert!(!output.contains("my_label"));
}
