use super::*;
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;
use unindent::unindent;
use yare::parameterized;

/// Map-like syntax for file definitions: `files!{ "name" => "content", ... }`
macro_rules! files {
    ($($name:expr => $content:expr),* $(,)?) => {
        &[$(($name, $content)),*] as &[(&str, &str)]
    };
}

fn write_files(dir: &Path, files: &[(&str, &str)]) {
    for (path, content) in files {
        let p = dir.join(path);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&p, content).unwrap();
    }
}

// ─── from_diff: file count ───

#[parameterized(
    empty = { "", 0 },
    one_add = { "
        --- a/src/main.rs
        +++ b/src/main.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ", 1 },
    one_remove = { "
        --- a/src/main.rs
        +++ b/src/main.rs
        @@ -1,4 +1,3 @@
         fn main() {
        -    println!(\"hello\");
             // existing
         }
    ", 1 },
    deleted_file = { "
        --- a/src/old.rs
        +++ /dev/null
        @@ -1,3 +0,0 @@
        -fn old() {
        -    // gone
        -}
    ", 1 },
    rename = { "
        --- a/src/old.rs
        +++ b/src/new.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ", 2 },
)]
fn from_diff_file_count(diff: &str, expected_file_count: usize) {
    let diff = unindent(diff);
    let map = from_diff(&mut Cursor::new(diff)).unwrap();
    assert_eq!(
        map.len(),
        expected_file_count,
        "files: {:?}",
        map.keys().collect::<Vec<_>>()
    );
}

// ─── from_diff: line numbers ───

#[parameterized(
    add = { "
        --- a/src/main.rs
        +++ b/src/main.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ", "src/main.rs", &[2usize], &[] },
    remove = { "
        --- a/src/main.rs
        +++ b/src/main.rs
        @@ -1,4 +1,3 @@
         fn main() {
        -    println!(\"hello\");
             // existing
         }
    ", "src/main.rs", &[], &[2usize] },
)]
fn from_diff_lines(diff: &str, file: &str, expected_added: &[usize], expected_removed: &[usize]) {
    let diff = unindent(diff);
    let map = from_diff(&mut Cursor::new(diff)).unwrap();
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

// ─── from_directory ───

#[parameterized(
    one_file = { files!{ "a.rs" => "line1\nline2\n" }, 1 },
    two_files = { files!{ "a.rs" => "l1\n", "b.rs" => "l1\nl2\n" }, 2 },
    empty_file = { files!{ "a.rs" => "" }, 0 },
)]
fn from_directory_file_count(files: &[(&str, &str)], expected_file_count: usize) {
    let dir = TempDir::new().unwrap();
    write_files(dir.path(), files);
    let (map, _) = from_directory(dir.path());
    assert_eq!(
        map.len(),
        expected_file_count,
        "files: {:?}",
        map.keys().collect::<Vec<_>>()
    );
}

#[test]
fn from_directory_all_lines_added() {
    let dir = TempDir::new().unwrap();
    write_files(dir.path(), files! { "a.rs" => "line1\nline2\nline3\n" });
    let (map, _) = from_directory(dir.path());
    let changes = &map["a.rs"];
    assert_eq!(changes.added_lines, HashSet::from([1, 2, 3]));
    assert!(changes.removed_lines.is_empty());
}

#[test]
fn from_directory_skips_binary() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("binary.rs");
    let mut content = b"line1\n".to_vec();
    content.push(0);
    fs::write(&file, content).unwrap();

    let (map, _) = from_directory(dir.path());
    assert!(map.is_empty());
}

// ─── from_diff: removed_new_positions merge ───

#[test]
fn from_diff_merges_removed_new_positions_for_duplicate_file() {
    // Two patches both target the same new file path (rename + independent modify).
    // The second patch has removals whose removed_new_positions must be merged.
    // Patch 1 (rename): removal at new-file position 2
    // Patch 2 (direct): removal at new-file position 3
    let diff = unindent(
        "
        --- a/old.rs
        +++ b/target.rs
        @@ -1,3 +1,2 @@
         fn a() {}
        -fn removed_via_rename() {}
         fn c() {}
        --- a/target.rs
        +++ b/target.rs
        @@ -1,4 +1,3 @@
         fn x() {}
         fn y() {}
        -fn removed_directly() {}
         fn z() {}
    ",
    );

    let map = from_diff(&mut Cursor::new(diff)).unwrap();
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

// ─── from_diff: rename tracking (issue 3) ───

#[test]
fn from_diff_rename_includes_added_lines_in_old_path() {
    let diff = unindent(
        "
        --- a/old.rs
        +++ b/new.rs
        @@ -1,3 +1,4 @@
         fn main() {
        +    println!(\"hello\");
             // existing
         }
    ",
    );
    let map = from_diff(&mut Cursor::new(diff)).unwrap();
    let old_changes = &map["old.rs"];
    assert!(
        !old_changes.added_lines.is_empty(),
        "old path should have added_lines from rename, got: {:?}",
        old_changes.added_lines,
    );
}

// ─── from_directory: .git exclusion ───

#[test]
fn from_directory_skips_dot_git() {
    let dir = TempDir::new().unwrap();
    // Create a normal file
    write_files(dir.path(), files! { "a.rs" => "line1\n" });
    // Create a file inside .git/
    let git_dir = dir.path().join(".git");
    fs::create_dir_all(&git_dir).unwrap();
    fs::write(git_dir.join("config.rs"), "line1\nline2\n").unwrap();

    let (map, _) = from_directory(dir.path());
    assert_eq!(
        map.len(),
        1,
        "should only contain a.rs, got: {:?}",
        map.keys().collect::<Vec<_>>()
    );
    assert!(map.contains_key("a.rs"));
}
