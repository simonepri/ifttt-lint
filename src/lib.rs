pub mod check;
pub mod cli;
pub(crate) mod languages;
pub(crate) mod parser;
pub mod reports;
pub mod vcs;
// pub(crate) for normal builds; exposed for benchmarks behind test-util.
#[cfg(not(any(test, feature = "test-util")))]
pub(crate) mod vcs_git;
#[cfg(any(test, feature = "test-util"))]
pub mod vcs_git;

pub use vcs::{ChangeMap, FileChanges};

/// Test/bench only — not part of the public API.
#[cfg(any(test, feature = "test-util"))]
pub mod vcs_mock;
