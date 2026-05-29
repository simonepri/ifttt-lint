use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::vcs::{ChangeMap, FileContent, FileFilter, VcsProvider};

/// Filesystem-backed `VcsProvider` that knows nothing about a VCS.
///
/// Implements only the operations that read raw files from disk and resolve
/// directive paths. The VCS-dependent operations (`diff`, `suppressions`,
/// `search_string_in_files`) return an error — callers that need them must
/// use a real backend that composes `NoneVcsProvider` and overrides those
/// methods (e.g. [`crate::vcs_git::GitVcsProvider`]).
pub struct NoneVcsProvider {
    root: PathBuf,
    strict: bool,
    files: Vec<String>,
}

impl NoneVcsProvider {
    /// `files` is taken verbatim — no glob expansion, symlink filtering, or
    /// path normalization. Callers must pre-process. The git backend does this
    /// in [`crate::vcs_git::GitVcsProvider::new`] before constructing the inner
    /// `NoneVcsProvider`.
    pub fn new(root: PathBuf, strict: bool, files: Vec<String>) -> Self {
        Self {
            root,
            strict,
            files,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

impl VcsProvider for NoneVcsProvider {
    fn diff(&self) -> Result<ChangeMap> {
        anyhow::bail!("diff is not supported by NoneVcsProvider — needs a VCS backend")
    }

    fn suppressions(&self) -> Result<Option<String>> {
        anyhow::bail!("suppressions are not supported by NoneVcsProvider — needs a VCS backend")
    }

    fn read_file(&self, rel_path: &str) -> Result<Option<FileContent>> {
        use std::io::Read;
        let abs = self.root.join(rel_path);
        if abs.metadata().is_ok_and(|m| m.is_dir()) {
            anyhow::bail!("{rel_path} is a directory");
        }
        let mut file = match std::fs::File::open(&abs) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::IsADirectory => {
                anyhow::bail!("{rel_path} is a directory")
            }
            Err(e) => return Err(anyhow::anyhow!(e).context(format!("read {rel_path}"))),
        };
        let mut probe = [0u8; 8192];
        let n = file
            .read(&mut probe)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        let head = &probe[..n];
        if head.contains(&0) || std::str::from_utf8(head).is_err_and(|e| e.error_len().is_some()) {
            return Ok(Some(FileContent::Binary));
        }
        let mut buf = Vec::from(head);
        file.read_to_end(&mut buf)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        let text = String::from_utf8(buf)
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        Ok(Some(FileContent::Text(text)))
    }

    fn file_exists(&self, rel_path: &str) -> Result<bool> {
        let abs = self.root.join(rel_path);
        Ok(abs.metadata().is_ok_and(|m| m.is_file()))
    }

    fn search_string_in_files(
        &self,
        _needle: &str,
        _filter: &FileFilter<'_>,
    ) -> Result<Vec<String>> {
        anyhow::bail!(
            "search_string_in_files is not supported by NoneVcsProvider — needs a VCS backend"
        )
    }

    fn try_resolve_path(&self, raw: &str) -> Result<String, String> {
        if self.strict {
            crate::vcs::strict_resolve_path(raw)
        } else {
            crate::vcs::permissive_resolve_path(raw)
        }
    }

    fn is_strict(&self) -> bool {
        self.strict
    }

    fn validate_files(&self) -> &[String] {
        &self.files
    }
}

#[cfg(test)]
#[path = "vcs_none_test.rs"]
mod tests;
