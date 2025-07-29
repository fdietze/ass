//! # File State Manger
//!
//! This acts as a cache. If a file is read or edited multiple times in a single
//! user request, we don't need to re-read it from disk or re-generate the LIDs.
//! Using the canonical file path as the key ensures that different relative paths
//! pointing to the same file are treated as the same entry.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Result, anyhow};

use crate::file_state::FileState;

#[derive(Default)]
pub struct FileStateManager {
    pub open_files: HashMap<String, FileState>,
}

impl FileStateManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The main entry point for accessing a file's state.
    /// If the file is already in the manager and its content is fresh,
    /// it returns the cached mutable state. Otherwise, it reads the file
    /// from disk, creates a new `FileState`, caches it, and then returns it.
    pub fn open_file(&mut self, path_str: &str) -> Result<&mut FileState> {
        let canonical_path = self.get_canonical_path(path_str)?;
        let canonical_key = canonical_path.to_string_lossy().to_string();

        if self.is_content_stale(&canonical_key, &canonical_path)? {
            let content = fs::read_to_string(&canonical_path)?;
            let file_state = FileState::new(canonical_path, &content);
            self.open_files.insert(canonical_key.clone(), file_state);
        }

        Ok(self.open_files.get_mut(&canonical_key).unwrap())
    }

    /// Retrieves the current state of a file from the manager, mutably.
    pub fn get_file_state_mut(&mut self, path_str: &str) -> Result<&mut FileState> {
        let canonical_path = PathBuf::from(path_str).canonicalize()?;
        self.open_files
            .get_mut(&canonical_path.to_string_lossy().to_string())
            .ok_or_else(|| {
                anyhow!(
                    "File state for '{}' not found in manager. It must be read first.",
                    path_str
                )
            })
    }

    /// Checks if the cached file state is stale compared to the disk.
    /// Returns true if the file is not in the cache or if the content differs.
    fn is_content_stale(&self, key: &str, path: &Path) -> Result<bool> {
        match self.open_files.get(key) {
            Some(cached_state) => {
                let disk_content = fs::read_to_string(path)?;
                // Compare the reconstructed content from the cache with the actual disk content.
                Ok(cached_state.get_full_content() != disk_content)
            }
            None => {
                // Not in cache, so it's "stale" in the sense that we need to load it.
                Ok(true)
            }
        }
    }

    fn get_canonical_path(&self, path_str: &str) -> Result<PathBuf> {
        let path = Path::new(path_str);
        if !path.exists() {
            return Err(anyhow!("Path does not exist: {}", path.display()));
        }
        if !path.is_file() {
            return Err(anyhow!("Path is not a file: {}", path.display()));
        }
        Ok(fs::canonicalize(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::Builder;

    fn setup_test_file(content: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp_dir = Builder::new().prefix("test-fs-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test.txt");
        fs::write(&file_path, content).unwrap();
        (tmp_dir, file_path)
    }

    #[test]
    fn test_file_state_manager_cannot_open_directory() {
        let tmp_dir = Builder::new().prefix("test-fs-dir-").tempdir().unwrap();
        let mut manager = FileStateManager::new();
        let dir_path_str = tmp_dir.path().to_str().unwrap();

        let result = manager.open_file(dir_path_str);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Path is not a file")
        );
    }

    #[test]
    fn test_state_manager_reloads_on_stale_content() {
        let (_tmp_dir, file_path) = setup_test_file("initial");
        let file_path_str = file_path.to_str().unwrap();
        let mut manager = FileStateManager::new();

        // First open, reads from disk
        let state1 = manager.open_file(file_path_str).unwrap();
        assert_eq!(state1.get_full_content(), "initial");
        let original_hash = state1.lif_hash.clone();

        // Modify file on disk
        fs::write(&file_path, "updated").unwrap();

        // `open_file` should now detect the change and reload.
        let state2 = manager.open_file(file_path_str).unwrap();
        assert_eq!(state2.get_full_content(), "updated");
        assert_ne!(state2.lif_hash, original_hash);
    }

    #[test]
    fn test_state_manager_caching() {
        let (_tmp_dir, file_path) = setup_test_file("initial");
        let mut manager = FileStateManager::new();

        // First open, reads from disk
        let state1 = manager.open_file(file_path.to_str().unwrap()).unwrap();
        assert_eq!(state1.get_full_content(), "initial");
        //Fake a patch being applied
        state1.lif_hash = "new_hash".to_string();

        // Second open, should be cached and reflect changes
        let state2 = manager.open_file(file_path.to_str().unwrap()).unwrap();
        assert_eq!(state2.lif_hash, "new_hash");
    }

    #[test]
    fn test_reload_on_manual_edit_after_patch() {
        // This test simulates the user's bug report:
        // 1. A file is edited by the agent.
        // 2. The user manually changes the file on disk.
        // 3. The agent reads the file again and should see the manual changes, not a cached version.

        // Setup a file with a trailing newline, which is crucial for triggering the bug.
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2\n");
        let file_path_str = file_path.to_str().unwrap();
        let mut manager = FileStateManager::new();

        // 1. First "edit", which writes to disk and updates the cache.
        // Due to the bug in `apply_patch`, this will write the file without the trailing newline.
        let initial_state = manager.open_file(file_path_str).unwrap();
        let patch = vec![/* an empty patch will still trigger the rewrite */];
        initial_state.apply_and_write_patch(&patch).unwrap();

        // At this point, the cached state incorrectly believes `ends_with_newline` is false.

        // 2. Manually edit the file on disk to something completely different.
        fs::write(&file_path, "MANUAL EDIT").unwrap();

        // 3. Open the file again. The manager should detect the manual change.
        let reloaded_state = manager.open_file(file_path_str).unwrap();

        // 4. Assert that we have the content from the manual edit, not the cached version.
        assert_eq!(
            reloaded_state.get_full_content(),
            "MANUAL EDIT",
            "Manager should have reloaded the file from disk after a manual edit."
        );
    }
}
