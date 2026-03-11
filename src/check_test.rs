use super::*;
use crate::changes;
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;
use unindent::unindent;
use yare::parameterized;

// ─── Shared test infrastructure ───

/// Map-like syntax for file definitions: `files!{ "name" => "content", ... }`
macro_rules! files {
    ($($name:expr => $content:expr),* $(,)?) => {
        &[$(($name, $content)),*] as &[(&str, &str)]
    };
}

/// A single test case for `check()`.
///
/// - `expected_errors`:   `None` = skip, `Some(&[])` = assert empty, `Some(&["msg"])` = assert contains
/// - `expected_findings`: `None` = skip, `Some(&[])` = assert empty, `Some(&["msg"])` = assert contains
struct CheckCase {
    files: &'static [(&'static str, &'static str)],
    diff: Option<&'static str>,
    ignore_patterns: &'static [&'static str],
    expected_errors: Option<&'static [&'static str]>,
    expected_findings: Option<&'static [&'static str]>,
}

const DEFAULTS: CheckCase = CheckCase {
    files: &[],
    diff: None,
    ignore_patterns: &[],
    expected_errors: None,
    expected_findings: None,
};

fn run_case(case: &CheckCase) {
    // Unindent file contents
    let files: Vec<(String, String)> = case
        .files
        .iter()
        .map(|(path, content)| (path.to_string(), unindent(content)))
        .collect();
    let file_refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, c)| (p.as_str(), c.as_str()))
        .collect();

    // Set up temp directory
    let dir = TempDir::new().unwrap();
    for (path, content) in &file_refs {
        let p = dir.path().join(path);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
    }

    // Parse input
    let diff = case.diff.map(|d| unindent(d));
    let (change_map, content_hint) = match diff.as_deref() {
        Some(d) => (changes::from_diff(&mut Cursor::new(d)).unwrap(), None),
        None => {
            let (chgs, content) = changes::from_directory(dir.path());
            (chgs, Some(content))
        }
    };

    // Build ignore matchers
    let matchers: Vec<globset::GlobMatcher> = case
        .ignore_patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect();

    let result = check(&change_map, dir.path(), &matchers, content_hint.as_ref());

    // Assert parse errors
    if let Some(expected) = case.expected_errors {
        if expected.is_empty() {
            assert!(
                result.parse_errors.is_empty(),
                "expected no errors, got: {:?}",
                result.parse_errors
            );
        } else {
            for msg in expected {
                assert!(
                    result.parse_errors.iter().any(|e| e.message.contains(msg)),
                    "expected error containing '{}', got: {:?}",
                    msg,
                    result.parse_errors
                );
            }
        }
    }

    // Assert findings
    if let Some(expected) = case.expected_findings {
        if expected.is_empty() {
            assert!(
                result.findings.is_empty(),
                "expected no findings, got: {:?}",
                result.findings
            );
        } else {
            for msg in expected {
                assert!(
                    result.findings.iter().any(|f| f.message.contains(msg)),
                    "expected finding containing '{}', got: {:?}",
                    msg,
                    result.findings
                );
            }
        }
    }
}

// ─── Structural validation (scan mode — no diff) ───

#[parameterized(
    clean_file = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('//b.rs')
            ",
            "b.rs" => "fn b() {}\n",
        },
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    orphan_then = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.ThenChange(':x')
            ",
        },
        expected_errors: Some(&["without preceding IfChange"]),
        ..DEFAULTS
    } },
    unclosed_label = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.Label('x')
                code
            ",
        },
        expected_errors: Some(&["without matching EndLabel"]),
        ..DEFAULTS
    } },
    orphan_end = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.EndLabel
            ",
        },
        expected_errors: Some(&["without matching Label"]),
        ..DEFAULTS
    } },
    unclosed_if = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
            ",
        },
        expected_errors: Some(&["without matching ThenChange"]),
        ..DEFAULTS
    } },
    unknown_directive = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.FooBar
            ",
        },
        expected_errors: Some(&["unknown directive"]),
        ..DEFAULTS
    } },
    duplicate_if_label = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange('x')
                code
                // LINT.ThenChange('//y.rs')
                // LINT.IfChange('x')
                more
                // LINT.ThenChange('//z.rs')
            ",
            "y.rs" => "y\n",
            "z.rs" => "z\n",
        },
        expected_errors: Some(&["duplicate"]),
        ..DEFAULTS
    } },
    duplicate_label = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.Label('x')
                code
                // LINT.EndLabel
                // LINT.Label('x')
                more
                // LINT.EndLabel
            ",
        },
        expected_errors: Some(&["duplicate"]),
        ..DEFAULTS
    } },
)]
fn scan_structure(case: CheckCase) {
    run_case(&case);
}

// ─── Comment styles ───
//
// Each test uses diff mode (source changed, target untouched) so the assertion
// `expected_findings: Some(&["not modified"])` proves the directive was
// actually recognised — a test that only checks `expected_errors: Some(&[])`
// would pass even if the comment prefix were silently ignored.

#[parameterized(
    slash = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('//b.rs')
            ",
            "b.rs" => "x\n",
        },
        diff: Some("
            --- a/test.rs
            +++ b/test.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -code
            +code v2
             // LINT.ThenChange('//b.rs')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    hash = { CheckCase {
        files: files!{
            "test.py" => "
                # LINT.IfChange
                code
                # LINT.ThenChange('//b.py')
            ",
            "b.py" => "x\n",
        },
        diff: Some("
            --- a/test.py
            +++ b/test.py
            @@ -1,3 +1,3 @@
             # LINT.IfChange
            -code
            +code v2
             # LINT.ThenChange('//b.py')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    html = { CheckCase {
        files: files!{
            "test.html" => "
                <!-- LINT.IfChange -->
                code
                <!-- LINT.ThenChange('//b.html') -->
            ",
            "b.html" => "x\n",
        },
        diff: Some("
            --- a/test.html
            +++ b/test.html
            @@ -1,3 +1,3 @@
             <!-- LINT.IfChange -->
            -code
            +code v2
             <!-- LINT.ThenChange('//b.html') -->
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    dash = { CheckCase {
        files: files!{
            "test.sql" => "
                -- LINT.IfChange
                code
                -- LINT.ThenChange('//b.sql')
            ",
            "b.sql" => "x\n",
        },
        diff: Some("
            --- a/test.sql
            +++ b/test.sql
            @@ -1,3 +1,3 @@
             -- LINT.IfChange
            -code
            +code v2
             -- LINT.ThenChange('//b.sql')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    block = { CheckCase {
        files: files!{
            "test.js" => "
                /* LINT.IfChange */
                code
                /* LINT.ThenChange('//b.js') */
            ",
            "b.js" => "x\n",
        },
        diff: Some("
            --- a/test.js
            +++ b/test.js
            @@ -1,3 +1,3 @@
             /* LINT.IfChange */
            -code
            +code v2
             /* LINT.ThenChange('//b.js') */
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    semi = { CheckCase {
        files: files!{
            "test.lisp" => "
                ;; LINT.IfChange
                code
                ;; LINT.ThenChange('//b.lisp')
            ",
            "b.lisp" => "x\n",
        },
        diff: Some("
            --- a/test.lisp
            +++ b/test.lisp
            @@ -1,3 +1,3 @@
             ;; LINT.IfChange
            -code
            +code v2
             ;; LINT.ThenChange('//b.lisp')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    pct = { CheckCase {
        files: files!{
            "test.tex" => "
                % LINT.IfChange
                code
                % LINT.ThenChange('//b.tex')
            ",
            "b.tex" => "x\n",
        },
        diff: Some("
            --- a/test.tex
            +++ b/test.tex
            @@ -1,3 +1,3 @@
             % LINT.IfChange
            -code
            +code v2
             % LINT.ThenChange('//b.tex')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    bang = { CheckCase {
        files: files!{
            "test.f90" => "
                ! LINT.IfChange
                code
                ! LINT.ThenChange('//b.f90')
            ",
            "b.f90" => "x\n",
        },
        diff: Some("
            --- a/test.f90
            +++ b/test.f90
            @@ -1,3 +1,3 @@
             ! LINT.IfChange
            -code
            +code v2
             ! LINT.ThenChange('//b.f90')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    // Makefile and Dockerfile use filename-based detection (not extension),
    // so they warrant separate tests from the generic `hash` case above.
    makefile = { CheckCase {
        files: files!{
            "Makefile" => "
                # LINT.IfChange
                all:
                # LINT.ThenChange('//b.mk')
            ",
            "b.mk" => "x\n",
        },
        diff: Some("
            --- a/Makefile
            +++ b/Makefile
            @@ -1,3 +1,3 @@
             # LINT.IfChange
            -all:
            +all: v2
             # LINT.ThenChange('//b.mk')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    dockerfile = { CheckCase {
        files: files!{
            "Dockerfile" => "
                # LINT.IfChange
                FROM ubuntu
                # LINT.ThenChange('//b.dockerfile')
            ",
            "b.dockerfile" => "x\n",
        },
        diff: Some("
            --- a/Dockerfile
            +++ b/Dockerfile
            @@ -1,3 +1,3 @@
             # LINT.IfChange
            -FROM ubuntu
            +FROM ubuntu:22.04
             # LINT.ThenChange('//b.dockerfile')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    // .vue and .php each accept multiple comment styles; test each separately.
    vue_slash = { CheckCase {
        files: files!{
            "test.vue" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('//b.vue')
            ",
            "b.vue" => "x\n",
        },
        diff: Some("
            --- a/test.vue
            +++ b/test.vue
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -code
            +code v2
             // LINT.ThenChange('//b.vue')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    vue_html = { CheckCase {
        files: files!{
            "test.vue" => "
                <!-- LINT.IfChange -->
                code
                <!-- LINT.ThenChange('//b.vue') -->
            ",
            "b.vue" => "x\n",
        },
        diff: Some("
            --- a/test.vue
            +++ b/test.vue
            @@ -1,3 +1,3 @@
             <!-- LINT.IfChange -->
            -code
            +code v2
             <!-- LINT.ThenChange('//b.vue') -->
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    php_slash = { CheckCase {
        files: files!{
            "test.php" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('//b.php')
            ",
            "b.php" => "x\n",
        },
        diff: Some("
            --- a/test.php
            +++ b/test.php
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -code
            +code v2
             // LINT.ThenChange('//b.php')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    php_hash = { CheckCase {
        files: files!{
            "test.php" => "
                # LINT.IfChange
                code
                # LINT.ThenChange('//b.php')
            ",
            "b.php" => "x\n",
        },
        diff: Some("
            --- a/test.php
            +++ b/test.php
            @@ -1,3 +1,3 @@
             # LINT.IfChange
            -code
            +code v2
             # LINT.ThenChange('//b.php')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
)]
fn comment_styles(case: CheckCase) {
    run_case(&case);
}

// ─── Path security (scan mode) ───

#[parameterized(
    traversal = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('//foo/../etc/passwd')
            ",
        },
        expected_errors: Some(&["path traversal"]),
        ..DEFAULTS
    } },
    absolute = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('///etc/passwd')
            ",
        },
        expected_errors: Some(&["must be relative"]),
        ..DEFAULTS
    } },
    double_dot_in_filename_allowed = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange('//src/file..txt')
            ",
            "src/file..txt" => "x\n",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
)]
fn path_security(case: CheckCase) {
    run_case(&case);
}

// ─── Quoting and labels (scan mode) ───

#[parameterized(
    double_quoted_if_label = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange(\"my_label\")
                code
                // LINT.ThenChange('//b.rs')
            ",
            "b.rs" => "x\n",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    double_quoted_target = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(\"//b.rs\")
            ",
            "b.rs" => "x\n",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    mixed_quotes_array = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(['//a.rs', \"//b.rs\"])
            ",
            "a.rs" => "x\n",
            "b.rs" => "x\n",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    label_section = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(':section_2')
            ",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    label_hyphen = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(':my-label')
            ",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    multiline_then_change = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange([
                //   '//a.rs',
                //   '//b.rs'
                // ])
            ",
            "a.rs" => "x\n",
            "b.rs" => "x\n",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
)]
fn quoting_and_labels(case: CheckCase) {
    run_case(&case);
}

// ─── Non-comment context & binary files ───

#[test]
fn directive_in_string_literal_ignored() {
    run_case(&CheckCase {
        files: files! {
            "test.rs" => "let s = \"LINT.ThenChange('//foo.rs')\";\n",
        },
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    });
}

#[test]
fn binary_file_skipped() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("binary.rs");
    let mut content = b"// LINT.ThenChange('//x.rs')\n".to_vec();
    content.push(0);
    fs::write(&file, content).unwrap();

    let (changes, _) = changes::from_directory(dir.path());
    let result = check(&changes, dir.path(), &[], None);
    assert!(result.parse_errors.is_empty());
    assert!(result.findings.is_empty());
}

// ─── Cross-file validation (diff mode) ───

#[parameterized(
    target_modified_passes = { CheckCase {
        files: files!{
            "src/api.rs" => "
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange('//src/handler.rs')
            ",
            "src/handler.rs" => "fn handler() {}\n",
        },
        diff: Some("
            --- a/src/api.rs
            +++ b/src/api.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn api() {}
            +fn api() { v2 }
             // LINT.ThenChange('//src/handler.rs')
            --- a/src/handler.rs
            +++ b/src/handler.rs
            @@ -1 +1 @@
            -fn handler() {}
            +fn handler() { v2 }
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    target_not_modified = { CheckCase {
        files: files!{
            "src/api.rs" => "
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange('//src/handler.rs')
            ",
            "src/handler.rs" => "fn handler() {}\n",
        },
        diff: Some("
            --- a/src/api.rs
            +++ b/src/api.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn api() {}
            +fn api() { v2 }
             // LINT.ThenChange('//src/handler.rs')
        "),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    target_file_missing = { CheckCase {
        files: files!{
            "src/api.rs" => "
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange('//src/missing.rs')
            ",
        },
        diff: Some("
            --- a/src/api.rs
            +++ b/src/api.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn api() {}
            +fn api() { v2 }
             // LINT.ThenChange('//src/missing.rs')
        "),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    changes_outside_if_range_ignored = { CheckCase {
        files: files!{
            "src/api.rs" => "
                fn before() {}
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange('//src/handler.rs')
                fn after() {}
            ",
            "src/handler.rs" => "fn handler() {}\n",
        },
        diff: Some("
            --- a/src/api.rs
            +++ b/src/api.rs
            @@ -1,5 +1,5 @@
            -fn before() {}
            +fn before() { v2 }
             // LINT.IfChange
             fn api() {}
             // LINT.ThenChange('//src/handler.rs')
             fn after() {}
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    self_loop_without_label = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/a.rs')
            ",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/a.rs')
        "),
        expected_findings: Some(&["self-referencing"]),
        ..DEFAULTS
    } },
    removed_lines_trigger_if_change = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,2 @@
             // LINT.IfChange
            -fn f() {}
             // LINT.ThenChange('//src/b.rs')
        "),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    renamed_file_tracked = { CheckCase {
        files: files!{
            "src/old.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            "src/new.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        diff: Some("
            --- a/src/old.rs
            +++ b/src/new.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/b.rs')
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1 +1 @@
            -fn b() {}
            +fn b() { v2 }
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    multiple_blocks_only_triggered_one_fires = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f1() {}
                // LINT.ThenChange('//src/b.rs')
                fn gap() {}
                // LINT.IfChange
                fn f2() {}
                // LINT.ThenChange('//src/c.rs')
            ",
            "src/b.rs" => "fn b() {}\n",
            "src/c.rs" => "fn c() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,7 +1,7 @@
             // LINT.IfChange
            -fn f1() {}
            +fn f1() { v2 }
             // LINT.ThenChange('//src/b.rs')
             fn gap() {}
             // LINT.IfChange
             fn f2() {}
             // LINT.ThenChange('//src/c.rs')
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1 +1 @@
            -fn b() {}
            +fn b() { v2 }
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    same_file_label_reference = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(':section')
                // LINT.Label('section')
                fn target() {}
                // LINT.EndLabel
            ",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,6 +1,6 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange(':section')
             // LINT.Label('section')
            -fn target() {}
            +fn target() { v2 }
             // LINT.EndLabel
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
)]
fn cross_file_validation(case: CheckCase) {
    run_case(&case);
}

// ─── Label validation (diff mode) ───

#[parameterized(
    labeled_target_modified_passes = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs:section')
            ",
            "src/b.rs" => "
                fn before() {}
                // LINT.Label('section')
                fn target() {}
                // LINT.EndLabel
                fn after() {}
            ",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/b.rs:section')
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1,5 +1,5 @@
             fn before() {}
             // LINT.Label('section')
            -fn target() {}
            +fn target() { v2 }
             // LINT.EndLabel
             fn after() {}
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    change_outside_label_fails = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs:section')
            ",
            "src/b.rs" => "
                fn before() {}
                // LINT.Label('section')
                fn target() {}
                // LINT.EndLabel
                fn after() {}
            ",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/b.rs:section')
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1,5 +1,5 @@
            -fn before() {}
            +fn before() { v2 }
             // LINT.Label('section')
             fn target() {}
             // LINT.EndLabel
             fn after() {}
        "),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
    missing_label = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs:nonexistent')
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/b.rs:nonexistent')
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1 +1 @@
            -fn b() {}
            +fn b() { v2 }
        "),
        expected_findings: Some(&["not found"]),
        ..DEFAULTS
    } },
    implicit_label_from_if_change = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs:api')
            ",
            "src/b.rs" => "
                // LINT.IfChange('api')
                fn api() {}
                // LINT.ThenChange('//src/a.rs')
            ",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/b.rs:api')
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange('api')
            -fn api() {}
            +fn api() { v2 }
             // LINT.ThenChange('//src/a.rs')
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    nested_labels_outer_resolved = { CheckCase {
        files: files!{
            "source.rs" => "
                // LINT.IfChange
                fn src() {}
                // LINT.ThenChange('//target.rs:outer')
            ",
            "target.rs" => "
                // LINT.Label('outer')
                fn outer_start() {}
                // LINT.Label('inner')
                fn inner() {}
                // LINT.EndLabel
                fn outer_end() {}
                // LINT.EndLabel
            ",
        },
        diff: Some("
            --- a/source.rs
            +++ b/source.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn src() {}
            +fn src() { v2 }
             // LINT.ThenChange('//target.rs:outer')
            --- a/target.rs
            +++ b/target.rs
            @@ -1,7 +1,7 @@
             // LINT.Label('outer')
             fn outer_start() {}
             // LINT.Label('inner')
             fn inner() {}
             // LINT.EndLabel
            -fn outer_end() {}
            +fn outer_end() { v2 }
             // LINT.EndLabel
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
)]
fn label_validation(case: CheckCase) {
    run_case(&case);
}

// ─── Multiple targets (diff mode) ───

#[parameterized(
    all_modified_passes = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(['//src/b.rs', '//src/c.rs'])
            ",
            "src/b.rs" => "fn b() {}\n",
            "src/c.rs" => "fn c() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange(['//src/b.rs', '//src/c.rs'])
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1 +1 @@
            -fn b() {}
            +fn b() { v2 }
            --- a/src/c.rs
            +++ b/src/c.rs
            @@ -1 +1 @@
            -fn c() {}
            +fn c() { v2 }
        "),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    one_not_modified = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(['//src/b.rs', '//src/c.rs'])
            ",
            "src/b.rs" => "fn b() {}\n",
            "src/c.rs" => "fn c() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange(['//src/b.rs', '//src/c.rs'])
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1 +1 @@
            -fn b() {}
            +fn b() { v2 }
        "),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
)]
fn multiple_targets(case: CheckCase) {
    run_case(&case);
}

// ─── Ignore patterns (diff mode) ───

#[parameterized(
    skips_matching_target = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//generated/api.rs')
            ",
            "generated/api.rs" => "fn api() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//generated/api.rs')
        "),
        ignore_patterns: &["generated/*"],
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    does_not_affect_non_matching = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        diff: Some("
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/b.rs')
        "),
        ignore_patterns: &["generated/*"],
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    } },
)]
fn ignore_patterns(case: CheckCase) {
    run_case(&case);
}

// ─── Issue 1: validate_structure error reported on wrong line ───

#[test]
fn consecutive_if_change_error_on_first_line() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("test.rs"),
        unindent(
            "
            // LINT.IfChange
            code
            // LINT.IfChange
            more
            // LINT.ThenChange('//b.rs')
        ",
        ),
    )
    .unwrap();
    fs::write(dir.path().join("b.rs"), "x\n").unwrap();

    let (changes, _) = changes::from_directory(dir.path());
    let result = check(&changes, dir.path(), &[], None);

    let err = result
        .parse_errors
        .iter()
        .find(|e| {
            e.message.contains("without matching ThenChange") && e.message.contains("previous")
        })
        .expect("should have structural error for consecutive IfChange");
    assert_eq!(
        err.line.get(),
        1,
        "error should be on the first IfChange (line 1), got line {}",
        err.line,
    );
}

// ─── Issue 2: find_label_range cleared by non-matching IfChange ───

#[test]
fn find_label_range_not_cleared_by_non_matching_if_change() {
    // target.rs has two consecutive IfChange without ThenChange in between (malformed).
    // The first has the label we're looking for. The current code clears if_content_start
    // when it sees the second non-matching IfChange, so the label is incorrectly not found.
    run_case(&CheckCase {
        files: files! {
            "source.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//target.rs:wanted')
            ",
            "target.rs" => "
                // LINT.IfChange('wanted')
                target content
                // LINT.IfChange('other')
                other content
                // LINT.ThenChange('//z.rs')
                // LINT.ThenChange('//z.rs')
            ",
            "z.rs" => "z\n",
        },
        diff: Some(
            "
            --- a/source.rs
            +++ b/source.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//target.rs:wanted')
            --- a/target.rs
            +++ b/target.rs
            @@ -1,6 +1,6 @@
             // LINT.IfChange('wanted')
            -target content
            +target content v2
             // LINT.IfChange('other')
             other content
             // LINT.ThenChange('//z.rs')
             // LINT.ThenChange('//z.rs')
        ",
        ),
        expected_findings: Some(&[]),
        ..DEFAULTS
    });
}

// ─── Issue 4: block comment suffix leaks into directive parsing ───

#[test]
fn multiline_block_comment_then_change() {
    // Multi-line ThenChange in block comments: each line ends with */.
    // The */ suffix must be stripped so multi-line joining produces a clean directive.
    // Using diff mode (source changed, target untouched) to prove the directive
    // is correctly parsed end-to-end, not just that it doesn't crash.
    run_case(&CheckCase {
        files: files! {
            "test.js" => "
                /* LINT.IfChange */
                code
                /* LINT.ThenChange( */
                /* '//b.js') */
            ",
            "b.js" => "x\n",
        },
        diff: Some(
            "
            --- a/test.js
            +++ b/test.js
            @@ -1,4 +1,4 @@
             /* LINT.IfChange */
            -code
            +code v2
             /* LINT.ThenChange( */
             /* '//b.js') */
        ",
        ),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    });
}

#[test]
fn multiline_html_comment_then_change() {
    // Same as multiline_block_comment_then_change but for HTML comment suffixes.
    run_case(&CheckCase {
        files: files! {
            "test.html" => "
                <!-- LINT.IfChange -->
                code
                <!-- LINT.ThenChange( -->
                <!-- '//b.html') -->
            ",
            "b.html" => "x\n",
        },
        diff: Some(
            "
            --- a/test.html
            +++ b/test.html
            @@ -1,4 +1,4 @@
             <!-- LINT.IfChange -->
            -code
            +code v2
             <!-- LINT.ThenChange( -->
             <!-- '//b.html') -->
        ",
        ),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    });
}

// ─── Issue 6: parse error for missing file uses line 0 ───

#[test]
fn parse_error_for_missing_file_uses_line_1_not_0() {
    let dir = TempDir::new().unwrap();
    // Don't create "missing.rs" on disk but reference it in the diff
    let diff = unindent(
        "
        --- a/missing.rs
        +++ b/missing.rs
        @@ -1 +1 @@
        -old
        +new
    ",
    );
    let changes = changes::from_diff(&mut Cursor::new(diff)).unwrap();
    let result = check(&changes, dir.path(), &[], None);
    let err = result
        .parse_errors
        .iter()
        .find(|e| e.message.contains("failed to read"))
        .expect("should have read error for missing file");
    assert_eq!(
        err.line.get(),
        1,
        "file read error should use line 1, got {}",
        err.line,
    );
}

// ─── Bug: same-file reference must use :label syntax (not //file:label) ───

#[test]
fn same_file_with_full_path_and_label_is_rejected() {
    // ThenChange('//src/a.rs:section') inside src/a.rs should produce a finding.
    // The correct syntax is ThenChange(':section').
    // Currently this is silently treated as a cross-file label-range check on
    // the same file, producing no finding even when both regions are modified.
    run_case(&CheckCase {
        files: files! {
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//src/a.rs:section')
                // LINT.Label('section')
                fn g() {}
                // LINT.EndLabel
            ",
        },
        diff: Some(
            "
            --- a/src/a.rs
            +++ b/src/a.rs
            @@ -1,7 +1,7 @@
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//src/a.rs:section')
             // LINT.Label('section')
            -fn g() {}
            +fn g() { v2 }
             // LINT.EndLabel
        ",
        ),
        // Should produce a finding directing the user to use ':section' syntax.
        expected_findings: Some(&["':section'"]),
        ..DEFAULTS
    });
}

// ─── Bug: prose comment mentioning ThenChange should not trigger multiline mode ───

#[test]
fn prose_comment_mentioning_then_change_does_not_trigger_multiline() {
    // A comment like "// Use LINT.ThenChange to keep in sync." does NOT start with
    // "LINT.", so it should not trigger multiline collection. If the bug regresses,
    // collect_multiline_text consumes the following LINT.IfChange line, causing it
    // to be lost and producing a spurious "ThenChange without IfChange" error.
    //
    // Using diff mode so `expected_findings: Some(&["not modified"])` proves the
    // IfChange after the prose comment was correctly recognised and fires a
    // constraint check — a scan-mode `expected_findings: Some(&[])` assertion
    // would be vacuously true and catch nothing.
    run_case(&CheckCase {
        files: files! {
            "test.rs" => "
                // Use LINT.ThenChange to keep in sync.
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange('//b.rs')
            ",
            "b.rs" => "x\n",
        },
        diff: Some(
            "
            --- a/test.rs
            +++ b/test.rs
            @@ -1,4 +1,4 @@
             // Use LINT.ThenChange to keep in sync.
             // LINT.IfChange
            -fn f() {}
            +fn f() { v2 }
             // LINT.ThenChange('//b.rs')
        ",
        ),
        expected_errors: Some(&[]),
        expected_findings: Some(&["not modified"]),
        ..DEFAULTS
    });
}

// ─── Finding context ───

#[test]
fn finding_includes_if_change_label() {
    let src = unindent(
        "
        // LINT.IfChange('my_section')
        fn f() {}
        // LINT.ThenChange('//src/b.rs')
    ",
    );
    let diff = unindent(
        "
        --- a/src/a.rs
        +++ b/src/a.rs
        @@ -1,3 +1,3 @@
         // LINT.IfChange('my_section')
        -fn f() {}
        +fn f() { v2 }
         // LINT.ThenChange('//src/b.rs')
    ",
    );

    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.rs"), &src).unwrap();
    fs::write(dir.path().join("src/b.rs"), "fn b() {}\n").unwrap();

    let changes = changes::from_diff(&mut Cursor::new(&diff)).unwrap();
    let result = check(&changes, dir.path(), &[], None);
    assert_eq!(result.findings.len(), 1);
    // Assert the label is surfaced in the formatted location shown to users,
    // not just stored in an internal field.
    assert!(
        result.findings[0].source_location().contains("my_section"),
        "finding location should include the label name, got: {}",
        result.findings[0].source_location(),
    );
}
