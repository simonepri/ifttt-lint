use std::fmt;

use unindent::unindent;

use ifttt_lint::check;
use ifttt_lint::vcs_mock::MockVcsProvider;
use ifttt_lint::{ChangeMap, FileChanges};

fn main() {
    divan::main();
}

/// Mirrors the scan-mode setup in check_test.rs.
fn setup_mock(files: &[(&str, &str)]) -> (MockVcsProvider, ChangeMap) {
    let mut mock = MockVcsProvider::default();
    let mut change_map = ChangeMap::new();

    for (path, content) in files {
        mock.add_file(path, content);

        let n_lines = content.lines().count();
        let mut fc = FileChanges::default();
        fc.added_lines = (1..=n_lines).collect();
        change_map.insert(path.to_string(), fc);
    }

    (mock, change_map)
}

fn gen_large_file_pair(n_blocks: usize) -> (String, String) {
    let mut source = String::with_capacity(n_blocks * 120);
    let mut target = String::with_capacity(n_blocks * 100);

    for i in 0..n_blocks {
        source.push_str(&unindent(&format!(
            "
            // LINT.IfChange(block_{i})
            fn source_{i}() {{ /* logic */ }}
            // LINT.ThenChange(//target.rs:label_{i})
        "
        )));

        target.push_str(&unindent(&format!(
            "
            // LINT.IfChange(label_{i})
            fn target_{i}() {{ /* logic */ }}
            // LINT.ThenChange(//source.rs:block_{i})
        "
        )));
    }

    for i in 0..n_blocks {
        source.push_str(&format!("fn padding_src_{i}() {{}}\n"));
        target.push_str(&format!("fn padding_tgt_{i}() {{}}\n"));
    }

    (source, target)
}

/// Ring topology: file_0 → file_1 → ... → file_0.
fn gen_many_small_files(n_files: usize) -> Vec<(String, String)> {
    (0..n_files)
        .map(|i| {
            let next = (i + 1) % n_files;
            let name = format!("file_{i}.rs");
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn work_{i}() {{}}
                // LINT.ThenChange(//file_{next}.rs)
            "
            ));
            (name, content)
        })
        .collect()
}

/// Labeled ring: file_0:label_0 → file_1:label_1 → ... → file_0:label_0.
fn gen_many_small_files_with_labels(n_files: usize) -> Vec<(String, String)> {
    (0..n_files)
        .map(|i| {
            let next = (i + 1) % n_files;
            let name = format!("file_{i}.rs");
            let prev = (i + n_files - 1) % n_files;
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn work_{i}() {{}}
                // LINT.ThenChange(//file_{next}.rs:label_{next})
                // LINT.IfChange(label_{i})
                fn target_{i}() {{{{}}}}
                // LINT.ThenChange(//file_{prev}.rs)
            "
            ));
            (name, content)
        })
        .collect()
}

/// (n_files, blocks_per_file) — displayed as "NxM".
#[derive(Clone, Copy)]
struct ScalingConfig(usize, usize);

impl fmt::Display for ScalingConfig {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}x{}", self.0, self.1)
    }
}

const SCALING_1X10000: ScalingConfig = ScalingConfig(1, 10_000);
const SCALING_10X1000: ScalingConfig = ScalingConfig(10, 1_000);
const SCALING_100X100: ScalingConfig = ScalingConfig(100, 100);
const SCALING_1000X10: ScalingConfig = ScalingConfig(1_000, 10);
const SCALING_10000X1: ScalingConfig = ScalingConfig(10_000, 1);

/// (n_plain_files, n_lint_files) — displayed as "N_plain_M_lint".
#[derive(Clone, Copy)]
struct RepoSize(usize, usize);

impl fmt::Display for RepoSize {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}_plain_{}_lint", self.0, self.1)
    }
}

const REPO_100_10: RepoSize = RepoSize(100, 10);
const REPO_1000_100: RepoSize = RepoSize(1_000, 100);
const REPO_10000_1000: RepoSize = RepoSize(10_000, 1_000);

mod cpu_bound {
    use super::*;

    #[divan::bench(args = [100, 1_000, 10_000], sample_count = 10)]
    fn large_file_pair(bencher: divan::Bencher, n_blocks: usize) {
        let (src, tgt) = gen_large_file_pair(n_blocks);
        let files: Vec<(&str, &str)> = vec![("source.rs", &src), ("target.rs", &tgt)];
        let (mock, changes) = setup_mock(&files);

        bencher.bench_local(|| {
            check::check(&mock, &changes, &[], 0).unwrap();
        });
    }
}

mod io_bound {
    use super::*;

    #[divan::bench(args = [100, 1_000, 10_000], sample_count = 10)]
    fn many_small_files(bencher: divan::Bencher, n_files: usize) {
        let file_data = gen_many_small_files(n_files);
        let files: Vec<(&str, &str)> = file_data
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let (mock, changes) = setup_mock(&files);

        bencher.bench_local(|| {
            check::check(&mock, &changes, &[], 0).unwrap();
        });
    }
}

mod label_resolution {
    use super::*;

    #[divan::bench(args = [100, 1_000, 10_000], sample_count = 10)]
    fn labeled_ring(bencher: divan::Bencher, n_files: usize) {
        let file_data = gen_many_small_files_with_labels(n_files);
        let files: Vec<(&str, &str)> = file_data
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let (mock, changes) = setup_mock(&files);

        bencher.bench_local(|| {
            check::check(&mock, &changes, &[], 0).unwrap();
        });
    }
}

mod scaling {
    use super::*;

    /// Fixed total work (~10k directive blocks), varying distribution:
    /// few large files vs many small files.
    #[divan::bench(
        args = [SCALING_1X10000, SCALING_10X1000, SCALING_100X100, SCALING_1000X10, SCALING_10000X1],
        sample_count = 10,
    )]
    fn files_x_blocks(bencher: divan::Bencher, cfg: ScalingConfig) {
        let n_files = cfg.0;
        let blocks_per_file = cfg.1;
        let mut file_data: Vec<(String, String)> = Vec::new();

        for i in 0..n_files {
            let mut content = String::new();
            for j in 0..blocks_per_file {
                let target_file = (i + 1) % n_files;
                content.push_str(&unindent(&format!(
                    "
                    // LINT.IfChange
                    fn work_{i}_{j}() {{}}
                    // LINT.ThenChange(//file_{target_file}.rs)
                "
                )));
            }
            file_data.push((format!("file_{i}.rs"), content));
        }

        let files: Vec<(&str, &str)> = file_data
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_str()))
            .collect();
        let (mock, changes) = setup_mock(&files);

        bencher.bench_local(|| {
            check::check(&mock, &changes, &[], 0).unwrap();
        });
    }
}

/// Measures content-filter performance during reverse-lookup.
/// Many deleted files × many surviving LINT files. The compound FileQuery
/// (LINT. AND any-of deleted paths) filters candidates via search_files;
/// surviving files are then parsed and checked for stale references.
mod content_filter {
    use super::*;

    /// (n_deleted, n_survivors) — displayed as "Ndel_Msurv".
    #[derive(Clone, Copy)]
    struct FilterConfig(usize, usize);

    impl fmt::Display for FilterConfig {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{}del_{}surv", self.0, self.1)
        }
    }

    const FILTER_10_1000: FilterConfig = FilterConfig(10, 1_000);
    const FILTER_100_1000: FilterConfig = FilterConfig(100, 1_000);
    const FILTER_1000_1000: FilterConfig = FilterConfig(1_000, 1_000);

    #[divan::bench(
        args = [FILTER_10_1000, FILTER_100_1000, FILTER_1000_1000],
        sample_count = 10,
    )]
    fn many_deleted_targets(bencher: divan::Bencher, cfg: FilterConfig) {
        let n_deleted = cfg.0;
        let n_survivors = cfg.1;

        let mut mock = MockVcsProvider::default();

        // Surviving lint files: each references the next (not deleted).
        // All contain "LINT." so they are candidates for content filtering.
        for i in 0..n_survivors {
            let next = (i + 1) % n_survivors;
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn survivor_{i}() {{}}
                // LINT.ThenChange(//survivor_{next}.rs)
            "
            ));
            mock.add_file(&format!("survivor_{i}.rs"), &content);
        }

        // One referencing file per deleted target (produces findings).
        for i in 0..n_deleted {
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn ref_{i}() {{}}
                // LINT.ThenChange(//deleted_{i}.rs)
            "
            ));
            mock.add_file(&format!("ref_{i}.rs"), &content);
        }

        // All deleted files in the change map.
        let changes: ChangeMap = (0..n_deleted)
            .map(|i| (format!("deleted_{i}.rs"), FileChanges::deleted()))
            .collect();

        bencher.bench_local(|| {
            check::check(&mock, &changes, &[], 0).unwrap();
        });
    }
}

/// `n_plain` files (no LINT), `n_lint` files (LINT but not referencing deleted),
/// one file referencing the deleted target.
mod deleted_target_reverse_lookup {
    use super::*;

    #[divan::bench(args = [REPO_100_10, REPO_1000_100, REPO_10000_1000], sample_count = 10)]
    fn repo_size(bencher: divan::Bencher, cfg: RepoSize) {
        let n_plain = cfg.0;
        let n_lint = cfg.1;

        let mut mock = MockVcsProvider::default();

        for i in 0..n_plain {
            mock.add_file(&format!("plain_{i}.rs"), &format!("fn plain_{i}() {{}}\n"));
        }

        for i in 0..n_lint {
            let next = (i + 1) % n_lint;
            let content = unindent(&format!(
                "
                // LINT.IfChange
                fn lint_{i}() {{}}
                // LINT.ThenChange(//lint_{next}.rs)
            "
            ));
            mock.add_file(&format!("lint_{i}.rs"), &content);
        }

        mock.add_file(
            "referencing.rs",
            &unindent(
                "
                // LINT.IfChange
                fn referencing() {}
                // LINT.ThenChange(//deleted.rs)
            ",
            ),
        );

        let changes: ChangeMap = [("deleted.rs".to_string(), FileChanges::deleted())]
            .into_iter()
            .collect();

        bencher.bench_local(|| {
            check::check(&mock, &changes, &[], 0).unwrap();
        });
    }
}
