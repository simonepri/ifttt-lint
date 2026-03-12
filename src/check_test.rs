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
    /// Root-relative file paths to validate structurally (structural validity pass).
    file_list: &'static [&'static str],
    ignore_patterns: &'static [&'static str],
    expected_errors: Option<&'static [&'static str]>,
    expected_findings: Option<&'static [&'static str]>,
    /// Exact number of findings expected. `None` skips the count check.
    expected_finding_count: Option<usize>,
}

const DEFAULTS: CheckCase = CheckCase {
    files: &[],
    diff: None,
    file_list: &[],
    ignore_patterns: &[],
    expected_errors: None,
    expected_findings: None,
    expected_finding_count: None,
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
    let diff = case.diff.map(unindent);
    let change_map = match diff.as_deref() {
        Some(d) => changes::from_diff(&mut Cursor::new(d)).unwrap(),
        None => {
            let (chgs, _) = changes::from_directory(dir.path());
            chgs
        }
    };

    // Build ignore matchers
    let matchers: Vec<globset::GlobMatcher> = case
        .ignore_patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect();

    let file_list: Vec<String> = case.file_list.iter().map(|s| s.to_string()).collect();

    let result = check(&change_map, dir.path(), &matchers, &file_list);

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
                // "source:<pattern>" matches source_file; everything else matches message.
                let (field, pattern) = msg
                    .strip_prefix("source:")
                    .map(|p| ("source_file", p))
                    .unwrap_or(("message", *msg));
                assert!(
                    result.findings.iter().any(|f| {
                        if field == "source_file" {
                            f.source_file.contains(pattern)
                        } else {
                            f.message.contains(pattern)
                        }
                    }),
                    "expected finding with {field} containing '{pattern}', got: {:?}",
                    result.findings
                );
            }
        }
    }

    // Assert exact finding count (when specified)
    if let Some(count) = case.expected_finding_count {
        assert_eq!(
            result.findings.len(),
            count,
            "expected exactly {count} finding(s), got: {:?}",
            result.findings
        );
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
    let result = check(&changes, dir.path(), &[], &[]);
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
        expected_findings: Some(&["target file not found"]),
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

// ─── Reverse lookup: deleted target referenced by unchanged file ───

#[parameterized(
    // A points to B via ThenChange; B is deleted; A is unchanged.
    // The reverse lookup should find A and emit a finding.
    target_deleted_referenced_by_unchanged_file = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            // src/b.rs is absent — it was deleted
        },
        diff: Some("
            --- a/src/b.rs
            +++ /dev/null
            @@ -1,3 +0,0 @@
            -// LINT.IfChange
            -fn b() {}
            -// LINT.ThenChange('//src/a.rs')
        "),
        expected_findings: Some(&["target file not found: src/b.rs"]),
        ..DEFAULTS
    } },
    // A and B reference each other; A is deleted; B is unchanged.
    // The reverse lookup should find B's dangling reference to A.
    circular_reference_deleted_side = { CheckCase {
        files: files!{
            "src/b.rs" => "
                // LINT.IfChange
                fn b() {}
                // LINT.ThenChange('//src/a.rs')
            ",
            // src/a.rs is absent — it was deleted
        },
        diff: Some("
            --- a/src/a.rs
            +++ /dev/null
            @@ -1,3 +0,0 @@
            -// LINT.IfChange
            -fn a() {}
            -// LINT.ThenChange('//src/b.rs')
        "),
        expected_findings: Some(&["target file not found: src/a.rs"]),
        ..DEFAULTS
    } },
    // A points to B:section via ThenChange; B is deleted; A is unchanged.
    // The finding should include the label name.
    deleted_target_with_label = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//src/b.rs:section')
            ",
            // src/b.rs is absent — it was deleted
        },
        diff: Some("
            --- a/src/b.rs
            +++ /dev/null
            @@ -1,3 +0,0 @@
            -// LINT.IfChange
            -fn b() {}
            -// LINT.ThenChange('//src/a.rs')
        "),
        expected_findings: Some(&["target file not found: src/b.rs (label 'section')"]),
        ..DEFAULTS
    } },
    // No deletion in the diff — reverse lookup must not run / produce findings.
    no_deletion_no_reverse_lookup = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        diff: Some("
            --- a/src/b.rs
            +++ b/src/b.rs
            @@ -1 +1 @@
            -fn b() {}
            +fn b() { v2 }
        "),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    // src/a.rs is in file_list and points to src/b.rs which was deleted.
    // The structural pass catches it; the reverse-lookup skips file_list files.
    // Exactly one finding expected (no duplicate).
    file_list_catches_deleted_target_structurally = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//src/b.rs')
            ",
            // src/b.rs is absent — it was deleted
        },
        diff: Some("
            --- a/src/b.rs
            +++ /dev/null
            @@ -1 +0,0 @@
            -fn b() {}
        "),
        file_list: &["src/a.rs"],
        expected_findings: Some(&["target file not found: src/b.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    // Same as above but the ThenChange includes a label. Both the finding message
    // and the exact count are checked.
    file_list_catches_deleted_target_with_label = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//src/b.rs:api')
            ",
            // src/b.rs is absent — it was deleted
        },
        diff: Some("
            --- a/src/b.rs
            +++ /dev/null
            @@ -1 +0,0 @@
            -fn b() {}
        "),
        file_list: &["src/a.rs"],
        expected_findings: Some(&["target file not found: src/b.rs (label 'api')"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
)]
fn deleted_target_reverse_lookup(case: CheckCase) {
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
    let result = check(&changes, dir.path(), &[], &[]);

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

// ─── Issue 6: modified-but-missing file produces a parse error ───
//
// A file that appears as modified (not deleted) in the diff but does not
// exist on disk indicates a stale patch or race condition.  This is reported
// as a parse error.  Deleted files (new path = /dev/null) are expected to be
// absent and are silently skipped.

#[test]
fn modified_but_missing_file_produces_parse_error() {
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
    let result = check(&changes, dir.path(), &[], &[]);
    let err = result
        .parse_errors
        .iter()
        .find(|e| e.message.contains("not found on disk"))
        .expect("should have error for modified-but-missing file");
    assert_eq!(
        err.line.get(),
        1,
        "missing-file error should use line 1, got {}",
        err.line,
    );
}

#[test]
fn deleted_file_missing_from_disk_produces_no_error() {
    let dir = TempDir::new().unwrap();
    let diff = unindent(
        "
        --- a/deleted.rs
        +++ /dev/null
        @@ -1 +0,0 @@
        -old
    ",
    );
    let changes = changes::from_diff(&mut Cursor::new(diff)).unwrap();
    let result = check(&changes, dir.path(), &[], &[]);
    assert!(
        result.parse_errors.is_empty(),
        "expected no parse errors for a deleted file, got: {:?}",
        result.parse_errors,
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

// ─── Structural validity pass (file_list) ───

#[parameterized(
    // File in list with ThenChange → missing target file → finding "target file not found".
    // Uses an empty diff so a.rs is NOT in the change map; only the structural pass fires.
    missing_target_file = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//missing.rs')
            ",
        },
        diff: Some(""),
        file_list: &["a.rs"],
        expected_findings: Some(&["target file not found: missing.rs"]),
        ..DEFAULTS
    } },
    // File in list with ThenChange → missing label in existing target → finding "label not found".
    // Uses an empty diff so a.rs is NOT in the change map; only the structural pass fires.
    missing_label = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//b.rs:nonexistent')
            ",
            "b.rs" => "fn b() {}\n",
        },
        diff: Some(""),
        file_list: &["a.rs"],
        expected_findings: Some(&["label 'nonexistent' not found in b.rs"]),
        ..DEFAULTS
    } },
    // File in list with ThenChange → valid target file and label → no finding.
    // Uses an empty diff so a.rs is NOT in the change map; only the structural pass fires.
    valid_target = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//b.rs:section')
            ",
            "b.rs" => "
                // LINT.Label('section')
                fn b() {}
                // LINT.EndLabel
            ",
        },
        diff: Some(""),
        file_list: &["a.rs"],
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    // File in list but itself missing on disk → parse error reported.
    file_itself_missing = { CheckCase {
        files: files!{},
        diff: Some(""),
        file_list: &["nonexistent.rs"],
        expected_errors: Some(&["file not found on disk"]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    // File in file_list AND triggered by diff → exactly one finding (no duplicate).
    // The structural pass skips triggered pairs, so the diff pass is the sole source.
    no_duplicate_when_in_diff_and_file_list = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//b.rs')
            ",
            "b.rs" => "fn b() {}\n",
        },
        diff: Some("
            --- a/a.rs
            +++ b/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn a() {}
            +fn a() { v2 }
             // LINT.ThenChange('//b.rs')
        "),
        file_list: &["a.rs"],
        expected_findings: Some(&["not modified"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    // Same-file :label reference where label exists → no finding.
    same_file_label_exists = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(':section')
                // LINT.Label('section')
                fn b() {}
                // LINT.EndLabel
            ",
        },
        diff: Some(""),
        file_list: &["a.rs"],
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    // Same-file :label reference where label does NOT exist → finding.
    same_file_label_missing = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(':nonexistent')
            ",
        },
        diff: Some(""),
        file_list: &["a.rs"],
        expected_findings: Some(&["label 'nonexistent' not found in a.rs"]),
        ..DEFAULTS
    } },
    // Reverse lookup scoped to non-file_list files.
    // B is deleted; A (in file_list) gets a finding from the structural pass,
    // C (not in file_list) gets a finding from the reverse lookup — total = 2.
    reverse_lookup_scoped_to_non_file_list = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//b.rs')
            ",
            "c.rs" => "
                // LINT.IfChange
                fn c() {}
                // LINT.ThenChange('//b.rs')
            ",
            // b.rs is absent — it was deleted
        },
        diff: Some("
            --- a/b.rs
            +++ /dev/null
            @@ -1,3 +0,0 @@
            -// LINT.IfChange
            -fn b() {}
            -// LINT.ThenChange('//a.rs')
        "),
        file_list: &["a.rs"],
        // Both a.rs (structural pass) and c.rs (reverse lookup) should report a
        // dangling reference to b.rs.  Match source_file to verify both emitters.
        expected_findings: Some(&["source:a.rs", "source:c.rs"]),
        expected_finding_count: Some(2),
        ..DEFAULTS
    } },
)]
fn structural_validity_pass(case: CheckCase) {
    run_case(&case);
}

// ─── Combined --diff + [FILES]... mode ───

#[parameterized(
    // Diff pass and structural pass each emit one finding on different files;
    // total must be exactly 2 with no cross-contamination.
    diff_and_structural_distinct_findings = { CheckCase {
        files: files!{
            // a.rs: in file_list only, points at a file that doesn't exist.
            // Structural pass should emit one finding.
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//missing_structural.rs')
            ",
            // b.rs: modified in diff, points at c.rs which exists but is not modified.
            // Diff pass should emit one finding.
            "b.rs" => "
                // LINT.IfChange
                fn b() {}
                // LINT.ThenChange('//c.rs')
            ",
            "c.rs" => "fn c() {}\n",
        },
        diff: Some("
            --- a/b.rs
            +++ b/b.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn b() {}
            +fn b() { v2 }
             // LINT.ThenChange('//c.rs')
        "),
        file_list: &["a.rs"],
        expected_findings: Some(&["source:a.rs", "source:b.rs"]),
        expected_finding_count: Some(2),
        ..DEFAULTS
    } },
    // Both passes fire on the same file; finding must appear exactly once.
    diff_and_structural_same_file_no_duplicate = { CheckCase {
        files: files!{
            // a.rs: in file_list AND modified in diff; b.rs doesn't exist.
            // Only one finding expected regardless of how many passes touch a.rs.
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange('//missing.rs')
            ",
        },
        diff: Some("
            --- a/a.rs
            +++ b/a.rs
            @@ -1,3 +1,3 @@
             // LINT.IfChange
            -fn a() {}
            +fn a() { v2 }
             // LINT.ThenChange('//missing.rs')
        "),
        file_list: &["a.rs"],
        expected_findings: Some(&["missing.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
)]
fn combined_diff_and_file_list(case: CheckCase) {
    run_case(&case);
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
    let result = check(&changes, dir.path(), &[], &[]);
    assert_eq!(result.findings.len(), 1);
    // Assert the label is surfaced in the formatted location shown to users,
    // not just stored in an internal field.
    assert!(
        result.findings[0].source_location().contains("my_section"),
        "finding location should include the label name, got: {}",
        result.findings[0].source_location(),
    );
}
