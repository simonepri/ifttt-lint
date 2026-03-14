use super::*;
use crate::vcs_mock::MockVcsProvider;
use unindent::unindent;
use yare::parameterized;

/// Map-like syntax for file definitions: `files!{ "name" => "content", ... }`
macro_rules! files {
    ($($name:expr => $content:expr),* $(,)?) => {
        &[$(($name, $content)),*] as &[(&str, &str)]
    };
}

/// Per-file change specification: `(path, added_lines, removed_new_positions)`.
///
/// `removed_lines` is populated with the same values as `removed_new_positions`
/// (they coincide for the simple single-hunk diffs used in tests).
type TestChange = (&'static str, &'static [usize], &'static [usize]);

/// Shorthand for a substitution: same lines appear in both added and removed sets.
const fn sub(path: &'static str, lines: &'static [usize]) -> TestChange {
    (path, lines, lines)
}

/// - `expected_errors`:   `None` = skip, `Some(&[])` = assert empty, `Some(&["msg"])` = assert contains
/// - `expected_findings`: `None` = skip, `Some(&[])` = assert empty, `Some(&["msg"])` = assert contains
struct CheckCase {
    files: &'static [(&'static str, &'static str)],
    /// `None` = scan mode (every line of every file is "added").
    changes: Option<&'static [TestChange]>,
    deleted: &'static [&'static str],
    /// Root-relative file paths to validate structurally (structural validity pass).
    file_list: &'static [&'static str],
    ignore_patterns: &'static [&'static str],
    strict: bool,
    expected_errors: Option<&'static [&'static str]>,
    expected_findings: Option<&'static [&'static str]>,
    /// Exact number of findings expected. `None` skips the count check.
    expected_finding_count: Option<usize>,
}

const DEFAULTS: CheckCase = CheckCase {
    files: &[],
    changes: None,
    deleted: &[],
    file_list: &[],
    ignore_patterns: &[],
    strict: true,
    expected_errors: None,
    expected_findings: None,
    expected_finding_count: None,
};

fn run_case(case: &CheckCase) {
    let mut mock = MockVcsProvider::default();
    mock.set_strict(case.strict);
    let files: Vec<(String, String)> = case
        .files
        .iter()
        .map(|(path, content)| (path.to_string(), unindent(content)))
        .collect();
    for (path, content) in &files {
        mock.add_file(path, content);
    }

    let change_map: ChangeMap = match case.changes {
        Some(changes) => {
            let mut map: ChangeMap = changes
                .iter()
                .map(|(path, added, removed_new)| {
                    (
                        path.to_string(),
                        FileChanges {
                            added_lines: added.iter().copied().collect(),
                            removed_lines: removed_new.iter().copied().collect(),
                            removed_new_positions: removed_new.iter().copied().collect(),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            for &path in case.deleted {
                map.insert(
                    path.to_string(),
                    FileChanges {
                        deleted: true,
                        ..Default::default()
                    },
                );
            }
            map
        }
        None => files
            .iter()
            .filter_map(|(path, content)| {
                let line_count = content.lines().count();
                if line_count == 0 {
                    return None;
                }
                Some((
                    path.clone(),
                    FileChanges {
                        added_lines: (1..=line_count).collect(),
                        ..Default::default()
                    },
                ))
            })
            .collect(),
    };

    let matchers: Vec<globset::GlobMatcher> = case
        .ignore_patterns
        .iter()
        .filter_map(|p| globset::Glob::new(p).ok().map(|g| g.compile_matcher()))
        .collect();

    mock.set_validate_files(case.file_list);

    let result = check(&mock, &change_map, &matchers);

    let errors: Vec<_> = result
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    let warnings: Vec<_> = result
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();

    if let Some(expected) = case.expected_errors {
        if expected.is_empty() {
            assert!(errors.is_empty(), "expected no errors, got: {:?}", errors);
        } else {
            for msg in expected {
                assert!(
                    errors.iter().any(|e| e.message.contains(msg)),
                    "expected error containing '{}', got: {:?}",
                    msg,
                    errors
                );
            }
        }
    }

    if let Some(expected) = case.expected_findings {
        if expected.is_empty() {
            assert!(
                warnings.is_empty(),
                "expected no findings, got: {:?}",
                warnings
            );
        } else {
            for msg in expected {
                let (field, pattern) = msg
                    .strip_prefix("source:")
                    .map(|p| ("file", p))
                    .unwrap_or(("message", *msg));
                assert!(
                    warnings.iter().any(|f| {
                        if field == "file" {
                            f.file.contains(pattern)
                        } else {
                            f.message.contains(pattern)
                        }
                    }),
                    "expected finding with {field} containing '{pattern}', got: {:?}",
                    warnings
                );
            }
        }
    }

    if let Some(count) = case.expected_finding_count {
        assert_eq!(
            warnings.len(),
            count,
            "expected exactly {count} finding(s), got: {:?}",
            warnings
        );
    }
}

// What comment styles does the tool parse?
// Each case uses diff mode (source changed, target untouched) so
// `expected_findings: Some(&["may need to be reflected in"])` proves the directive was
// recognised — a scan-mode assertion would pass even if the syntax were ignored.

#[parameterized(
    c_style_slash_slash = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    hash_comment = { CheckCase {
        files: files!{
            "test.py" => "
                # LINT.IfChange
                code
                # LINT.ThenChange(//b.py)
            ",
            "b.py" => "x\n",
        },
        changes: Some(const { &[sub("test.py", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    html_comment = { CheckCase {
        files: files!{
            "test.html" => "
                <!-- LINT.IfChange -->
                code
                <!-- LINT.ThenChange(//b.html) -->
            ",
            "b.html" => "x\n",
        },
        changes: Some(const { &[sub("test.html", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    sql_dash = { CheckCase {
        files: files!{
            "test.sql" => "
                -- LINT.IfChange
                code
                -- LINT.ThenChange(//b.sql)
            ",
            "b.sql" => "x\n",
        },
        changes: Some(const { &[sub("test.sql", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    block_comment = { CheckCase {
        files: files!{
            "test.js" => "
                /* LINT.IfChange */
                code
                /* LINT.ThenChange(//b.js) */
            ",
            "b.js" => "x\n",
        },
        changes: Some(const { &[sub("test.js", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    semicolon = { CheckCase {
        files: files!{
            "test.lisp" => "
                ; LINT.IfChange
                code
                ; LINT.ThenChange(//b.lisp)
            ",
            "b.lisp" => "x\n",
        },
        changes: Some(const { &[sub("test.lisp", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    percent = { CheckCase {
        files: files!{
            "test.tex" => "
                % LINT.IfChange
                code
                % LINT.ThenChange(//b.tex)
            ",
            "b.tex" => "x\n",
        },
        changes: Some(const { &[sub("test.tex", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    makefile = { CheckCase {
        files: files!{
            "Makefile" => "
                # LINT.IfChange
                all:
                # LINT.ThenChange(//b.mk)
            ",
            "b.mk" => "x\n",
        },
        changes: Some(const { &[sub("Makefile", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    dockerfile = { CheckCase {
        files: files!{
            "Dockerfile" => "
                # LINT.IfChange
                FROM ubuntu
                # LINT.ThenChange(//b.dockerfile)
            ",
            "b.dockerfile" => "x\n",
        },
        changes: Some(const { &[sub("Dockerfile", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    vue_slash = { CheckCase {
        files: files!{
            "test.vue" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//b.vue)
            ",
            "b.vue" => "x\n",
        },
        changes: Some(const { &[sub("test.vue", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    vue_html = { CheckCase {
        files: files!{
            "test.vue" => "
                <!-- LINT.IfChange -->
                code
                <!-- LINT.ThenChange(//b.vue) -->
            ",
            "b.vue" => "x\n",
        },
        changes: Some(const { &[sub("test.vue", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    php_slash = { CheckCase {
        files: files!{
            "test.php" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//b.php)
            ",
            "b.php" => "x\n",
        },
        changes: Some(const { &[sub("test.php", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    php_hash = { CheckCase {
        files: files!{
            "test.php" => "
                # LINT.IfChange
                code
                # LINT.ThenChange(//b.php)
            ",
            "b.php" => "x\n",
        },
        changes: Some(const { &[sub("test.php", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    haskell_block_comment = { CheckCase {
        files: files!{
            "test.hs" => "
                {- LINT.IfChange -}
                code = 42
                {- LINT.ThenChange(//b.hs) -}
            ",
            "b.hs" => "x\n",
        },
        changes: Some(const { &[sub("test.hs", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    sql_block_comment = { CheckCase {
        files: files!{
            "test.sql" => "
                /* LINT.IfChange */
                SELECT 1;
                /* LINT.ThenChange(//b.sql) */
            ",
            "b.sql" => "x\n",
        },
        changes: Some(const { &[sub("test.sql", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    directive_in_string_literal_ignored = { CheckCase {
        files: files! {
            "test.rs" => "let s = \"LINT.ThenChange(//foo.rs)\";
",
        },
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_multiline_string_ignored_rust = { CheckCase {
        files: files! {
            "test.rs" => r#"
                let s = "
                    // LINT.IfChange
                    some code
                    // LINT.ThenChange(//nonexistent.rs)
                ";
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_raw_string_ignored_rust = { CheckCase {
        files: files! {
            "test.rs" => r##"
                let s = r#"
                    // LINT.IfChange
                    some code
                    // LINT.ThenChange(//nonexistent.rs)
                "#;
            "##,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_triple_quote_ignored_python = { CheckCase {
        files: files! {
            "test.py" => r#"
                s = """
                # LINT.IfChange
                some code
                # LINT.ThenChange(//nonexistent.py)
                """
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_backtick_ignored_js = { CheckCase {
        files: files! {
            "test.js" => "
                const s = `
                    // LINT.IfChange
                    some code
                    // LINT.ThenChange(//nonexistent.js)
                `;
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_double_quote_ignored_c = { CheckCase {
        files: files! {
            "test.c" => r#"
                char *s = "// LINT.IfChange";
                char *t = "// LINT.ThenChange(//nonexistent.c)";
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_heredoc_ignored_shell = { CheckCase {
        files: files! {
            "test.sh" => "
                cat <<'EOF'
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.sh)
                EOF
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_heredoc_ignored_ruby = { CheckCase {
        files: files! {
            "test.rb" => "
                x = <<~HEREDOC
                  # LINT.IfChange
                  some text
                  # LINT.ThenChange(//nonexistent.rb)
                HEREDOC
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_heredoc_ignored_perl = { CheckCase {
        files: files! {
            "test.pl" => "
                my $x = <<END;
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.pl)
                END
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_heredoc_ignored_php = { CheckCase {
        files: files! {
            "test.php" => r#"
                $x = <<<EOT
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.php)
                EOT;
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_heredoc_ignored_hcl = { CheckCase {
        files: files! {
            "test.tf" => "
                variable = <<-EOT
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.tf)
                EOT
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_raw_string_ignored_cpp = { CheckCase {
        files: files! {
            "test.cpp" => r#"
                auto s = R"(
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.cpp)
                )";
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_verbatim_string_ignored_csharp = { CheckCase {
        files: files! {
            "test.cs" => r#"
                var s = @"
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.cs)
                ";
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_text_block_ignored_java = { CheckCase {
        files: files! {
            "test.java" => r#"
                String s = """
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.java)
                """;
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_multiline_string_ignored_swift = { CheckCase {
        files: files! {
            "test.swift" => r#"
                let s = """
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.swift)
                """
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_single_hash_extended_string_ignored_swift = { CheckCase {
        // `#"..."#` is a Swift extended string — the interior may contain
        // unescaped `"` characters, so the generic string handler can't
        // reliably skip it. Directives inside must still be ignored.
        files: files! {
            "test.swift" => r###"
                let s = #"
                she said "hello"
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.swift)
                "#
            "###,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_triple_quote_ignored_groovy = { CheckCase {
        files: files! {
            "test.groovy" => r#"
                def s = """
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.groovy)
                """
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_long_string_ignored_lua = { CheckCase {
        files: files! {
            "test.lua" => "
                local s = [[
                -- LINT.IfChange
                some text
                -- LINT.ThenChange(//nonexistent.lua)
                ]]
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_multiline_string_ignored_toml = { CheckCase {
        files: files! {
            "test.toml" => r#"
                x = """
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.toml)
                """
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_triple_quote_ignored_starlark = { CheckCase {
        files: files! {
            "test.bzl" => r#"
                x = """
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.bzl)
                """
            "#,
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_triple_single_quote_ignored_dart = { CheckCase {
        files: files! {
            "test.dart" => "
                var s = '''
                // LINT.IfChange
                some text
                // LINT.ThenChange(//nonexistent.dart)
                ''';
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_herestring_ignored_powershell = { CheckCase {
        files: files! {
            "test.ps1" => "
                $s = @\"
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.ps1)
                \"@
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    directive_in_multiline_string_ignored_nix = { CheckCase {
        files: files! {
            "test.nix" => "
                x = ''
                # LINT.IfChange
                some text
                # LINT.ThenChange(//nonexistent.nix)
                '';
            ",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    nix_indented_string_closed_before_letter = { CheckCase {
        // `''n` is NOT a Nix escape — `''` closes the string, `n` is code.
        // The real directive after the string must be recognised.
        files: files! {
            "test.nix" => "
                x = ''
                  inner
                ''n;
                # LINT.IfChange
                code
                # LINT.ThenChange(//b.nix)
            ",
            "b.nix" => "x\n",
        },
        changes: Some(const { &[sub("test.nix", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    real_directive_pairs_around_string_literal = { CheckCase {
        // Real IfChange, fake ThenChange inside string, real ThenChange after.
        // The scanner must skip the string so the real pair is correctly matched.
        files: files! {
            "test.rs" => r#"
                // LINT.IfChange
                let s = "
                    // LINT.ThenChange(//decoy.rs)
                ";
                real code
                // LINT.ThenChange(//b.rs)
            "#,
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    real_directive_pairs_around_triple_quote = { CheckCase {
        files: files! {
            "test.py" => r#"
                # LINT.IfChange
                s = """
                # LINT.ThenChange(//decoy.py)
                """
                real_code = 1
                # LINT.ThenChange(//b.py)
            "#,
            "b.py" => "x\n",
        },
        changes: Some(const { &[sub("test.py", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    real_directive_pairs_around_backtick = { CheckCase {
        files: files! {
            "test.js" => "
                // LINT.IfChange
                const s = `
                    // LINT.ThenChange(//decoy.js)
                `;
                real_code();
                // LINT.ThenChange(//b.js)
            ",
            "b.js" => "x\n",
        },
        changes: Some(const { &[sub("test.js", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    decoy_if_change_in_string_ignored = { CheckCase {
        // Real IfChange, fake IfChange inside string, real ThenChange after.
        // Without proper string skipping, the fake IfChange would trigger a
        // "consecutive IfChange" error on the real one.
        files: files! {
            "test.rs" => r#"
                // LINT.IfChange
                let s = "
                    // LINT.IfChange
                ";
                real code
                // LINT.ThenChange(//b.rs)
            "#,
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    decoy_if_change_in_triple_quote_ignored = { CheckCase {
        files: files! {
            "test.py" => r#"
                # LINT.IfChange
                s = """
                # LINT.IfChange
                """
                real_code = 1
                # LINT.ThenChange(//b.py)
            "#,
            "b.py" => "x\n",
        },
        changes: Some(const { &[sub("test.py", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    decoy_if_change_in_backtick_ignored = { CheckCase {
        files: files! {
            "test.js" => "
                // LINT.IfChange
                const s = `
                    // LINT.IfChange
                `;
                real_code();
                // LINT.ThenChange(//b.js)
            ",
            "b.js" => "x\n",
        },
        changes: Some(const { &[sub("test.js", &[5])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    escaped_quote_in_string_not_confused = { CheckCase {
        files: files! {
            "test.rs" => r#"
                let s = "she said \"// LINT.IfChange\" ok";
                // LINT.IfChange
                real code
                // LINT.ThenChange(//b.rs)
            "#,
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[3])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    multiline_then_change_block_comment = { CheckCase {
        files: files! {
            "test.js" => "
                /* LINT.IfChange */
                code
                /* LINT.ThenChange( */
                /* //b.js) */
            ",
            "b.js" => "x\n",
        },
        changes: Some(const { &[sub("test.js", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    multiline_then_change_html_comment = { CheckCase {
        files: files! {
            "test.html" => "
                <!-- LINT.IfChange -->
                code
                <!-- LINT.ThenChange( -->
                <!-- //b.html) -->
            ",
            "b.html" => "x\n",
        },
        changes: Some(const { &[sub("test.html", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    multiline_then_change_trailing_comma = { CheckCase {
        // A trailing comma in a multi-line ThenChange must not produce a
        // phantom empty target (which would fail path validation).
        files: files! {
            "a.toml" => "
                # LINT.IfChange(ver)
                version = \"1.0\"
                # LINT.ThenChange(
                #     //b.toml:ver,
                # )
            ",
            "b.toml" => "
                # LINT.IfChange(ver)
                version = \"1.0\"
                # LINT.ThenChange(
                #     //a.toml:ver,
                # )
            ",
        },
        changes: Some(const { &[sub("a.toml", &[2]), sub("b.toml", &[2])] }),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    backtick_mention_in_doc_comment_ignored = { CheckCase {
        // `LINT.ThenChange` inside backtick-quoted text in a doc comment must
        // not be treated as a directive (backtick is not comment decoration).
        files: files! {
            "test.rs" => "
                /// Returns the range between `LINT.IfChange` and its
                /// `LINT.ThenChange`, or `None` if missing.
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[4])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    repeated_slash_prefix_not_recognized = { CheckCase {
        // `///` is not the registered prefix `//` — must not match.
        files: files! {
            "test.rs" => "
                /// LINT.IfChange
                fn f() {}
                /// LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    repeated_hash_prefix_not_recognized = { CheckCase {
        // `##` is not the registered prefix `#` — must not match.
        files: files! {
            "test.py" => "
                ## LINT.IfChange
                x = 1
                ## LINT.ThenChange(//b.py)
            ",
            "b.py" => "x\n",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    repeated_dash_prefix_not_recognized = { CheckCase {
        // `---` is not the registered prefix `--` — must not match.
        files: files! {
            "test.sql" => "
                --- LINT.IfChange
                SELECT 1;
                --- LINT.ThenChange(//b.sql)
            ",
            "b.sql" => "x\n",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    repeated_semi_prefix_not_recognized = { CheckCase {
        // `;;` is not the registered prefix `;` — must not match.
        files: files! {
            "test.lisp" => "
                ;; LINT.IfChange
                code
                ;; LINT.ThenChange(//b.lisp)
            ",
            "b.lisp" => "x\n",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    repeated_pct_prefix_not_recognized = { CheckCase {
        // `%%` is not the registered prefix `%` — must not match.
        files: files! {
            "test.tex" => "
                %% LINT.IfChange
                content
                %% LINT.ThenChange(//b.tex)
            ",
            "b.tex" => "x\n",
        },
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    multibyte_utf8_parsed_correctly = { CheckCase {
        // Files with multi-byte UTF-8 characters (em-dash, CJK, emoji) are
        // scanned correctly and directives around them are recognized.
        files: files! {
            "test.rs" => "
                // LINT.IfChange
                let s = \"Principles — must follow\";
                let t = \"café ☕ naïve 日本語\";
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    prose_mention_not_parsed_as_multiline = { CheckCase {
        // "Use LINT.ThenChange to keep in sync." must not trigger multiline mode.
        files: files! {
            "test.rs" => "
                // Use LINT.ThenChange to keep in sync.
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[3])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    string_ending_at_eol_not_treated_as_multiline = { CheckCase {
        // Regression: when a closing `"` is the last byte on a line (e.g.
        // `#include "foo.h"`), the scanner used to enter a multi-line skip
        // region, swallowing subsequent comment tokens including LINT directives.
        files: files! {
            "test.cc" => r#"
                #include "foo.h"
                // LINT.IfChange
                int x = 1;
                // LINT.ThenChange(//b.cc)
            "#,
            "b.cc" => "x\n",
        },
        changes: Some(const { &[sub("test.cc", &[3])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    fenced_code_block_in_markdown_ignored = { CheckCase {
        files: files! {
            "docs/guide.md" => "
                <!-- LINT.IfChange -->
                Real content here.
                <!-- LINT.ThenChange(//src/lib.rs) -->

                ```python
                # LINT.IfChange
                SPEED = 88
                # LINT.ThenChange(//tests/test_speed.py)
                ```
            ",
            "src/lib.rs" => "fn lib() {}\n",
        },
        changes: Some(const { &[sub("docs/guide.md", &[2])] }),
        expected_findings: Some(&["src/lib.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    fenced_code_block_in_doc_comment_ignored = { CheckCase {
        files: files! {
            "src/lib.rs" => "
                // LINT.IfChange
                fn real_code() {}
                // LINT.ThenChange(//src/other.rs)

                // Here\'s an example:
                // ```
                // // LINT.IfChange
                // some_function();
                // // LINT.ThenChange(//nonexistent.rs)
                // ```
            ",
            "src/other.rs" => "fn other() {}\n",
        },
        changes: Some(const { &[sub("src/lib.rs", &[2])] }),
        expected_findings: Some(&["src/other.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    directives_after_fenced_block_parsed = { CheckCase {
        files: files! {
            "docs/guide.md" => "
                ```
                <!-- LINT.IfChange -->
                <!-- LINT.ThenChange(//nonexistent.md) -->
                ```

                <!-- LINT.IfChange -->
                Real content.
                <!-- LINT.ThenChange(//target.md) -->
            ",
            "target.md" => "x\n",
        },
        changes: Some(const { &[sub("docs/guide.md", &[7])] }),
        expected_findings: Some(&["target.md"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    inline_backtick_fenced_code_not_swallowed = { CheckCase {
        // Inline backtick-quoted triple backticks (e.g., ` ``` `) mid-line
        // must not open a FencedCode skip region that swallows later directives.
        files: files! {
            "docs/readme.md" => "
                Fenced code blocks (` ``` `) are skipped by the scanner.

                <!-- LINT.IfChange -->
                Important content.
                <!-- LINT.ThenChange(//other.md) -->
            ",
            "other.md" => "x\n",
        },
        changes: Some(const { &[sub("docs/readme.md", &[4])] }),
        expected_findings: Some(&["other.md"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    unicode_before_html_close_tag = { CheckCase {
        // <script> containing İ (U+0130, 2 bytes) whose lowercase i̇ is
        // 3 bytes.  find_close_offset must return an offset in the original
        // string, not the lowered copy, or pos overshoots by 1 byte and
        // the trailing quote is mishandled — the scanner enters InSkipRegion
        // looking for a closing `"`, swallowing directives on later lines.
        files: files! {
            "test.html" => "<script>\u{0130}</script>\"quoted text\"\n<!-- LINT.IfChange -->\ncontent\n<!-- LINT.ThenChange(//b.html) -->\n",
            "b.html" => "x\n",
        },
        changes: Some(const { &[sub("test.html", &[3])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    escaped_quote_at_eol_not_confused = { CheckCase {
        // A string ending with \" at end of line: the backslash-quote
        // pair exercises the pos += 2 overshoot in skip_single_line_string.
        // The escaped quote must not close the string; directives after
        // the real close on the next line must still be recognised.
        files: files! {
            "test.rs" => "let s = \"embedded \\\"\nclosing\";\n// LINT.IfChange\ncode\n// LINT.ThenChange(//b.rs)\n",
            "b.rs" => "x\n",
        },
        changes: Some(const { &[sub("test.rs", &[4])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
)]
fn directive_recognition(case: CheckCase) {
    run_case(&case);
}

// What malformed input does the tool catch?

#[parameterized(
    orphan_then_change = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.ThenChange(:x)
            ",
        },
        expected_errors: Some(&["without preceding IfChange"]),
        ..DEFAULTS
    } },
    unclosed_if_change = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
            ",
        },
        expected_errors: Some(&["without matching ThenChange"]),
        ..DEFAULTS
    } },
    consecutive_if_change_reports_first_line = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.IfChange
                more
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        expected_errors: Some(&["without matching ThenChange", "previous"]),
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
    duplicate_label = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange(x)
                code
                // LINT.ThenChange(//y.rs)
                // LINT.IfChange(x)
                more
                // LINT.ThenChange(//z.rs)
            ",
            "y.rs" => "y\n",
            "z.rs" => "z\n",
        },
        expected_errors: Some(&["duplicate"]),
        ..DEFAULTS
    } },
    bare_path_missing_prefix = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(src/foo.rs)
            ",
        },
        expected_errors: Some(&["must start with //"]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    path_traversal_rejected = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//foo/../etc/passwd)
            ",
        },
        expected_errors: Some(&["path traversal"]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    absolute_path_rejected = { CheckCase {
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(///etc/passwd)
            ",
        },
        expected_errors: Some(&["must be relative"]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    double_dot_in_filename_allowed = { CheckCase {
        // ".." as part of a filename (not a path component) is fine.
        files: files!{
            "test.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//src/file..txt)
            ",
            "src/file..txt" => "x\n",
        },
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
    modified_but_missing_file = { CheckCase {
        // File appears in diff as modified but doesn't exist on disk.
        files: files!{},
        changes: Some(const { &[sub("missing.rs", &[1])] }),
        expected_errors: Some(&["not found on disk"]),
        ..DEFAULTS
    } },
)]
fn parse_errors(case: CheckCase) {
    run_case(&case);
}

// When a guarded region changes, its targets must also change.

#[parameterized(
    source_and_target_both_modified = { CheckCase {
        files: files!{
            "src/api.rs" => "
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange(//src/handler.rs)
            ",
            "src/handler.rs" => "fn handler() {}\n",
        },
        changes: Some(const { &[sub("src/api.rs", &[2]), sub("src/handler.rs", &[1])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    source_modified_target_unchanged = { CheckCase {
        files: files!{
            "src/api.rs" => "
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange(//src/handler.rs)
            ",
            "src/handler.rs" => "fn handler() {}\n",
        },
        changes: Some(const { &[sub("src/api.rs", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    target_file_missing = { CheckCase {
        files: files!{
            "src/api.rs" => "
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange(//src/missing.rs)
            ",
        },
        changes: Some(const { &[sub("src/api.rs", &[2])] }),
        expected_findings: Some(&["target file not found"]),
        ..DEFAULTS
    } },
    changes_outside_guarded_range_ignored = { CheckCase {
        files: files!{
            "src/api.rs" => "
                fn before() {}
                // LINT.IfChange
                fn api() {}
                // LINT.ThenChange(//src/handler.rs)
                fn after() {}
            ",
            "src/handler.rs" => "fn handler() {}\n",
        },
        changes: Some(const { &[sub("src/api.rs", &[1])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    removed_lines_trigger_check = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs)
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        changes: Some(const { &[("src/a.rs", &[], &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    multiple_blocks_only_triggered_fires = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f1() {}
                // LINT.ThenChange(//src/b.rs)
                fn gap() {}
                // LINT.IfChange
                fn f2() {}
                // LINT.ThenChange(//src/c.rs)
            ",
            "src/b.rs" => "fn b() {}\n",
            "src/c.rs" => "fn c() {}\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2]), sub("src/b.rs", &[1])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    multiple_targets_all_modified = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs, //src/c.rs)
            ",
            "src/b.rs" => "fn b() {}\n",
            "src/c.rs" => "fn c() {}\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2]), sub("src/b.rs", &[1]), sub("src/c.rs", &[1])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    multiple_targets_one_not_modified = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs, //src/c.rs)
            ",
            "src/b.rs" => "fn b() {}\n",
            "src/c.rs" => "fn c() {}\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2]), sub("src/b.rs", &[1])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    self_reference_without_label = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/a.rs)
            ",
        },
        changes: Some(const { &[sub("src/a.rs", &[2])] }),
        expected_findings: Some(&["self-referencing"]),
        ..DEFAULTS
    } },
)]
fn change_detection(case: CheckCase) {
    run_case(&case);
}

// How do labeled regions narrow the "must change" requirement?

#[parameterized(
    labeled_target_modified_in_range = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs:section)
            ",
            "src/b.rs" => "
                fn before() {}
                // LINT.IfChange(section)
                fn target() {}
                // LINT.ThenChange(//src/a.rs)
                fn after() {}
            ",
        },
        changes: Some(const { &[sub("src/a.rs", &[2]), sub("src/b.rs", &[3])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    change_outside_label_range = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs:section)
            ",
            "src/b.rs" => "
                fn before() {}
                // LINT.IfChange(section)
                fn target() {}
                // LINT.ThenChange(//src/a.rs)
                fn after() {}
            ",
        },
        changes: Some(const { &[sub("src/a.rs", &[2]), sub("src/b.rs", &[1])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
    target_label_not_found = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs:nonexistent)
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2]), sub("src/b.rs", &[1])] }),
        expected_findings: Some(&["not found"]),
        ..DEFAULTS
    } },
    same_file_label_both_modified = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange(top)
                fn f() {}
                // LINT.ThenChange(:section)
                // LINT.IfChange(section)
                fn target() {}
                // LINT.ThenChange(:top)
            ",
        },
        changes: Some(const { &[sub("src/a.rs", &[2, 5])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    full_path_self_reference_with_label = { CheckCase {
        files: files! {
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/a.rs:section)
                // LINT.IfChange(section)
                fn g() {}
                // LINT.ThenChange(//src/dummy.rs)
            ",
            "src/dummy.rs" => "x\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2, 5])] }),
        expected_findings: Some(&[":section"]),
        ..DEFAULTS
    } },
    hyphenated_label_accepted = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange(my-label)
                fn f() {}
                // LINT.ThenChange(:other-part)
                // LINT.IfChange(other-part)
                fn g() {}
                // LINT.ThenChange(:my-label)
            ",
        },
        changes: Some(const { &[sub("src/a.rs", &[2, 5])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    dotted_label_accepted = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange(Payments.Pix.Result)
                fn f() {}
                // LINT.ThenChange(:Other.Section)
                // LINT.IfChange(Other.Section)
                fn g() {}
                // LINT.ThenChange(:Payments.Pix.Result)
            ",
        },
        changes: Some(const { &[sub("src/a.rs", &[2, 5])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    label_not_cleared_by_non_matching_if_change = { CheckCase {
        files: files! {
            "source.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//target.rs:wanted)
            ",
            "target.rs" => "
                // LINT.IfChange(wanted)
                target content
                // LINT.IfChange(other)
                other content
                // LINT.ThenChange(//z.rs)
                // LINT.ThenChange(//z.rs)
            ",
            "z.rs" => "z\n",
        },
        changes: Some(const { &[sub("source.rs", &[2]), sub("target.rs", &[2])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
)]
fn label_scoped_detection(case: CheckCase) {
    run_case(&case);
}

// What happens when a referenced file is deleted?
// The reverse lookup finds surviving files whose ThenChange targets reference
// the deleted file and reports a finding.

#[parameterized(
    unchanged_file_references_deleted_target = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//src/b.rs)
            ",
        },
        changes: Some(&[]),
        deleted: &["src/b.rs"],
        expected_findings: Some(&["target file not found: src/b.rs"]),
        ..DEFAULTS
    } },
    deleted_target_with_label = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//src/b.rs:section)
            ",
        },
        changes: Some(&[]),
        deleted: &["src/b.rs"],
        expected_findings: Some(&["target file not found: src/b.rs (label section)"]),
        ..DEFAULTS
    } },
    no_deletions_no_findings = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//src/b.rs)
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        changes: Some(const { &[sub("src/b.rs", &[1])] }),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    deleted_file_no_error = { CheckCase {
        // Deleted files are expected to be absent — no error.
        files: files!{},
        deleted: &["deleted.rs"],
        changes: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
)]
fn deleted_file_detection(case: CheckCase) {
    run_case(&case);
}

// What does the structural validity pass check?
// Files in the file_list are validated even without a diff trigger:
// target existence, label existence, and no duplicate findings when both
// passes fire on the same file.

#[parameterized(
    target_file_missing = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//missing.rs)
            ",
        },
        changes: Some(&[]),
        file_list: &["a.rs"],
        expected_findings: Some(&["target file not found: missing.rs"]),
        ..DEFAULTS
    } },
    target_label_missing = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//b.rs:nonexistent)
            ",
            "b.rs" => "fn b() {}\n",
        },
        changes: Some(&[]),
        file_list: &["a.rs"],
        expected_findings: Some(&["label nonexistent not found in b.rs"]),
        ..DEFAULTS
    } },
    valid_target = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//b.rs:section)
            ",
            "b.rs" => "
                // LINT.IfChange(section)
                fn b() {}
                // LINT.ThenChange(//a.rs)
            ",
        },
        changes: Some(&[]),
        file_list: &["a.rs"],
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    file_itself_missing = { CheckCase {
        files: files!{},
        changes: Some(&[]),
        file_list: &["nonexistent.rs"],
        expected_errors: Some(&["file not found on disk"]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    in_file_list_and_diff_no_duplicate = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "fn b() {}\n",
        },
        changes: Some(const { &[sub("a.rs", &[2])] }),
        file_list: &["a.rs"],
        expected_findings: Some(&["may need to be reflected in"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    same_file_label_exists = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(:section)
                // LINT.IfChange(section)
                fn b() {}
                // LINT.ThenChange(//dummy.rs)
            ",
            "dummy.rs" => "x\n",
        },
        changes: Some(&[]),
        file_list: &["a.rs"],
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    same_file_label_missing = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(:nonexistent)
            ",
        },
        changes: Some(&[]),
        file_list: &["a.rs"],
        expected_findings: Some(&["label nonexistent not found in a.rs"]),
        ..DEFAULTS
    } },
    structural_and_diff_distinct = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//missing_structural.rs)
            ",
            "b.rs" => "
                // LINT.IfChange
                fn b() {}
                // LINT.ThenChange(//c.rs)
            ",
            "c.rs" => "fn c() {}\n",
        },
        // b.rs has content changes but is NOT in file_list → diff pass
        // is scoped to file_list and skips b.rs. Only the structural
        // finding from a.rs is expected.
        changes: Some(const { &[sub("b.rs", &[2])] }),
        file_list: &["a.rs"],
        expected_findings: Some(&["source:a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    structural_and_diff_same_file = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//missing.rs)
            ",
        },
        changes: Some(const { &[sub("a.rs", &[2])] }),
        file_list: &["a.rs"],
        expected_findings: Some(&["missing.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    reverse_lookup_scoped_to_non_file_list = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//b.rs)
            ",
            "c.rs" => "
                // LINT.IfChange
                fn c() {}
                // LINT.ThenChange(//b.rs)
            ",
        },
        changes: Some(&[]),
        deleted: &["b.rs"],
        file_list: &["a.rs"],
        expected_findings: Some(&["source:a.rs", "source:c.rs"]),
        expected_finding_count: Some(2),
        ..DEFAULTS
    } },
    deleted_target_no_duplicate_with_file_list = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn a() {}
                // LINT.ThenChange(//src/b.rs)
            ",
        },
        changes: Some(&[]),
        deleted: &["src/b.rs"],
        file_list: &["src/a.rs"],
        expected_findings: Some(&["target file not found: src/b.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
)]
fn file_list_validation(case: CheckCase) {
    run_case(&case);
}

// How do ignore patterns work?

#[parameterized(
    matching_target_skipped = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//generated/api.rs)
            ",
            "generated/api.rs" => "fn api() {}\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2])] }),
        ignore_patterns: &["generated/*"],
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    non_matching_still_checked = { CheckCase {
        files: files!{
            "src/a.rs" => "
                // LINT.IfChange
                fn f() {}
                // LINT.ThenChange(//src/b.rs)
            ",
            "src/b.rs" => "fn b() {}\n",
        },
        changes: Some(const { &[sub("src/a.rs", &[2])] }),
        ignore_patterns: &["generated/*"],
        expected_findings: Some(&["may need to be reflected in"]),
        ..DEFAULTS
    } },
)]
fn ignore_patterns(case: CheckCase) {
    run_case(&case);
}

// Binary file requires injecting a null byte, which can't be done via static
// string literals in the parameterized macro. Tested as a standalone case that
// manually constructs the mock.
#[test]
fn binary_file_skipped() {
    let mut mock = MockVcsProvider::default();
    let mut content = "// LINT.IfChange\ncode\n// LINT.ThenChange(//nonexistent.rs)\n".to_string();
    content.push('\0');
    mock.add_file("binary.rs", &content);

    let changes: ChangeMap = [(
        "binary.rs".to_string(),
        FileChanges {
            added_lines: (1..=3).collect(),
            ..Default::default()
        },
    )]
    .into_iter()
    .collect();

    let result = check(&mock, &changes, &[]);
    assert!(result.is_empty());
}

// A realistic PNG file (starts with the 8-byte PNG signature which contains null
// bytes) must be detected as binary and skipped entirely.
#[test]
fn binary_png_file_skipped() {
    let mut mock = MockVcsProvider::default();
    // PNG signature: 0x89 P N G \r \n 0x1A \n followed by a minimal IHDR chunk.
    // Contains null bytes that trigger binary detection.
    let png_bytes: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, // IHDR chunk length
        b'I', b'H', b'D', b'R', // IHDR tag
    ];
    let content = String::from_utf8_lossy(png_bytes).into_owned();
    mock.add_file("icon.png", &content);

    let changes: ChangeMap = [(
        "icon.png".to_string(),
        FileChanges {
            added_lines: (1..=1).collect(),
            ..Default::default()
        },
    )]
    .into_iter()
    .collect();

    let result = check(&mock, &changes, &[]);
    assert!(result.is_empty());
}

// Bare filenames resolve relative to the source file's directory.

#[parameterized(
    bare_filename_same_dir = { CheckCase {
        // A bare filename like `bar.cc` in ThenChange should resolve to the
        // source file's directory, not the project root.
        files: files! {
            "a/b/foo.h" => "
                // LINT.IfChange
                int x;
                // LINT.ThenChange(bar.cc)
            ",
            "a/b/bar.cc" => "int y;\n",
        },
        strict: false,
        file_list: &["a/b/foo.h"],
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    bare_filename_diff_mode = { CheckCase {
        // Diff mode: change in foo.h should trigger a finding against a/b/bar.cc.
        files: files! {
            "a/b/foo.h" => "
                // LINT.IfChange
                int x;
                // LINT.ThenChange(bar.cc)
            ",
            "a/b/bar.cc" => "int y;\n",
        },
        strict: false,
        changes: Some(const { &[sub("a/b/foo.h", &[2])] }),
        expected_findings: Some(&["may need to be reflected in a/b/bar.cc"]),
        ..DEFAULTS
    } },
    bare_filename_at_root = { CheckCase {
        // When the source file is at the root, bare filenames stay root-relative.
        files: files! {
            "foo.h" => "
                // LINT.IfChange
                int x;
                // LINT.ThenChange(bar.cc)
            ",
            "bar.cc" => "int y;\n",
        },
        strict: false,
        file_list: &["foo.h"],
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    path_with_slash_stays_root_relative = { CheckCase {
        // Paths containing `/` are root-relative even without `//`.
        files: files! {
            "a/b/foo.h" => "
                // LINT.IfChange
                int x;
                // LINT.ThenChange(c/bar.cc)
            ",
            "c/bar.cc" => "int y;\n",
        },
        strict: false,
        file_list: &["a/b/foo.h"],
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    then_change_empty_parens = { CheckCase {
        // ThenChange with empty parens is a valid closure with no targets.
        files: files! {
            "test.rs" => "
                // LINT.IfChange
                int x;
                // LINT.ThenChange()
            ",
        },
        file_list: &["test.rs"],
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
    self_ref_suppressed = { CheckCase {
        // //same-file:label should NOT warn in non-strict mode.
        files: files! {
            "a/foo.h" => "
                // LINT.IfChange(section_a)
                int x;
                // LINT.ThenChange(//a/foo.h:section_b)
                // LINT.IfChange(section_b)
                int y;
                // LINT.ThenChange(//a/foo.h:section_a)
            ",
        },
        strict: false,
        file_list: &["a/foo.h"],
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        ..DEFAULTS
    } },
)]
fn non_strict(case: CheckCase) {
    run_case(&case);
}

// Adding new directive pairs, modifying ThenChange targets, or renaming
// IfChange labels should NOT trigger diff-based validation — only content
// between directives matters.

#[parameterized(
    new_pair_around_existing_code = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // Directive lines added, content unchanged.
        changes: Some(&[("a.rs", &[1, 3], &[])]),
        expected_findings: Some(&[]),
        expected_finding_count: Some(0),
        ..DEFAULTS
    } },
    new_pair_with_new_content = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                new_code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // All lines added — new file or new block.
        changes: Some(&[("a.rs", &[1, 2, 3], &[])]),
        expected_findings: Some(&[]),
        expected_finding_count: Some(0),
        ..DEFAULTS
    } },
    modify_then_change_only = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//b.rs, //c.rs)
            ",
            "b.rs" => "x\n",
            "c.rs" => "x\n",
        },
        // Only ThenChange line substituted (e.g. target added).
        changes: Some(const { &[sub("a.rs", &[3])] }),
        expected_findings: Some(&[]),
        expected_finding_count: Some(0),
        ..DEFAULTS
    } },
    rename_if_change_label = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange(new_name)
                code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // Only IfChange line substituted (label rename, content unchanged).
        changes: Some(const { &[sub("a.rs", &[1])] }),
        expected_findings: Some(&[]),
        expected_finding_count: Some(0),
        ..DEFAULTS
    } },
    rename_if_change_label_with_content_change = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange(new_name)
                modified_code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // IfChange line AND content line both substituted simultaneously.
        // Renaming a label while also modifying content must still trigger —
        // the is_triggered guard only suppresses brand-new pairs (IfChange
        // added without a matching removal at its position).
        changes: Some(const { &[sub("a.rs", &[1, 2])] }),
        expected_findings: Some(&["may need to be reflected in b.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    empty_block_then_change_substituted = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                // LINT.ThenChange(//b.rs, //c.rs)
            ",
            "b.rs" => "x\n",
            "c.rs" => "x\n",
        },
        // ThenChange substituted on a block with no content lines.
        // content_start > content_end — any_in_range's lo > hi guard must
        // prevent a false positive here.
        changes: Some(const { &[sub("a.rs", &[2])] }),
        expected_findings: Some(&[]),
        expected_finding_count: Some(0),
        ..DEFAULTS
    } },
    empty_parens_content_changed = { CheckCase {
        // ThenChange() with empty target list: content changes inside the
        // guarded block produce no finding because there is nothing to check.
        files: files! {
            "test.rs" => "
                // LINT.IfChange
                int x;
                // LINT.ThenChange()
            ",
        },
        changes: Some(const { &[sub("test.rs", &[2])] }),
        expected_errors: Some(&[]),
        expected_findings: Some(&[]),
        expected_finding_count: Some(0),
        ..DEFAULTS
    } },
)]
fn directive_only_changes(case: CheckCase) {
    run_case(&case);
}

// When guarded content between IfChange/ThenChange is modified, all targets
// must also show changes.

#[parameterized(
    content_modified_target_untouched = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                modified_code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // Content line substituted.
        changes: Some(const { &[sub("a.rs", &[2])] }),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    content_removed_inside_block = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                remaining
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // Content was removed; collapse point is line 2 (between directives).
        changes: Some(&[("a.rs", &[], &[2])]),
        expected_findings: Some(&["may need to be reflected in"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    content_modified_and_new_target = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                modified
                // LINT.ThenChange(//b.rs, //c.rs)
            ",
            "b.rs" => "x\n",
            "c.rs" => "x\n",
        },
        // Content changed AND ThenChange replaced (new target added).
        changes: Some(&[("a.rs", &[2, 3], &[2, 3])]),
        expected_findings: Some(&[
            "may need to be reflected in b.rs",
            "may need to be reflected in c.rs",
        ]),
        expected_finding_count: Some(2),
        ..DEFAULTS
    } },
    content_removed_and_then_change_replaced = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                remaining
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
        },
        // Content line removed (collapse) AND ThenChange line replaced.
        // This exercises the `removal_end = content_end` cap in is_triggered:
        // the removal at the ThenChange position is excluded from the content
        // check, but the removal at the content position still fires.
        changes: Some(&[("a.rs", &[3], &[2, 3])]),
        expected_findings: Some(&["may need to be reflected in b.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
)]
fn content_changes_trigger(case: CheckCase) {
    run_case(&case);
}

// The file list restricts diff-based validation to listed files. Reverse
// lookup is NOT scoped — it always runs globally.

#[parameterized(
    file_list_scopes_diff = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                modified
                // LINT.ThenChange(//target_a.rs)
            ",
            "b.rs" => "
                // LINT.IfChange
                modified
                // LINT.ThenChange(//target_b.rs)
            ",
            "target_a.rs" => "x\n",
            "target_b.rs" => "x\n",
        },
        // Both files have content changes.
        changes: Some(const { &[sub("a.rs", &[2]), sub("b.rs", &[2])] }),
        // Only a.rs in file list — b.rs's finding should NOT appear.
        file_list: &["a.rs"],
        expected_findings: Some(&["may need to be reflected in target_a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    file_list_no_scope_reverse_lookup = { CheckCase {
        files: files!{
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//deleted.rs)
            ",
        },
        // No file changes except deleted.
        changes: Some(&[]),
        deleted: &["deleted.rs"],
        // ref.rs is NOT in file list, but reverse lookup fires globally.
        file_list: &["other.rs"],
        expected_findings: Some(&["target file not found: deleted.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
)]
fn file_list_scoping(case: CheckCase) {
    run_case(&case);
}

// When no file list is provided but staged changes exist, the structural
// validation set is auto-derived from changed (non-deleted) files.

#[parameterized(
    auto_validate_staged_files = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//missing.rs)
            ",
        },
        // a.rs in changes (but no line-level changes). No explicit file_list.
        changes: Some(&[("a.rs", &[], &[])]),
        file_list: &[],
        expected_findings: Some(&["target file not found: missing.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    // Non-triggered pair with labeled target to an existing unchanged file.
    // The structural pass must not report "label not found" — the target
    // exists and has the label, it just wasn't touched in this diff.
    auto_derive_non_triggered_labeled_target = { CheckCase {
        files: files!{
            "a.rs" => "
                // LINT.IfChange
                guarded code
                // LINT.ThenChange(//b.rs:foo)
                unguarded code
            ",
            "b.rs" => "
                // LINT.IfChange(foo)
                target code
                // LINT.ThenChange(//a.rs)
            ",
        },
        // Line 5 changed — outside the IfChange region (lines 2-4), so
        // the pair is NOT triggered and the structural pass checks it.
        changes: Some(const { &[sub("a.rs", &[5])] }),
        file_list: &[],
        expected_findings: Some(&[]),
        expected_errors: Some(&[]),
        ..DEFAULTS
    } },
)]
fn auto_populate_validation(case: CheckCase) {
    run_case(&case);
}

// When a file is modified and its labels change, surviving references from
// other files to removed labels are caught by the reverse lookup pass.

#[parameterized(
    stale_label_removed = { CheckCase {
        files: files!{
            // Label was removed from a.rs.
            "a.rs" => "
                fn a() {}
            ",
            // ref.rs still references the old label.
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//a.rs:old_label)
            ",
        },
        // a.rs is in the diff (was modified to remove label).
        changes: Some(&[("a.rs", &[], &[])]),
        file_list: &[],
        expected_findings: Some(&["label old_label not found in a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    renamed_label = { CheckCase {
        files: files!{
            // Label was renamed in a.rs.
            "a.rs" => "
                // LINT.IfChange(new_name)
                code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
            // ref.rs references the old name.
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//a.rs:old_name)
            ",
        },
        // IfChange line substituted (rename).
        changes: Some(const { &[sub("a.rs", &[1])] }),
        file_list: &[],
        expected_findings: Some(&["label old_name not found in a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    moved_label = { CheckCase {
        files: files!{
            // Label moved away from a.rs.
            "a.rs" => "
                fn a() {}
            ",
            // Label now lives in b.rs.
            "b.rs" => "
                // LINT.IfChange(moved_label)
                code
                // LINT.ThenChange(//c.rs)
            ",
            "c.rs" => "x\n",
            // ref.rs still points to the old location.
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//a.rs:moved_label)
            ",
        },
        // Both a.rs and b.rs changed.
        changes: Some(&[("a.rs", &[], &[]), ("b.rs", &[1, 2, 3], &[])]),
        file_list: &[],
        expected_findings: Some(&["label moved_label not found in a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    valid_label_no_false_positive = { CheckCase {
        files: files!{
            // Labels intact in a.rs.
            "a.rs" => "
                // LINT.IfChange(my_label)
                code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
            // ref.rs correctly references the label.
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//a.rs:my_label)
            ",
        },
        // Content changed in a.rs, labels intact.
        changes: Some(const { &[sub("a.rs", &[2])] }),
        file_list: &[],
        // Diff finding fires (b.rs not changed), but NO stale-label finding.
        expected_findings: Some(&["may need to be reflected in b.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    stale_label_when_ref_file_also_in_diff = { CheckCase {
        files: files!{
            // a.rs has a renamed label.
            "a.rs" => "
                // LINT.IfChange(new_name)
                code
                // LINT.ThenChange(//b.rs)
            ",
            "b.rs" => "x\n",
            // ref.rs is also a changed file and still references the old label.
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//a.rs:old_name)
            ",
        },
        // Both a.rs and ref.rs are in the diff. With derived_validate_files,
        // file_list_set = {a.rs, ref.rs}: reverse lookup skips ref.rs, but
        // the structural pass handles ref.rs and catches the stale label
        // (the pair is not triggered — no sorted changes affect its content).
        changes: Some(const { &[sub("a.rs", &[1]), ("ref.rs", &[], &[])] }),
        file_list: &[],
        expected_findings: Some(&["label old_name not found in a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
    partial_label_removal = { CheckCase {
        files: files!{
            // a.rs originally had IfChange(foo) and IfChange(bar); foo was removed.
            "a.rs" => "
                // LINT.IfChange(bar)
                bar_body
                // LINT.ThenChange(//c.rs)
            ",
            "c.rs" => "x\n",
            // ref.rs still references the removed foo label.
            "ref.rs" => "
                // LINT.IfChange
                code
                // LINT.ThenChange(//a.rs:foo)
            ",
        },
        // a.rs is in the diff with removals (the foo pair was deleted).
        // removed_new_pos=[1] signals that lines were removed from the file,
        // triggering the label_sets guard to include a.rs in the reverse lookup.
        changes: Some(&[("a.rs", &[], &[1])]),
        file_list: &[],
        expected_findings: Some(&["label foo not found in a.rs"]),
        expected_finding_count: Some(1),
        ..DEFAULTS
    } },
)]
fn stale_label_reverse_lookup(case: CheckCase) {
    run_case(&case);
}

// Verify that Finding::source_location() returns file:line without label.
#[test]
fn finding_source_location_omits_label() {
    let mut mock = MockVcsProvider::default();
    mock.add_file(
        "src/a.rs",
        &unindent(
            "
            // LINT.IfChange(my_section)
            fn f() {}
            // LINT.ThenChange(//src/b.rs)
        ",
        ),
    );
    mock.add_file("src/b.rs", "fn b() {}\n");

    let changes: ChangeMap = [(
        "src/a.rs".to_string(),
        FileChanges {
            added_lines: [2].into_iter().collect(),
            removed_lines: [2].into_iter().collect(),
            removed_new_positions: [2].into_iter().collect(),
            ..Default::default()
        },
    )]
    .into_iter()
    .collect();
    let result = check(&mock, &changes, &[]);
    let warnings: Vec<_> = result
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].location(), "src/a.rs:1");
}

// When a non-matching IfChange appears between a matching IfChange and its
// ThenChange (malformed input), find_label_range caps the range at the
// interrupting IfChange rather than extending into a different pair's content.
// validate_structure catches the malformed pairing separately as a parse error.

#[test]
fn find_label_range_with_interleaved_non_matching_if_change() {
    let mut cache = ParseCache::new();
    cache.insert(
        "target.rs".to_string(),
        ParsedFile {
            directives: vec![
                Directive::IfChange {
                    line: NonZeroUsize::new(1).unwrap(),
                    label: Some("foo".to_string()),
                },
                Directive::IfChange {
                    line: NonZeroUsize::new(3).unwrap(),
                    label: Some("bar".to_string()),
                },
                Directive::ThenChange {
                    line: NonZeroUsize::new(5).unwrap(),
                    targets: vec![],
                },
            ],
            errors: vec![],
        },
    );

    let range = find_label_range("target.rs", "foo", &cache);
    assert!(range.is_some(), "should find label despite malformed input");
    let range = range.unwrap();
    // Range is [2, 2] — capped at the interrupting IfChange(bar) on line 3,
    // not extending to the ThenChange on line 5.
    assert_eq!(range.start, 2);
    assert_eq!(range.end, 2);
}

#[test]
fn find_label_range_missing_label() {
    let mut cache = ParseCache::new();
    cache.insert(
        "target.rs".to_string(),
        ParsedFile {
            directives: vec![
                Directive::IfChange {
                    line: NonZeroUsize::new(1).unwrap(),
                    label: Some("bar".to_string()),
                },
                Directive::ThenChange {
                    line: NonZeroUsize::new(3).unwrap(),
                    targets: vec![],
                },
            ],
            errors: vec![],
        },
    );

    assert!(
        find_label_range("target.rs", "nonexistent", &cache).is_none(),
        "should return None for missing label"
    );
}
