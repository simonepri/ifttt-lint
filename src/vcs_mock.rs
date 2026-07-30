use std::collections::HashMap;

use anyhow::Result;

use crate::vcs::{ChangeMap, FileContent, VcsProvider};
use crate::vcs_none::{probe_is_binary, BINARY_PROBE_BYTES};

/// In-memory VcsProvider for tests. Stores raw bytes and classifies them on
/// read with the same probe as the real backends, so undecodable and binary
/// content behave faithfully.
pub struct MockVcsProvider {
    files: HashMap<String, Vec<u8>>,
    diff: ChangeMap,
    suppression: Option<String>,
    validate_files: Vec<String>,
    strict: bool,
}

impl Default for MockVcsProvider {
    fn default() -> Self {
        Self {
            files: HashMap::new(),
            diff: ChangeMap::new(),
            suppression: None,
            validate_files: Vec::new(),
            strict: true,
        }
    }
}

impl MockVcsProvider {
    pub fn add_file(&mut self, rel_path: &str, content: &str) {
        self.add_file_bytes(rel_path, content.as_bytes());
    }

    pub fn add_file_bytes(&mut self, rel_path: &str, content: &[u8]) {
        self.files.insert(rel_path.to_string(), content.to_vec());
    }

    pub fn set_diff(&mut self, diff: ChangeMap) {
        self.diff = diff;
    }

    pub fn set_suppression(&mut self, reason: &str) {
        self.suppression = Some(reason.to_string());
    }

    pub fn set_validate_files(&mut self, paths: &[&str]) {
        self.validate_files = paths.iter().map(|s| s.to_string()).collect();
    }

    pub fn set_strict(&mut self, enabled: bool) {
        self.strict = enabled;
    }
}

impl VcsProvider for MockVcsProvider {
    fn diff(&self) -> Result<ChangeMap> {
        Ok(self.diff.clone())
    }

    fn suppressions(&self) -> Result<Option<String>> {
        Ok(self.suppression.clone())
    }

    fn read_file(&self, rel_path: &str) -> Result<Option<FileContent>> {
        let Some(bytes) = self.files.get(rel_path) else {
            return Ok(None);
        };
        if probe_is_binary(probe_head(bytes)) {
            return Ok(Some(FileContent::Binary));
        }
        let text = String::from_utf8(bytes.clone())
            .map_err(|e| anyhow::anyhow!(e).context(format!("read {rel_path}")))?;
        Ok(Some(FileContent::Text(text)))
    }

    fn read_file_bytes(&self, rel_path: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .files
            .get(rel_path)
            .filter(|bytes| !probe_is_binary(probe_head(bytes)))
            .cloned())
    }

    fn file_exists(&self, rel_path: &str) -> Result<bool> {
        Ok(self.files.contains_key(rel_path))
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
        &self.validate_files
    }

    fn search_string_in_files(&self, needle: &str) -> Result<Vec<String>> {
        let mut paths: Vec<String> = self
            .files
            .iter()
            .filter(|(_path, bytes)| {
                // Empty-needle guard: `windows(0)` panics, while an empty
                // pattern matches everything (as in `str::contains`).
                needle.is_empty()
                    || bytes
                        .windows(needle.len())
                        .any(|window| window == needle.as_bytes())
            })
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort();
        Ok(paths)
    }
}

/// The probe window of `bytes` — the whole content when shorter.
fn probe_head(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len().min(BINARY_PROBE_BYTES)]
}
