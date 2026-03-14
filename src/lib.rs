pub mod check;
pub mod cli;
pub(crate) mod languages;
pub(crate) mod parser;
pub mod reports;
pub mod vcs;
pub(crate) mod vcs_git;

pub use vcs::{ChangeMap, FileChanges};

/// Test/bench only — not part of the public API.
#[cfg(any(test, feature = "test-util"))]
pub mod vcs_mock;
