use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;
use unindent::unindent;

use ifttt_lint::changes::ChangeMap;
use ifttt_lint::check;

// ─── Fixture helpers ───

/// Create a temp directory with the given files and return (dir, change_map).
fn setup_scan(files: &[(&str, &str)]) -> (TempDir, ChangeMap) {
    let dir = TempDir::new().unwrap();
    write_files(dir.path(), files);
    let (changes, _) = ifttt_lint::changes::from_directory(dir.path());
    (dir, changes)
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

// ─── Generators ───

/// Generate a single large file with `n_blocks` IfChange/ThenChange pairs,
/// each targeting a label in a companion file.
/// Returns (source_content, target_content).
fn gen_large_file_pair(n_blocks: usize) -> (String, String) {
    let mut source = String::with_capacity(n_blocks * 120);
    let mut target = String::with_capacity(n_blocks * 100);

    for i in 0..n_blocks {
        source.push_str(&unindent(&format!(
            "
            // LINT.IfChange('block_{i}')
            fn source_{i}() {{ /* logic */ }}
            // LINT.ThenChange('//target.rs:label_{i}')
        "
        )));

        target.push_str(&unindent(&format!(
            "
            // LINT.Label('label_{i}')
            fn target_{i}() {{ /* logic */ }}
            // LINT.EndLabel
        "
        )));
    }

    // Pad both files with plain code to reach realistic sizes
    for i in 0..n_blocks {
        source.push_str(&format!("fn padding_src_{i}() {{}}\n"));
        target.push_str(&format!("fn padding_tgt_{i}() {{}}\n"));
    }

    (source, target)
}

/// Generate `n_files` small files, each with one IfChange block targeting the
/// next file in a ring (file_0 → file_1 → ... → file_{n-1} → file_0).
fn gen_many_small_files(n_files: usize) -> Vec<(String, String)> {
    (0..n_files)
        .map(|i| {
            let next = (i + 1) % n_files;
            let name = format!("file_{i}.rs");
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn work_{i}() {{}}
                // LINT.ThenChange('//file_{next}.rs')
            "
            ));
            (name, content)
        })
        .collect()
}

/// Generate `n_files` small files with labeled cross-references forming a
/// chain: file_0:label_0 → file_1:label_1 → ... → file_0:label_0.
fn gen_many_small_files_with_labels(n_files: usize) -> Vec<(String, String)> {
    (0..n_files)
        .map(|i| {
            let next = (i + 1) % n_files;
            let name = format!("file_{i}.rs");
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn work_{i}() {{}}
                // LINT.ThenChange('//file_{next}.rs:label_{next}')
                // LINT.Label('label_{i}')
                fn target_{i}() {{}}
                // LINT.EndLabel
            "
            ));
            (name, content)
        })
        .collect()
}

// ─── Benchmarks ───

fn bench_cpu_bound(c: &mut Criterion) {
    let mut group = c.benchmark_group("cpu_bound");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(10);

    for n_blocks in [100, 1_000, 10_000] {
        let (src, tgt) = gen_large_file_pair(n_blocks);
        let files: Vec<(&str, &str)> = vec![("source.rs", &src), ("target.rs", &tgt)];
        let (dir, changes) = setup_scan(&files);

        group.bench_with_input(
            BenchmarkId::new("large_file_pair", n_blocks),
            &n_blocks,
            |b, _| {
                b.iter(|| {
                    check::check(&changes, dir.path(), &[], None);
                });
            },
        );
    }

    group.finish();
}

fn bench_io_bound(c: &mut Criterion) {
    let mut group = c.benchmark_group("io_bound");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(10);

    for n_files in [100, 1_000, 10_000] {
        let file_data = gen_many_small_files(n_files);
        let files: Vec<(&str, &str)> = file_data
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let (dir, changes) = setup_scan(&files);

        group.bench_with_input(
            BenchmarkId::new("many_small_files", n_files),
            &n_files,
            |b, _| {
                b.iter(|| {
                    check::check(&changes, dir.path(), &[], None);
                });
            },
        );
    }

    group.finish();
}

fn bench_label_resolution(c: &mut Criterion) {
    let mut group = c.benchmark_group("label_resolution");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(10);

    for n_files in [100, 1_000, 10_000] {
        let file_data = gen_many_small_files_with_labels(n_files);
        let files: Vec<(&str, &str)> = file_data
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let (dir, changes) = setup_scan(&files);

        group.bench_with_input(
            BenchmarkId::new("labeled_ring", n_files),
            &n_files,
            |b, _| {
                b.iter(|| {
                    check::check(&changes, dir.path(), &[], None);
                });
            },
        );
    }

    group.finish();
}

fn bench_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("scaling");
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
    group.sample_size(10);

    // Fixed total work (~10k directive blocks), varying distribution:
    // few large files vs many small files.
    let configs: &[(&str, usize, usize)] = &[
        ("1x10000", 1, 10_000), // 1 file, 10k blocks
        ("10x1000", 10, 1_000), // 10 files, 1k blocks each
        ("100x100", 100, 100),  // 100 files, 100 blocks each
        ("1000x10", 1_000, 10), // 1k files, 10 blocks each
        ("10000x1", 10_000, 1), // 10k files, 1 block each
    ];

    for &(label, n_files, blocks_per_file) in configs {
        let mut file_data: Vec<(String, String)> = Vec::new();

        for i in 0..n_files {
            let mut content = String::new();
            for j in 0..blocks_per_file {
                let target_file = (i + 1) % n_files;
                content.push_str(&unindent(&format!(
                    "
                    // LINT.IfChange
                    fn work_{i}_{j}() {{}}
                    // LINT.ThenChange('//file_{target_file}.rs')
                "
                )));
            }
            file_data.push((format!("file_{i}.rs"), content));
        }

        let files: Vec<(&str, &str)> = file_data
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let (dir, changes) = setup_scan(&files);

        group.bench_with_input(BenchmarkId::new("files_x_blocks", label), &label, |b, _| {
            b.iter(|| {
                check::check(&changes, dir.path(), &[], None);
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_cpu_bound,
    bench_io_bound,
    bench_label_resolution,
    bench_scaling,
);
criterion_main!(benches);
