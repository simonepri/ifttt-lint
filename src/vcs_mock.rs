use std::collections::HashMap;

use anyhow::Result;

use crate::vcs::{ChangeMap, FileContent, FileFilter, VcsProvider};

/// In-memory VcsProvider for tests.
pub struct MockVcsProvider {
    files: HashMap<String, String>,
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
        self.files.insert(rel_path.to_string(), content.to_string());
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
        Ok(self.files.get(rel_path).map(|content| {
            if content.as_bytes().iter().take(8192).any(|&b| b == 0) {
                FileContent::Binary
            } else {
                FileContent::Text(content.clone())
            }
        }))
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

    fn search_string_in_files(&self, needle: &str, filter: &FileFilter<'_>) -> Result<Vec<String>> {
        let mut paths: Vec<String> = self
            .files
            .iter()
            .filter(|(_path, content)| content.contains(needle) && filter.matches(content))
            .map(|(path, _)| path.clone())
            .collect();
        paths.sort();
        Ok(paths)
    }
}
