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
    open_files: HashMap<String, FileState>,
}

impl FileStateManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// The main entry point for accessing a file's state.
    /// If the file is already in the manager, it returns the cached mutable state.
    /// If not, it reads the file from disk, creates a new `FileState`, caches it,
    /// and then returns the new state.
    pub fn open_file(&mut self, path_str: &str) -> Result<&mut FileState> {
        let canonical_path = self.get_canonical_path(path_str)?;
        let canonical_key = canonical_path.to_string_lossy().to_string();

        if !self.open_files.contains_key(&canonical_key) {
            let content = fs::read_to_string(&canonical_path)?;
            let file_state = FileState::new(canonical_path, &content);
            self.open_files.insert(canonical_key.clone(), file_state);
        }

        Ok(self.open_files.get_mut(&canonical_key).unwrap())
    }

    /// Forces a re-read of the file from disk, overwriting any cached state.
    /// This ensures that the returned `FileState` is perfectly up-to-date with
    /// the filesystem, with freshly assigned LIDs.
    pub fn force_reload_file(&mut self, path_str: &str) -> Result<&mut FileState> {
        let canonical_path = self.get_canonical_path(path_str)?;
        let canonical_key = canonical_path.to_string_lossy().to_string();

        let content = fs::read_to_string(&canonical_path)?;
        let file_state = FileState::new(canonical_path, &content);
        self.open_files.insert(canonical_key.clone(), file_state);

        Ok(self.open_files.get_mut(&canonical_key).unwrap())
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
    fn test_state_manager_force_reload() {
        let (_tmp_dir, file_path) = setup_test_file("initial");
        let file_path_str = file_path.to_str().unwrap();
        let mut manager = FileStateManager::new();

        // First open, reads from disk
        let state1 = manager.open_file(file_path_str).unwrap();
        assert_eq!(state1.get_full_content(), "initial");
        let original_hash = state1.lif_hash.clone();

        // Modify file on disk
        fs::write(&file_path, "updated").unwrap();

        // open_file should return the cached version
        let state2 = manager.open_file(file_path_str).unwrap();
        assert_eq!(state2.get_full_content(), "initial");
        assert_eq!(state2.lif_hash, original_hash);

        // force_reload_file should read from disk
        let state3 = manager.force_reload_file(file_path_str).unwrap();
        assert_eq!(state3.get_full_content(), "updated");
        assert_ne!(state3.lif_hash, original_hash);

        // And now a normal open_file should see the reloaded state
        let state4 = manager.open_file(file_path_str).unwrap();
        assert_eq!(state4.get_full_content(), "updated");
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
}
