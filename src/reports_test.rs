use std::num::NonZeroUsize;

use super::*;
use yare::parameterized;

fn result_with_error() -> CheckResult {
    CheckResult {
        parse_errors: vec![ParseError {
            file: "test.rs".to_string(),
            line: NonZeroUsize::new(1).unwrap(),
            message: "test error".to_string(),
        }],
        findings: vec![],
    }
}

fn result_with_finding() -> CheckResult {
    CheckResult {
        parse_errors: vec![],
        findings: vec![Finding {
            source_file: "src/a.rs".to_string(),
            source_line: NonZeroUsize::new(1).unwrap(),
            source_label: None,
            target_raw: "//src/b.rs".to_string(),
            then_change_line: NonZeroUsize::new(3).unwrap(),
            message: "target src/b.rs was not modified".to_string(),
        }],
    }
}

#[parameterized(
    empty_pretty = { Format::Pretty, CheckResult::default(), false },
    empty_json = { Format::Json, CheckResult::default(), false },
    empty_plain = { Format::Plain, CheckResult::default(), false },
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
    assert!(output.contains("error: test error"));
    assert!(output.contains("--> test.rs:1"));
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
    assert!(output.contains("\"findings\""));
    assert!(output.contains("\"parse_errors\""));
    assert!(output.contains("not modified"));
}

#[test]
fn plain_format() {
    let output = format(&result_with_error(), Format::Plain, false);
    assert!(output.contains("test.rs:1: test error"));
}

#[test]
fn finding_with_label_shows_label() {
    let result = CheckResult {
        parse_errors: vec![],
        findings: vec![Finding {
            source_file: "src/a.rs".to_string(),
            source_line: NonZeroUsize::new(1).unwrap(),
            source_label: Some("my_label".to_string()),
            target_raw: "//src/b.rs".to_string(),
            then_change_line: NonZeroUsize::new(3).unwrap(),
            message: "test".to_string(),
        }],
    };
    let output = format(&result, Format::Pretty, false);
    assert!(output.contains("label 'my_label'"));
}
