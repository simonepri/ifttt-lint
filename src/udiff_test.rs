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
