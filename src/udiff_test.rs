use super::*;
use std::io::Cursor;
use unindent::unindent;
use yare::parameterized;

#[parameterized(
    empty = { "", 0 },
    one_add = { "
        --- src/main.rs
        +++ src/main.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ", 1 },
    one_remove = { "
        --- src/main.rs
        +++ src/main.rs
        @@ -1,4 +1,3 @@
         fn main() {
        -    println!(\"hello\");
             // existing
         }
    ", 1 },
    deleted_file = { "
        --- src/old.rs
        +++ /dev/null
        @@ -1,3 +0,0 @@
        -fn old() {
        -    // gone
        -}
    ", 1 },
    rename = { "
        --- src/old.rs
        +++ src/new.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ", 2 },
)]
fn parse_file_count(diff: &str, expected_file_count: usize) {
    let diff = unindent(diff);
    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    assert_eq!(
        map.len(),
        expected_file_count,
        "files: {:?}",
        map.keys().collect::<Vec<_>>()
    );
}

#[parameterized(
    add = { "
        --- src/main.rs
        +++ src/main.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ", "src/main.rs", &[2usize], &[] },
    remove = { "
        --- src/main.rs
        +++ src/main.rs
        @@ -1,4 +1,3 @@
         fn main() {
        -    println!(\"hello\");
             // existing
         }
    ", "src/main.rs", &[], &[2usize] },
)]
fn parse_lines(diff: &str, file: &str, expected_added: &[usize], expected_removed: &[usize]) {
    let diff = unindent(diff);
    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    let changes = &map[file];
    for &line in expected_added {
        assert!(
            changes.added_lines.contains(&line),
            "expected added line {line}"
        );
    }
    for &line in expected_removed {
        assert!(
            changes.removed_lines.contains(&line),
            "expected removed line {line}"
        );
    }
}

#[test]
fn parse_merges_removed_new_positions_for_duplicate_file() {
    // Two patches both target the same new file path (rename + independent modify).
    // The second patch has removals whose removed_new_positions must be merged.
    // Patch 1 (rename): removal at new-file position 2
    // Patch 2 (direct): removal at new-file position 3
    let diff = unindent(
        "
        --- old.rs
        +++ target.rs
        @@ -1,3 +1,2 @@
         fn a() {}
        -fn removed_via_rename() {}
         fn c() {}
        --- target.rs
        +++ target.rs
        @@ -1,4 +1,3 @@
         fn x() {}
         fn y() {}
        -fn removed_directly() {}
         fn z() {}
    ",
    );

    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    let changes = &map["target.rs"];

    // Patch 1 contributes removed_new_position 2, patch 2 contributes 3.
    // Both must be present after merging.
    assert!(
        changes.removed_new_positions.contains(&2),
        "removed_new_positions should contain position 2 from rename patch, got: {:?}",
        changes.removed_new_positions,
    );
    assert!(
        changes.removed_new_positions.contains(&3),
        "removed_new_positions should contain position 3 from direct patch, got: {:?}",
        changes.removed_new_positions,
    );
}

#[test]
fn parse_applies_normalize_to_paths() {
    let diff = unindent(
        "
        --- a/src/main.rs
        +++ b/src/main.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ",
    );
    // Strip `a/` / `b/` prefixes via normalize — simulates git prefix stripping.
    let map = parse(&mut Cursor::new(diff), |p| {
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(p)
            .to_string()
    })
    .unwrap();
    assert!(
        map.contains_key("src/main.rs"),
        "expected stripped key 'src/main.rs', got: {:?}",
        map.keys().collect::<Vec<_>>()
    );
    assert!(
        !map.contains_key("b/src/main.rs"),
        "prefixed key should not appear"
    );
}

#[test]
fn parse_deletion_after_modification_sets_deleted_flag() {
    // A file modified in an earlier patch and then deleted in a later patch
    // must have `deleted: true` regardless of patch order.
    let diff = unindent(
        "
        --- foo.rs
        +++ foo.rs
        @@ -1,3 +1,4 @@
         fn a() {}
        +fn b() {}
         fn c() {}
        --- foo.rs
        +++ /dev/null
        @@ -1,4 +0,0 @@
        -fn a() {}
        -fn b() {}
        -fn c() {}
    ",
    );
    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    let fc = &map["foo.rs"];
    assert!(
        fc.deleted,
        "deleted flag must be set when modification patch precedes deletion"
    );
}

#[test]
fn parse_deleted_file_marked() {
    let diff = unindent(
        "
        --- src/old.rs
        +++ /dev/null
        @@ -1,2 +0,0 @@
        -fn old() {}
        -fn also_old() {}
    ",
    );
    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    let fc = &map["src/old.rs"];
    assert!(fc.deleted, "deleted flag should be set");
    assert!(fc.added_lines.is_empty(), "no added lines for deleted file");
}

#[test]
fn parse_rename_after_modification_merges_changes_for_old_path() {
    // Patch 1: modify old.rs in place → adds a line at new-file position 2.
    // Patch 2: rename old.rs → new.rs → adds a line at new-file position 3.
    // old.rs must accumulate changes from BOTH patches.
    let diff = unindent(
        "
        --- old.rs
        +++ old.rs
        @@ -1,3 +1,4 @@
         fn a() {}
        +fn added_by_modify() {}
         fn c() {}
        --- old.rs
        +++ new.rs
        @@ -1,4 +1,5 @@
         fn a() {}
         fn added_by_modify() {}
        +fn added_by_rename() {}
         fn c() {}
    ",
    );
    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    let old_changes = &map["old.rs"];
    assert!(
        old_changes.added_lines.contains(&2),
        "old.rs must retain changes from the modify patch, got: {:?}",
        old_changes.added_lines,
    );
    assert!(
        old_changes.added_lines.contains(&3),
        "old.rs must also include changes from the rename patch (merge, not drop), got: {:?}",
        old_changes.added_lines,
    );
}

/// Strips git's `a/`/`b/` prefixes like the real git provider does.
fn git_normalize(p: &str) -> String {
    p.strip_prefix("a/")
        .or_else(|| p.strip_prefix("b/"))
        .unwrap_or(p)
        .to_string()
}

#[test]
fn parse_skips_binary_file_after_text_patch() {
    // A binary entry trailing the last text patch used to leave the unified-diff
    // parser with input it rejects outright (panic). It must now be dropped, and
    // the preceding text patch must still parse.
    let diff = unindent(
        "
        diff --git a/src/main.rs b/src/main.rs
        index 1234567..89abcde 100644
        --- a/src/main.rs
        +++ b/src/main.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
        diff --git a/assets/logo.png b/assets/logo.png
        index 1234567..89abcde 100644
        Binary files a/assets/logo.png and b/assets/logo.png differ
    ",
    );
    let map = parse(&mut Cursor::new(diff), git_normalize).unwrap();
    assert!(map.contains_key("src/main.rs"), "text patch should parse");
    assert!(
        !map.contains_key("assets/logo.png"),
        "binary file should be dropped, got: {:?}",
        map.keys().collect::<Vec<_>>()
    );
}

#[test]
fn parse_skips_binary_file_between_text_patches() {
    let diff = unindent(
        "
        diff --git a/a.rs b/a.rs
        index 1111111..2222222 100644
        --- a/a.rs
        +++ b/a.rs
        @@ -1,2 +1,3 @@
         fn a() {}
        +fn a2() {}
         fn z() {}
        diff --git a/img.png b/img.png
        index 3333333..4444444 100644
        Binary files a/img.png and b/img.png differ
        diff --git a/b.rs b/b.rs
        index 5555555..6666666 100644
        --- a/b.rs
        +++ b/b.rs
        @@ -1,2 +1,3 @@
         fn b() {}
        +fn b2() {}
         fn z() {}
    ",
    );
    let map = parse(&mut Cursor::new(diff), git_normalize).unwrap();
    assert!(map.contains_key("a.rs"), "first text patch should parse");
    assert!(map.contains_key("b.rs"), "second text patch should parse");
    assert!(
        !map.contains_key("img.png"),
        "binary file should be dropped"
    );
}

#[test]
fn parse_binary_only_diff_is_empty() {
    let diff = unindent(
        "
        diff --git a/one.png b/one.png
        index 1111111..2222222 100644
        Binary files a/one.png and b/one.png differ
        diff --git a/two.png b/two.png
        index 3333333..4444444 100644
        Binary files a/two.png and b/two.png differ
    ",
    );
    let map = parse(&mut Cursor::new(diff), git_normalize).unwrap();
    assert!(
        map.is_empty(),
        "expected no changes, got: {:?}",
        map.keys().collect::<Vec<_>>()
    );
}

#[test]
fn parse_skips_git_binary_patch_block() {
    // `git diff --binary` emits a `GIT binary patch` block instead of the
    // `Binary files … differ` summary; that block must be dropped too.
    let diff = unindent(
        "
        diff --git a/data.bin b/data.bin
        new file mode 100644
        index 0000000..1111111
        GIT binary patch
        literal 4
        Lc${NkU|;|M0RR91

        literal 0
        HcmV?d00001

        diff --git a/keep.rs b/keep.rs
        index 2222222..3333333 100644
        --- a/keep.rs
        +++ b/keep.rs
        @@ -1,2 +1,3 @@
         fn keep() {}
        +fn kept() {}
         fn z() {}
    ",
    );
    let map = parse(&mut Cursor::new(diff), git_normalize).unwrap();
    assert!(map.contains_key("keep.rs"), "text patch should parse");
    assert!(
        !map.contains_key("data.bin"),
        "binary patch block should be dropped"
    );
}

#[test]
fn parse_rename_includes_added_lines_in_old_path() {
    let diff = unindent(
        "
        --- old.rs
        +++ new.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ",
    );
    let map = parse(&mut Cursor::new(diff), str::to_string).unwrap();
    let old_changes = &map["old.rs"];
    assert!(
        !old_changes.added_lines.is_empty(),
        "old path should have added_lines from rename, got: {:?}",
        old_changes.added_lines,
    );
}
