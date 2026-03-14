use super::*;

#[test]
fn parse_target_file_and_label() {
    let t = parse_target("//src/lib.rs:my_label").unwrap();
    assert_eq!(t.file.as_deref(), Some("//src/lib.rs"));
    assert_eq!(t.label.as_deref(), Some("my_label"));
}

#[test]
fn parse_target_file_only() {
    let t = parse_target("//src/lib.rs").unwrap();
    assert_eq!(t.file.as_deref(), Some("//src/lib.rs"));
    assert_eq!(t.label, None);
}

#[test]
fn parse_target_label_only() {
    let t = parse_target(":my_label").unwrap();
    assert_eq!(t.file, None);
    assert_eq!(t.label.as_deref(), Some("my_label"));
}

#[test]
fn parse_target_windows_drive_letter_no_label() {
    // `C:\foo\bar` should NOT treat `C:` as file + label separator.
    let t = parse_target("C:\\foo\\bar").unwrap();
    assert_eq!(t.file.as_deref(), Some("C:\\foo\\bar"));
    assert_eq!(t.label, None);
}

#[test]
fn parse_target_windows_drive_letter_with_prefix() {
    // `//C:\foo` should NOT split on the drive-letter colon.
    let t = parse_target("//C:\\foo").unwrap();
    assert_eq!(t.file.as_deref(), Some("//C:\\foo"));
    assert_eq!(t.label, None);
}

#[test]
fn parse_target_windows_drive_letter_forward_slash() {
    let t = parse_target("C:/foo/bar").unwrap();
    assert_eq!(t.file.as_deref(), Some("C:/foo/bar"));
    assert_eq!(t.label, None);
}

#[test]
fn parse_target_drive_letter_with_label() {
    // `C:\foo:label` — rightmost colon is the label separator.
    let t = parse_target("C:\\foo:label").unwrap();
    assert_eq!(t.file.as_deref(), Some("C:\\foo"));
    assert_eq!(t.label.as_deref(), Some("label"));
}

#[test]
fn parse_target_colon_in_numeric_suffix_not_label() {
    // A colon followed by a digit is not a label separator.
    let t = parse_target("//file.txt:42").unwrap();
    assert_eq!(t.file.as_deref(), Some("//file.txt:42"));
    assert_eq!(t.label, None);
}

#[test]
fn parse_target_port_in_path_no_label() {
    // A colon in a directory component (e.g. port) must not be
    // mistaken for a label separator.
    let t = parse_target("//host:8080/api.rs").unwrap();
    assert_eq!(t.file.as_deref(), Some("//host:8080/api.rs"));
    assert_eq!(t.label, None);
}

#[test]
fn parse_target_port_in_path_with_label() {
    let t = parse_target("//host:8080/api.rs:my_label").unwrap();
    assert_eq!(t.file.as_deref(), Some("//host:8080/api.rs"));
    assert_eq!(t.label.as_deref(), Some("my_label"));
}

#[test]
fn then_change_with_parens_in_path() {
    let body = "LINT.ThenChange(//path(1).rs)";
    let targets = parse_then_change_targets(body).unwrap().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].file.as_deref(), Some("//path(1).rs"));
}

#[test]
fn then_change_with_parens_in_multiple_targets() {
    let body = "LINT.ThenChange(//a(1).rs, //b(2).rs)";
    let targets = parse_then_change_targets(body).unwrap().unwrap();
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].file.as_deref(), Some("//a(1).rs"));
    assert_eq!(targets[1].file.as_deref(), Some("//b(2).rs"));
}

#[test]
fn then_change_with_malformed_label_reports_error() {
    let content = "// LINT.IfChange\ncode\n// LINT.ThenChange(//b.rs:bad*label)\n";
    let (_, errors) = parse(content, "test.rs");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("malformed label \"bad*label\"")),
        "expected malformed label error, got: {errors:?}"
    );
}

// Full-pipeline tests: exercise parse() -> scan -> parse_directives (including
// is_prose_mention) rather than calling parse_then_change_targets directly.

#[test]
fn unclosed_paren_then_change_is_directive_not_prose() {
    let content = "// LINT.IfChange\ncode\n// LINT.ThenChange( // trailing comment\n";
    let (_, errors) = parse(content, "test.rs");
    assert!(
        errors
            .iter()
            .any(|e| e.message.contains("malformed directive")),
        "expected malformed directive error, got: {errors:?}"
    );
}

#[test]
fn parens_in_path_not_treated_as_prose() {
    let content = "// LINT.IfChange\ncode\n// LINT.ThenChange(//path(1).rs)\n";
    let (directives, errors) = parse(content, "test.rs");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(directives.len(), 2);
    match &directives[1] {
        Directive::ThenChange { targets, .. } => {
            assert_eq!(targets.len(), 1);
            assert_eq!(targets[0].file.as_deref(), Some("//path(1).rs"));
        }
        other => panic!("expected ThenChange, got {other:?}"),
    }
}

#[test]
fn parens_in_multiple_targets_not_treated_as_prose() {
    let content = "// LINT.IfChange\ncode\n// LINT.ThenChange(//a(1).rs, //b(2).rs)\n";
    let (directives, errors) = parse(content, "test.rs");
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    assert_eq!(directives.len(), 2);
    match &directives[1] {
        Directive::ThenChange { targets, .. } => {
            assert_eq!(targets.len(), 2);
            assert_eq!(targets[0].file.as_deref(), Some("//a(1).rs"));
            assert_eq!(targets[1].file.as_deref(), Some("//b(2).rs"));
        }
        other => panic!("expected ThenChange, got {other:?}"),
    }
}

#[test]
fn prose_after_parens_in_path_still_detected() {
    let content = "// LINT.ThenChange(//path(1).rs) marks the end\n";
    let (directives, errors) = parse(content, "test.rs");
    assert!(
        directives.is_empty(),
        "prose should not produce directives: {directives:?}"
    );
    assert!(
        errors.is_empty(),
        "prose should not produce errors: {errors:?}"
    );
}
