//! # Line-Indexed File (LIF) State Management
//!
//! This module implements the core logic for the LIF-Patch protocol, a robust mechanism
//! for allowing LLMs to edit files. The protocol is designed to be resilient against
//! common LLM failures, such as acting on stale information from earlier in a conversation.
//!
//! ## Guiding Principles
//!
//! 1.  **Offload Complexity**: The LLM's job is simple: generate a structured JSON patch.
//!     All complex logic (state tracking, ID generation, patch application) is handled by this module.
//! 2.  **Guarantee Consistency**: A hash-based verification system ensures that patches are only
//!     applied to the version of the file the LLM thinks it's editing, preventing corruption.
//! 3.  **Token Efficiency**: After the initial file read, edits are described compactly, saving tokens.
//!
//! ## Core Components
//!
//! -   **`FileState`**: Represents a single file in memory. It breaks the file into lines, each
//!     assigned a stable Line Identifier (LID).
//! -   **`FileStateManager`**: A singleton that acts as a cache, holding the `FileState` for
//!     all "open" files for the duration of a request.
//! -   **`PatchOperation`**: A set of commands (`insert`, `replace_range`) that describe an edit.
//! -   **`lif_hash`**: A SHA-1 hash of the file's LIF representation, acting as a version identifier.

use crate::patch::PatchOperation;
use anyhow::{Result, anyhow};
use sha1::{Digest, Sha1};
use crate::diff;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

/// The default gap between Line Identifiers (LIDs) when a file is first read.
///
/// ### Reasoning
/// Using large, gapped integers (e.g., 1000, 2000, 3000) provides ample "space"
/// for future insertions between any two existing lines without requiring a re-indexing
/// of the entire file. This ensures that LIDs remain stable throughout the editing session,
/// which is critical for the LLM's ability to reference lines reliably.
pub const STARTING_LID_GAP: u64 = 1000;

/// Represents the in-memory state of a single "open" file using the LIF protocol.
#[derive(Debug)]
pub struct FileState {
    /// The absolute, canonicalized path to the file on disk.
    pub path: PathBuf,
    /// The core of the LIF representation: a sorted map of LID -> line content.
    ///
    /// ### Reasoning
    /// A `BTreeMap` is used because it keeps the lines sorted by LID automatically, which
    /// makes it efficient to reconstruct the file and to find ranges for patching operations.
    pub lines: BTreeMap<u64, String>,
    /// The current SHA-1 hash of the LIF content, used for state synchronization.
    /// This hash acts as a version identifier for the file's state.
    pub lif_hash: String,
}

/// Generates a colorized, human-readable diff between the old and new file states.
impl FileState {
    /// Creates a new `FileState` from a file path and its raw string content.
    /// This function generates the initial LIDs and computes the first hash.
    pub fn new(path: PathBuf, content: &str) -> Self {
        let mut lines = BTreeMap::new();
        // Use split to correctly handle trailing newlines, but handle the empty
        // string case explicitly, as "".split() produces one empty string element.
        if !content.is_empty() {
            for (i, line_content) in content.split('\n').enumerate() {
                let lid = (i as u64 + 1) * STARTING_LID_GAP;
                lines.insert(lid, line_content.to_string());
            }
        }

        let mut initial_state = Self {
            path,
            lines,
            lif_hash: String::new(),
        };

        let lif_content = initial_state.get_lif_content_for_hashing();
        initial_state.lif_hash = Self::calculate_hash(&lif_content);
        initial_state
    }

    /// Applies a series of patch operations to the file state.
    ///
    /// ### Reasoning
    /// This method is transactional within a single patch request. It operates on a clone
    /// of the lines (`temp_lines`). This ensures that if any single operation in the patch fails,
    /// the original state is preserved and not left in a partially modified, inconsistent state.
    /// After all operations are successfully applied to the clone, the original `lines` are
    /// swapped with the new state, and the `lif_hash` is recalculated.
    pub fn apply_patch(&mut self, patch: &[PatchOperation]) -> Result<()> {
        let mut temp_lines = self.lines.clone();

        for operation in patch {
            match operation {
                PatchOperation::Insert(op) => {
                    let new_lids =
                        Self::generate_new_lids(&temp_lines, &op.after_lid, op.content.len())?;
                    for (lid, line_content) in new_lids.iter().zip(op.content.iter()) {
                        temp_lines.insert(*lid, line_content.clone());
                    }
                }
                PatchOperation::Replace(op) => {
                    let start_lid_num = Self::parse_lid(&op.start_lid)?;
                    let end_lid_num = Self::parse_lid(&op.end_lid)?;

                    if !temp_lines.contains_key(&start_lid_num) {
                        return Err(anyhow!(
                            "start_lid '{}' does not exist in file",
                            op.start_lid
                        ));
                    }
                    if !temp_lines.contains_key(&end_lid_num) {
                        return Err(anyhow!("end_lid '{}' does not exist in file", op.end_lid));
                    }
                    if start_lid_num > end_lid_num {
                        return Err(anyhow!(
                            "start_lid '{}' cannot be numerically greater than end_lid '{}'",
                            op.start_lid,
                            op.end_lid
                        ));
                    }

                    let keys_to_remove: Vec<_> = temp_lines
                        .keys()
                        .filter(|&&k| k >= start_lid_num && k <= end_lid_num)
                        .copied()
                        .collect();

                    for k in keys_to_remove {
                        temp_lines.remove(&k);
                    }

                    let after_lid_for_insert = temp_lines
                        .range(..start_lid_num)
                        .next_back()
                        .map(|(k, _)| format!("LID{k}"))
                        .unwrap_or_else(|| "_START_OF_FILE_".to_string());

                    let new_lids = Self::generate_new_lids(
                        &temp_lines,
                        &after_lid_for_insert,
                        op.content.len(),
                    )?;

                    for (lid, line_content) in new_lids.iter().zip(op.content.iter()) {
                        temp_lines.insert(*lid, line_content.clone());
                    }
                }
            }
        }

        self.lines = temp_lines;
        let new_lif_content = self.get_lif_content_for_hashing();
        self.lif_hash = Self::calculate_hash(&new_lif_content);

        Ok(())
    }

    /// Applies the patch, writes the changes to disk, and returns a diff.
    pub fn apply_and_write_patch(&mut self, patch: &[PatchOperation]) -> Result<String> {
        let old_lines = self.lines.clone();
        self.apply_patch(patch)?; // This updates self.lines and self.lif_hash

        let diff = diff::generate_custom_diff(&old_lines, &self.lines);
        let final_content = self.get_full_content();

        fs::write(&self.path, &final_content)?;

        Ok(diff)
    }

    /// Reconstructs the full file content by joining the lines, without any LIF metadata.
    /// This is used to write the final content back to disk.
    pub fn get_full_content(&self) -> String {
        self.lines
            .values()
            .cloned()
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Generates the complete LIF representation of the file to be sent to the LLM.
    /// This includes the header with the file path and the crucial `lif_hash`.
    pub fn get_lif_representation(&self) -> String {
        self.get_lif_string_for_range(None, None)
    }

    /// Generates a LIF representation for a specific line range.
    ///
    /// This is the canonical way to display file content to the LLM. It generates
    /// a consistent header and formats the selected lines with their LIDs.
    pub fn get_lif_string_for_range(
        &self,
        start_line: Option<usize>,
        end_line: Option<usize>,
    ) -> String {
        let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let relative_path = self.path.strip_prefix(&project_root).unwrap_or(&self.path);
        let short_hash = self.get_short_hash();

        if self.lines.is_empty() {
            return format!(
                "File: {} | Hash: {} | Lines: 0-0/0\n[File is empty]",
                relative_path.display(),
                short_hash,
            );
        }

        let line_count = self.lines.len();
        let start_line_num = start_line.unwrap_or(1);
        let end_line_num = end_line.unwrap_or(line_count);

        let header = format!(
            "File: {} | Hash: {} | Lines: {start_line_num}-{end_line_num}/{line_count}",
            relative_path.display(),
            short_hash
        );

        let body = self
            .lines
            .iter()
            .enumerate()
            .skip(start_line_num.saturating_sub(1))
            .take(end_line_num - start_line_num + 1)
            .map(|(i, (lid, content))| {
                let line_num = i + 1;
                format!("{line_num:<5}LID{lid}: {content}")
            })
            .collect::<Vec<String>>()
            .join("\n");

        format!("{header}\n{body}")
    }

    /// Extracts the content of lines within a given LID range, inclusive.
    pub fn get_lines_in_range(
        &self,
        start_lid_str: &str,
        end_lid_str: &str,
    ) -> Result<Vec<String>> {
        let start_lid = Self::parse_lid(start_lid_str)?;
        let end_lid = Self::parse_lid(end_lid_str)?;

        if start_lid > end_lid {
            return Err(anyhow!(
                "start_lid '{start_lid_str}' cannot be after end_lid '{end_lid_str}'"
            ));
        }

        let lines_in_range: Vec<String> = self
            .lines
            .range(start_lid..=end_lid)
            .map(|(_, content)| content.clone())
            .collect();

        // This check is important. An empty result can be valid (e.g., copying an empty range),
        // but we should error if the LIDs themselves were not found in the file, which indicates
        // a more serious logic error from the AI.
        if lines_in_range.is_empty() {
            if !self.lines.contains_key(&start_lid) && start_lid_str != "_START_OF_FILE_" {
                return Err(anyhow!("start_lid '{start_lid_str}' not found in file."));
            }
            if !self.lines.contains_key(&end_lid) {
                return Err(anyhow!("end_lid '{end_lid_str}' not found in file."));
            }
        }

        Ok(lines_in_range)
    }

    /// A simple helper to compute the SHA-1 hash of a string.
    fn calculate_hash(content: &str) -> String {
        let mut hasher = Sha1::new();
        hasher.update(content.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Returns the truncated 8-character version of the LIF hash.
    pub fn get_short_hash(&self) -> &str {
        &self.lif_hash[..8.min(self.lif_hash.len())]
    }

    /// Generates the canonical string content that is used for hashing.
    /// It's crucial that this format is consistent.
    fn get_lif_content_for_hashing(&self) -> String {
        self.lines
            .iter()
            .map(|(lid, content)| format!("LID{lid}: {content}"))
            .collect::<Vec<String>>()
            .join("\n")
    }

    /// Parses a string like "LID1234" into its numeric form `1234`.
    pub fn parse_lid(lid_str: &str) -> Result<u64> {
        if !lid_str.starts_with("LID") {
            return Err(anyhow!("Invalid LID format: {lid_str}"));
        }
        lid_str[3..]
            .parse::<u64>()
            .map_err(|_| anyhow!("Invalid LID number: {lid_str}"))
    }

    /// Calculates new LIDs for an insertion operation.
    ///
    /// ### Reasoning
    /// This is the core of LID generation. To insert `N` lines after a given LID,
    /// it finds the space between the `after_lid` and the `next_lid`. It then divides
    /// this space by `N+1` to find an even `step`, and places the new LIDs at these
    /// stepped intervals (e.g., `after_lid + step`, `after_lid + 2*step`, ...).
    /// This "integer averaging" approach ensures new lines can always be inserted
    /// as long as there is a gap of at least `N` between two LIDs.
    /// It also handles the edge cases of inserting at the beginning (`_START_OF_FILE_`)
    /// or end of the file.
    fn generate_new_lids(
        lines: &BTreeMap<u64, String>,
        after_lid_str: &str,
        count: usize,
    ) -> Result<Vec<u64>> {
        let mut new_lids = Vec::with_capacity(count);

        if after_lid_str == "_START_OF_FILE_" {
            let next_lid = lines.keys().next().copied().unwrap_or(STARTING_LID_GAP);
            let step = next_lid / (count as u64 + 1);

            if step == 0 {
                return Err(anyhow!(
                    "Not enough space to insert at the beginning of the file before LID{next_lid}."
                ));
            }

            for i in 1..=count {
                new_lids.push(i as u64 * step);
            }
        } else {
            let after_lid = Self::parse_lid(after_lid_str)?;
            if !lines.contains_key(&after_lid) {
                return Err(anyhow!("LID not found: {after_lid_str}"));
            }

            let next_lid = lines
                .range(after_lid + 1..)
                .next()
                .map(|(&k, _)| k)
                .unwrap_or_else(|| after_lid + STARTING_LID_GAP);

            let range = next_lid - after_lid;
            let step = range / (count as u64 + 1);

            if step == 0 {
                return Err(anyhow!(
                    "Cannot insert {count} lines between LID{after_lid} and LID{next_lid}. Not enough space."
                ));
            }

            for i in 1..=count {
                new_lids.push(after_lid + i as u64 * step);
            }
        }
        Ok(new_lids)
    }
}

/// Manages all "open" `FileState` instances for the duration of a request.
///
/// ### Reasoning
/// This acts as a cache. If a file is read or edited multiple times in a single
/// user request, we don't need to re-read it from disk or re-generate the LIDs.
/// Using the canonical file path as the key ensures that different relative paths
/// pointing to the same file are treated as the same entry.
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
        let canonical_path = fs::canonicalize(path_str)?;
        let canonical_key = canonical_path.to_string_lossy().to_string();

        if !self.open_files.contains_key(&canonical_key) {
            if !canonical_path.is_file() {
                return Err(anyhow!("Path is not a file: {}", canonical_path.display()));
            }
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
        let canonical_path = fs::canonicalize(path_str)?;
        let canonical_key = canonical_path.to_string_lossy().to_string();

        if !canonical_path.is_file() {
            return Err(anyhow!("Path is not a file: {}", canonical_path.display()));
        }

        let content = fs::read_to_string(&canonical_path)?;
        let file_state = FileState::new(canonical_path, &content);
        self.open_files.insert(canonical_key.clone(), file_state);

        Ok(self.open_files.get_mut(&canonical_key).unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::style;
    use crate::patch::{InsertOperation, ReplaceOperation};
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
    fn test_file_state_new() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
        let state = FileState::new(file_path, "line 1\nline 2");

        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines.get(&1000), Some(&"line 1".to_string()));
        assert_eq!(state.lines.get(&2000), Some(&"line 2".to_string()));

        let expected_lif_content = "LID1000: line 1\nLID2000: line 2";
        let expected_hash = FileState::calculate_hash(expected_lif_content);
        assert_eq!(state.lif_hash, expected_hash);
    }

    #[test]
    fn test_get_lif_representation_new_format() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
        let state = FileState::new(file_path.clone(), "line 1\nline 2");
        let representation = state.get_lif_representation();

        let project_root = std::env::current_dir().unwrap();
        let relative_path = file_path.strip_prefix(&project_root).unwrap_or(&file_path);
        let short_hash = state.get_short_hash();

        let expected_header = format!(
            "File: {} | Hash: {} | Lines: 1-2/2",
            relative_path.display(),
            short_hash
        );
        let expected_body = "1    LID1000: line 1\n2    LID2000: line 2";
        assert_eq!(
            representation,
            format!("{expected_header}\n{expected_body}")
        );
    }

    #[test]
    fn test_get_lif_string_for_range() {
        let (_tmp_dir, file_path) = setup_test_file("1\n2\n3\n4\n5");
        let state = FileState::new(file_path.clone(), "1\n2\n3\n4\n5");

        let partial_representation = state.get_lif_string_for_range(Some(2), Some(4));

        let project_root = std::env::current_dir().unwrap();
        let relative_path = file_path.strip_prefix(&project_root).unwrap_or(&file_path);
        let short_hash = state.get_short_hash();

        let expected_header = format!(
            "File: {} | Hash: {} | Lines: 2-4/5",
            relative_path.display(),
            short_hash
        );
        let expected_body = "2    LID2000: 2\n3    LID3000: 3\n4    LID4000: 4";
        assert_eq!(
            partial_representation,
            format!("{expected_header}\n{expected_body}")
        );
    }

    #[test]
    fn test_apply_and_write_patch() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut state = FileState::new(file_path.clone(), "line 1\nline 3");

        let patch = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "LID1000".to_string(),
            content: vec!["line 2".to_string()],
            context_before: None,
            context_after: None,
        })];

        let diff = state.apply_and_write_patch(&patch).unwrap();

        // Check file on disk
        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");

        // Check in-memory state
        assert_eq!(state.get_full_content(), "line 1\nline 2\nline 3");

        // Check diff
        assert!(diff.contains(&style("+ LID1500: line 2".to_string()).green().to_string()));
    }

    #[test]
    fn test_get_full_content() {
        let (_tmp_dir, file_path) = setup_test_file("one\ntwo");
        let state = FileState::new(file_path, "one\ntwo");
        assert_eq!(state.get_full_content(), "one\ntwo");
    }

    #[test]
    fn test_patch_insert_at_start() {
        let (_tmp_dir, file_path) = setup_test_file("line 1");
        let mut state = FileState::new(file_path, "line 1");
        let original_hash = state.lif_hash.clone();

        let patch = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "_START_OF_FILE_".to_string(),
            content: vec!["new first line".to_string()],
            context_before: None,
            context_after: Some("line 1".to_string()),
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines.get(&500), Some(&"new first line".to_string()));
        assert_ne!(state.lif_hash, original_hash);
        assert_eq!(state.get_full_content(), "new first line\nline 1");
    }

    #[test]
    fn test_patch_insert_in_middle() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2");
        let mut state = FileState::new(file_path, "line 1\nline 2");

        let patch = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "LID1000".to_string(),
            content: vec!["new middle line".to_string()],
            context_before: Some("line 1".to_string()),
            context_after: Some("line 2".to_string()),
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.lines.get(&1500), Some(&"new middle line".to_string()));
        assert_eq!(state.get_full_content(), "line 1\nnew middle line\nline 2");
    }

    #[test]
    fn test_patch_delete_range() {
        let content = "line 1\nline 2\nline 3\nline 4";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID2000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec![],
            context_before: Some("line 1".to_string()),
            context_after: Some("line 4".to_string()),
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.get_full_content(), "line 1\nline 4");
    }

    #[test]
    fn test_patch_replace_range() {
        let content = "line 1\nline 2\nline 3\nline 4";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID2000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["replacement".to_string()],
            context_before: Some("line 1".to_string()),
            context_after: Some("line 4".to_string()),
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.get_full_content(), "line 1\nreplacement\nline 4");
    }

    #[test]
    fn test_state_manager_caching() {
        let (_tmp_dir, file_path) = setup_test_file("initial");
        let mut manager = FileStateManager::new();

        // First open, reads from disk
        let state1 = manager.open_file(file_path.to_str().unwrap()).unwrap();
        assert_eq!(state1.get_full_content(), "initial");
        let patch = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "LID1000".to_string(),
            content: vec![" new".to_string()],
            context_before: Some("initial".to_string()),
            context_after: None,
        })];
        state1.apply_patch(&patch).unwrap();
        assert_eq!(state1.get_full_content(), "initial\n new");

        // Second open, should be cached and reflect changes
        let state2 = manager.open_file(file_path.to_str().unwrap()).unwrap();
        assert_eq!(state2.get_full_content(), "initial\n new");
    }

    #[test]
    fn test_deserialize_patch_operation() {
        let json_data = r#"
        [
            {
                "op": "r",
                "start_lid": "LID1000",
                "end_lid": "LID2000",
                "content": ["new content"],
                "context_before": "optional context",
                "context_after": null
            },
            {
                "op": "i",
                "after_lid": "LID3000",
                "content": ["inserted line 1", "inserted line 2"],
                "context_before": null,
                "context_after": "optional context 2"
            }
        ]
        "#;
        let operations: Vec<PatchOperation> = serde_json::from_str(json_data).unwrap();
        assert_eq!(operations.len(), 2);
        assert_eq!(
            operations[0],
            PatchOperation::Replace(ReplaceOperation {
                start_lid: "LID1000".to_string(),
                end_lid: "LID2000".to_string(),
                content: vec!["new content".to_string()],
                context_before: Some("optional context".to_string()),
                context_after: None
            })
        );
        assert_eq!(
            operations[1],
            PatchOperation::Insert(InsertOperation {
                after_lid: "LID3000".to_string(),
                content: vec!["inserted line 1".to_string(), "inserted line 2".to_string()],
                context_before: None,
                context_after: Some("optional context 2".to_string())
            })
        );
    }

    #[test]
    fn test_edit_same_file_thrice_sequentially() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        // First patch
        let patch1 = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "LID1000".to_string(),
            content: vec!["inserted after 1".to_string()],
            context_before: Some("line 1".to_string()),
            context_after: Some("line 2".to_string()),
        })];
        state.apply_patch(&patch1).unwrap();

        assert_eq!(state.lines.len(), 4);
        assert_eq!(
            state.get_full_content(),
            "line 1\ninserted after 1\nline 2\nline 3"
        );

        // Second patch
        let patch2 = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID2000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["replacement".to_string()],
            context_before: Some("inserted after 1".to_string()),
            context_after: None,
        })];
        state.apply_patch(&patch2).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(
            state.get_full_content(),
            "line 1\ninserted after 1\nreplacement"
        );

        // Third patch
        let patch3 = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "LID1500".to_string(), // This was the LID for "inserted after 1"
            content: vec!["another insertion".to_string()],
            context_before: Some("inserted after 1".to_string()),
            context_after: Some("replacement".to_string()),
        })];
        state.apply_patch(&patch3).unwrap();

        assert_eq!(state.lines.len(), 4);
        assert_eq!(
            state.get_full_content(),
            "line 1\ninserted after 1\nanother insertion\nreplacement"
        );
    }


    #[test]
    fn test_patch_replace_invalid_range_start_after_end() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID3000".to_string(),
            end_lid: "LID1000".to_string(),
            content: vec!["new".to_string()],
            context_before: None,
            context_after: None,
        })];

        let result = state.apply_patch(&patch);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot be numerically greater than")
        );
    }

    #[test]
    fn test_patch_replace_non_existent_start_lid() {
        let content = "line 1\nline 2";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID999".to_string(), // Does not exist
            end_lid: "LID2000".to_string(),
            content: vec!["new".to_string()],
            context_before: None,
            context_after: None,
        })];

        let result = state.apply_patch(&patch);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("start_lid 'LID999' does not exist")
        );
    }

    #[test]
    fn test_patch_replace_non_existent_end_lid() {
        let content = "line 1\nline 2";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID1000".to_string(),
            end_lid: "LID9999".to_string(), // Does not exist
            content: vec!["new".to_string()],
            context_before: None,
            context_after: None,
        })];

        let result = state.apply_patch(&patch);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("end_lid 'LID9999' does not exist")
        );
    }

    #[test]
    fn test_error_on_lid_space_exhaustion() {
        let mut lines = BTreeMap::new();
        lines.insert(1000, "line 1".to_string());
        lines.insert(1002, "line 2".to_string()); // Only 1 space between LIDs

        // Try to insert 2 lines, which requires 3 slots (step would be 0)
        let result = FileState::generate_new_lids(&lines, "LID1000", 2);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot insert 2 lines between LID1000 and LID1002. Not enough space."
        );
    }

    #[test]
    fn test_generate_new_lids_at_start_with_no_space() {
        let mut lines = BTreeMap::new();
        lines.insert(1, "line 1".to_string()); // A very small starting LID

        // Try to insert a line at the start. The step will be 1 / (1+1) = 0.
        let result = FileState::generate_new_lids(&lines, "_START_OF_FILE_", 1);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Not enough space to insert at the beginning")
        );
    }

    #[test]
    fn test_patch_replace_first_line() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID1000".to_string(),
            end_lid: "LID1000".to_string(),
            content: vec!["new first".to_string()],
            context_before: None,
            context_after: None,
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.get_full_content(), "new first\nline 2\nline 3");
    }

    #[test]
    fn test_patch_replace_last_line() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID3000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["new last".to_string()],
            context_before: None,
            context_after: None,
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.get_full_content(), "line 1\nline 2\nnew last");
    }

    #[test]
    fn test_patch_replace_entire_file() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID1000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["all new".to_string()],
            context_before: None,
            context_after: None,
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 1);
        assert_eq!(state.get_full_content(), "all new");
    }

    #[test]
    fn test_patch_delete_entire_file() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Replace(ReplaceOperation {
            start_lid: "LID1000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec![],
            context_before: None,
            context_after: None,
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 0);
        assert_eq!(state.get_full_content(), "");
    }

    #[test]
    fn test_patch_insert_at_end() {
        let content = "line 1\nline 2";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Insert(InsertOperation {
            after_lid: "LID2000".to_string(),
            content: vec!["new last line".to_string()],
            context_before: None,
            context_after: None,
        })];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        // The new lid should be halfway between 2000 and the synthetic next lid (2000 + 1000).
        assert_eq!(state.lines.get(&2500), Some(&"new last line".to_string()));
        assert_eq!(state.get_full_content(), "line 1\nline 2\nnew last line");
    }

    #[test]
    fn test_parse_lid_invalid_formats() {
        assert!(FileState::parse_lid("foo").is_err());
        assert!(FileState::parse_lid("LID").is_err());
        assert!(FileState::parse_lid("LID-123").is_err());
        assert!(FileState::parse_lid("LID123a").is_err());
    }

    #[test]
    fn test_deserialize_malformed_patch_operation() {
        // Unknown operation code
        let json_unknown_op = r#"[{"op": "d", "afterLid": "LID1000"}]"#;
        let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_unknown_op);
        assert!(result.is_err());

        // Missing required field for "r"
        let json_missing_field_r = r#"[{"op": "r", "startLid": "LID1000"}]"#;
        let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_missing_field_r);
        assert!(result.is_err());

        // Missing required field for "i"
        let json_missing_field_i = r#"[{"op": "i"}]"#;
        let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_missing_field_i);
        assert!(result.is_err());
    }

    #[test]
    fn test_file_state_new_empty_file() {
        let (_tmp_dir, file_path) = setup_test_file("");
        let state = FileState::new(file_path, "");

        assert!(state.lines.is_empty());
        let expected_hash = FileState::calculate_hash("");
        assert_eq!(state.lif_hash, expected_hash);
        assert_eq!(state.get_full_content(), "");
    }

    #[test]
    fn test_file_state_new_with_single_newline() {
        let content = "\n";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let state = FileState::new(file_path, content);

        // A single newline creates two lines: one before, one after.
        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines.get(&1000), Some(&"".to_string()));
        assert_eq!(state.lines.get(&2000), Some(&"".to_string()));
        assert_eq!(state.get_full_content(), "\n");
    }

    #[test]
    fn test_file_state_new_with_trailing_newline() {
        let content = "line 1\n";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let state = FileState::new(file_path, content);

        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.lines.get(&1000), Some(&"line 1".to_string()));
        assert_eq!(state.lines.get(&2000), Some(&"".to_string()));
        assert_eq!(state.get_full_content(), "line 1\n");
    }
}