pub mod check;
pub mod cli;
pub(crate) mod languages;
pub(crate) mod parser;
pub mod reports;
pub mod vcs;
#[doc(hidden)]
pub mod vcs_git;

pub use vcs::{ChangeMap, FileChanges};

#[doc(hidden)]
pub mod vcs_mock;
