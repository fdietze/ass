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

use anyhow::{Result, anyhow};
use console::style;
use serde::Deserialize;
use serde::de::{self, Deserializer, SeqAccess, Visitor};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
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

/// Defines the elemental operations that can be part of a patch.
///
/// ### Reasoning
/// The operations are designed to be simple for an LLM to generate.
/// - `ReplaceRange` is a powerful primitive that handles modification, insertion, and deletion of
///   contiguous blocks of lines.
/// - `Insert` is a separate, more explicit operation for purely additive changes.
/// - The compact array format `["op_code", ...]` is token-efficient.
#[derive(Debug, PartialEq)]
pub enum PatchOperation {
    /// Replaces a contiguous range of lines with new content.
    ReplaceRange {
        start_lid: String,
        end_lid: String,
        content: Vec<String>,
    },
    /// Inserts new lines after a specific existing line.
    Insert {
        after_lid: String,
        content: Vec<String>,
    },
}

impl<'de> Deserialize<'de> for PatchOperation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PatchOperationVisitor;

        impl<'de> Visitor<'de> for PatchOperationVisitor {
            type Value = PatchOperation;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence representing a patch operation")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let op_code: String = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &self))?;

                match op_code.as_str() {
                    "r" => {
                        let start_lid: String = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let end_lid: String = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        let content: Vec<String> = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(3, &self))?;
                        Ok(PatchOperation::ReplaceRange {
                            start_lid,
                            end_lid,
                            content,
                        })
                    }
                    "i" => {
                        let after_lid: String = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(1, &self))?;
                        let content: Vec<String> = seq
                            .next_element()?
                            .ok_or_else(|| de::Error::invalid_length(2, &self))?;
                        Ok(PatchOperation::Insert { after_lid, content })
                    }
                    _ => Err(de::Error::unknown_variant(&op_code, &["r", "i"])),
                }
            }
        }

        deserializer.deserialize_seq(PatchOperationVisitor)
    }
}

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
fn generate_custom_diff(
    old_lines: &BTreeMap<u64, String>,
    new_lines: &BTreeMap<u64, String>,
) -> String {
    let old_keys: BTreeSet<_> = old_lines.keys().copied().collect();
    let new_keys: BTreeSet<_> = new_lines.keys().copied().collect();

    let modified_keys: BTreeSet<_> = old_keys
        .intersection(&new_keys)
        .filter(|&k| old_lines.get(k) != new_lines.get(k))
        .copied()
        .collect();

    let changed_keys: BTreeSet<_> = old_keys
        .symmetric_difference(&new_keys)
        .copied()
        .collect::<BTreeSet<_>>()
        .union(&modified_keys)
        .copied()
        .collect();

    if changed_keys.is_empty() {
        return "No changes detected.".to_string();
    }

    const CONTEXT_LINES: usize = 2;
    let mut diff_lines = Vec::new();
    let all_keys: Vec<_> = old_keys.union(&new_keys).copied().collect();

    // 1. Identify hunks (contiguous blocks of changed keys)
    let mut hunks: Vec<(usize, usize)> = vec![];
    if !all_keys.is_empty() {
        let mut i = 0;
        while i < all_keys.len() {
            if changed_keys.contains(&all_keys[i]) {
                let start = i;
                while i < all_keys.len() && changed_keys.contains(&all_keys[i]) {
                    i += 1;
                }
                hunks.push((start, i));
            } else {
                i += 1;
            }
        }
    }

    // If there are no hunks, but there are changes, it's a logic error.
    // But if there are no changes, we should have already returned.
    // If there are hunks, proceed.
    if hunks.is_empty() && !changed_keys.is_empty() {
        // This case can happen if, for example, a whitespace-only change is the ONLY change.
        // The old logic for whitespace changes was to treat them as additions, which the new logic
        // will now handle inside the hunk processing. Let's find the modified key.
        if let Some(key) = modified_keys.iter().next() {
            if let Some(idx) = all_keys.iter().position(|&r| r == *key) {
                hunks.push((idx, idx + 1));
            }
        }

        // If still no hunks, it's an unexpected state. Return a simple list of changes.
        if hunks.is_empty() {
            let mut removals = Vec::new();
            let mut additions = Vec::new();
            for key in &changed_keys {
                if let Some(line) = old_lines.get(key) {
                    removals.push(style(format!("- LID{key}: {line}")).red().to_string());
                }
                if let Some(line) = new_lines.get(key) {
                    additions.push(style(format!("+ LID{key}: {line}")).green().to_string());
                }
            }
            removals.extend(additions);
            return removals.join("\n");
        }
    }

    // 2. Render hunks with context
    let mut last_printed_index: Option<usize> = None;

    for (hunk_start, hunk_end) in hunks {
        let context_start = hunk_start.saturating_sub(CONTEXT_LINES);
        let print_start = if let Some(last_idx) = last_printed_index {
            // If the new hunk's context overlaps with or is adjacent to the last one,
            // we start right after the last printed line.
            // Otherwise, we print a separator.
            if context_start > last_idx {
                diff_lines.push("...".to_string());
                context_start
            } else {
                last_idx
            }
        } else {
            context_start
        };

        // Pre-hunk context
        for key in all_keys.iter().take(hunk_start).skip(print_start) {
            if let Some(line) = new_lines.get(key) {
                diff_lines.push(format!("  LID{key}: {line}"));
            }
        }

        // The hunk itself
        let mut hunk_removals = Vec::new();
        let mut hunk_additions = Vec::new();
        for key in all_keys.iter().take(hunk_end).skip(hunk_start) {
            let old_val = old_lines.get(key);
            let new_val = new_lines.get(key);
            match (old_val, new_val) {
                (Some(ov), Some(nv)) => {
                    // Modified
                    let old_normalized: String = ov.split_whitespace().collect();
                    let new_normalized: String = nv.split_whitespace().collect();

                    if old_normalized == new_normalized {
                        hunk_additions.push(format!("  LID{key}: {nv}"));
                    } else {
                        hunk_removals.push(style(format!("- LID{key}: {ov}")).red().to_string());
                        hunk_additions.push(style(format!("+ LID{key}: {nv}")).green().to_string());
                    }
                }
                (Some(ov), None) => {
                    // Deleted
                    hunk_removals.push(style(format!("- LID{key}: {ov}")).red().to_string());
                }
                (None, Some(nv)) => {
                    // Added
                    hunk_additions.push(style(format!("+ LID{key}: {nv}")).green().to_string());
                }
                (None, None) => unreachable!(),
            }
        }
        diff_lines.extend(hunk_removals);
        diff_lines.extend(hunk_additions);

        // Post-hunk context
        let context_end = (hunk_end + CONTEXT_LINES).min(all_keys.len());
        for key in all_keys.iter().take(context_end).skip(hunk_end) {
            if let Some(line) = new_lines.get(key) {
                diff_lines.push(format!("  LID{key}: {line}"));
            }
        }
        last_printed_index = Some(context_end);
    }

    diff_lines.join("\n")
}

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
                PatchOperation::Insert { after_lid, content } => {
                    let new_lids = Self::generate_new_lids(&temp_lines, after_lid, content.len())?;
                    for (lid, line_content) in new_lids.iter().zip(content.iter()) {
                        temp_lines.insert(*lid, line_content.clone());
                    }
                }
                PatchOperation::ReplaceRange {
                    start_lid,
                    end_lid,
                    content,
                } => {
                    let start_lid_num = Self::parse_lid(start_lid)?;
                    let end_lid_num = Self::parse_lid(end_lid)?;

                    if !temp_lines.contains_key(&start_lid_num) {
                        return Err(anyhow!("start_lid '{start_lid}' does not exist in file"));
                    }
                    if !temp_lines.contains_key(&end_lid_num) {
                        return Err(anyhow!("end_lid '{end_lid}' does not exist in file"));
                    }
                    if start_lid_num > end_lid_num {
                        return Err(anyhow!(
                            "start_lid '{start_lid}' cannot be numerically greater than end_lid '{end_lid}'"
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

                    let new_lids =
                        Self::generate_new_lids(&temp_lines, &after_lid_for_insert, content.len())?;

                    for (lid, line_content) in new_lids.iter().zip(content.iter()) {
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

        let diff = generate_custom_diff(&old_lines, &self.lines);
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
            .skip(start_line_num.saturating_sub(1))
            .take(end_line_num - start_line_num + 1)
            .map(|(lid, content)| format!("LID{lid}: {content}"))
            .collect::<Vec<String>>()
            .join("\n");

        format!("{header}\n{body}")
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
    fn parse_lid(lid_str: &str) -> Result<u64> {
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
            let content = fs::read_to_string(&canonical_path).unwrap_or_default();
            let file_state = FileState::new(canonical_path, &content);
            self.open_files.insert(canonical_key.clone(), file_state);
        }

        Ok(self.open_files.get_mut(&canonical_key).unwrap())
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
        let expected_body = "LID1000: line 1\nLID2000: line 2";
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
        let expected_body = "LID2000: 2\nLID3000: 3\nLID4000: 4";
        assert_eq!(
            partial_representation,
            format!("{expected_header}\n{expected_body}")
        );
    }

    #[test]
    fn test_apply_and_write_patch() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut state = FileState::new(file_path.clone(), "line 1\nline 3");

        let patch = vec![PatchOperation::Insert {
            after_lid: "LID1000".to_string(),
            content: vec!["line 2".to_string()],
        }];

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

        let patch = vec![PatchOperation::Insert {
            after_lid: "_START_OF_FILE_".to_string(),
            content: vec!["new first line".to_string()],
        }];
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

        let patch = vec![PatchOperation::Insert {
            after_lid: "LID1000".to_string(),
            content: vec!["new middle line".to_string()],
        }];
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

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID2000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec![],
        }];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 2);
        assert_eq!(state.get_full_content(), "line 1\nline 4");
    }

    #[test]
    fn test_patch_replace_range() {
        let content = "line 1\nline 2\nline 3\nline 4";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID2000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["replacement".to_string()],
        }];
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
        let patch = vec![PatchOperation::Insert {
            after_lid: "LID1000".to_string(),
            content: vec![" new".to_string()],
        }];
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
            ["r", "LID1000", "LID2000", ["new content"]],
            ["i", "LID3000", ["inserted line 1", "inserted line 2"]]
        ]
        "#;
        let operations: Vec<PatchOperation> = serde_json::from_str(json_data).unwrap();
        assert_eq!(operations.len(), 2);
        assert_eq!(
            operations[0],
            PatchOperation::ReplaceRange {
                start_lid: "LID1000".to_string(),
                end_lid: "LID2000".to_string(),
                content: vec!["new content".to_string()]
            }
        );
        assert_eq!(
            operations[1],
            PatchOperation::Insert {
                after_lid: "LID3000".to_string(),
                content: vec!["inserted line 1".to_string(), "inserted line 2".to_string()]
            }
        );
    }

    #[test]
    fn test_edit_same_file_thrice_sequentially() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        // First patch
        let patch1 = vec![PatchOperation::Insert {
            after_lid: "LID1000".to_string(),
            content: vec!["inserted after 1".to_string()],
        }];
        state.apply_patch(&patch1).unwrap();

        assert_eq!(state.lines.len(), 4);
        assert_eq!(
            state.get_full_content(),
            "line 1\ninserted after 1\nline 2\nline 3"
        );

        // Second patch
        let patch2 = vec![PatchOperation::ReplaceRange {
            start_lid: "LID2000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["replacement".to_string()],
        }];
        state.apply_patch(&patch2).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(
            state.get_full_content(),
            "line 1\ninserted after 1\nreplacement"
        );

        // Third patch
        let patch3 = vec![PatchOperation::Insert {
            after_lid: "LID1500".to_string(), // This was the LID for "inserted after 1"
            content: vec!["another insertion".to_string()],
        }];
        state.apply_patch(&patch3).unwrap();

        assert_eq!(state.lines.len(), 4);
        assert_eq!(
            state.get_full_content(),
            "line 1\ninserted after 1\nanother insertion\nreplacement"
        );
    }

    #[test]
    fn test_generate_custom_diff() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "line 1".to_string());
        old_lines.insert(2000, "line 2".to_string());
        old_lines.insert(3000, "line 3".to_string());

        // Case 1: No changes
        let no_change_diff = generate_custom_diff(&old_lines, &old_lines);
        assert_eq!(no_change_diff, "No changes detected.");

        // Case 2: Mix of changes (add, delete, modify)
        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "line 1".to_string()); // Unchanged, not part of hunk
        new_lines.insert(3000, "line 3 modified".to_string()); // Modify
        new_lines.insert(4000, "line 4 added".to_string()); // Add

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // All changes are contiguous in the master key list, so they form one hunk.
        // Removals first, then additions.
        let expected_lines = [
            format!("  LID{}: {}", 1000, "line 1"),
            style(format!("- LID{}: {}", 2000, "line 2"))
                .red()
                .to_string(), // Deletion
            style(format!("- LID{}: {}", 3000, "line 3"))
                .red()
                .to_string(), // Modification (old)
            style(format!("+ LID{}: {}", 3000, "line 3 modified"))
                .green()
                .to_string(), // Modification (new)
            style(format!("+ LID{}: {}", 4000, "line 4 added"))
                .green()
                .to_string(), // Addition
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_whitespace_change() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "  line 1".to_string());
        old_lines.insert(2000, "line 2".to_string()); // Unchanged

        let mut new_lines = old_lines.clone();
        new_lines.insert(1000, "line 1".to_string()); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // The neutral line is treated as an "addition" in the hunk.
        let expected_diff = format!("  LID{}: {}\n  LID{}: {}", 1000, "line 1", 2000, "line 2");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_hunk_ordering() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "line A".to_string());
        old_lines.insert(2000, "line B".to_string()); // To be replaced
        old_lines.insert(3000, "line C".to_string()); // To be replaced
        old_lines.insert(4000, "line D".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "line A".to_string()); // Unchanged
        new_lines.insert(2500, "line X".to_string()); // Replacement
        new_lines.insert(4000, "line D".to_string()); // Unchanged

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "line A"),
            style(format!("- LID{}: {}", 2000, "line B"))
                .red()
                .to_string(),
            style(format!("- LID{}: {}", 3000, "line C"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 2500, "line X"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 4000, "line D"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_generate_custom_diff_multiple_hunks() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "hunk 1 old".to_string());
        old_lines.insert(2000, "unchanged".to_string());
        old_lines.insert(3000, "hunk 2 old".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1500, "hunk 1 new".to_string());
        new_lines.insert(2000, "unchanged".to_string());
        new_lines.insert(3500, "hunk 2 new".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            // Hunk 1
            style(format!("- LID{}: {}", 1000, "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 1500, "hunk 1 new"))
                .green()
                .to_string(),
            // Context
            format!("  LID{}: {}", 2000, "unchanged"),
            // Hunk 2
            style(format!("- LID{}: {}", 3000, "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "hunk 2 new"))
                .green()
                .to_string(),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    // --- Start of added tests for context diff ---

    #[test]
    fn test_diff_with_basic_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context 1".to_string());
        old_lines.insert(2000, "context 2".to_string());
        old_lines.insert(3000, "to be changed".to_string());
        old_lines.insert(4000, "context 3".to_string());
        old_lines.insert(5000, "context 4".to_string());

        let mut new_lines = old_lines.clone();
        new_lines.remove(&3000);
        new_lines.insert(3500, "was changed".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "context 1"),
            format!("  LID{}: {}", 2000, "context 2"),
            style(format!("- LID{}: {}", 3000, "to be changed"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "was changed"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 4000, "context 3"),
            format!("  LID{}: {}", 5000, "context 4"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_hunk_grouping_with_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context 1".to_string());
        old_lines.insert(2000, "to change 1".to_string());
        old_lines.insert(3000, "to change 2".to_string());
        old_lines.insert(4000, "context 2".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "context 1".to_string());
        new_lines.insert(2500, "changed 1".to_string());
        new_lines.insert(2600, "changed 2".to_string());
        new_lines.insert(4000, "context 2".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "context 1"),
            style(format!("- LID{}: {}", 2000, "to change 1"))
                .red()
                .to_string(),
            style(format!("- LID{}: {}", 3000, "to change 2"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 2500, "changed 1"))
                .green()
                .to_string(),
            style(format!("+ LID{}: {}", 2600, "changed 2"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 4000, "context 2"),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_whitespace_only_change_with_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context".to_string());
        old_lines.insert(2000, "  indented".to_string());
        old_lines.insert(3000, "context".to_string());

        let mut new_lines = old_lines.clone();
        new_lines.insert(2000, "indented".to_string()); // Whitespace change

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should just show the new line as context, without +/-
        let expected_lines = [
            format!("  LID{}: {}", 1000, "context"),
            format!("  LID{}: {}", 2000, "indented"),
            format!("  LID{}: {}", 3000, "context"),
        ];
        let expected_diff = expected_lines.join("\n");
        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_with_overlapping_context() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "hunk 1 old".to_string());
        old_lines.insert(2000, "separator".to_string());
        old_lines.insert(3000, "hunk 2 old".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1500, "hunk 1 new".to_string());
        new_lines.insert(2000, "separator".to_string());
        new_lines.insert(3500, "hunk 2 new".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            style(format!("- LID{}: {}", 1000, "hunk 1 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 1500, "hunk 1 new"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 2000, "separator"),
            style(format!("- LID{}: {}", 3000, "hunk 2 old"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "hunk 2 new"))
                .green()
                .to_string(),
        ];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }

    #[test]
    fn test_diff_at_file_boundaries() {
        // Case 1: Change at the beginning
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "to change".to_string());
        old_lines.insert(2000, "context 1".to_string());
        old_lines.insert(3000, "context 2".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1500, "changed".to_string());
        new_lines.insert(2000, "context 1".to_string());
        new_lines.insert(3000, "context 2".to_string());

        let diff_start = generate_custom_diff(&old_lines, &new_lines);
        let expected_start = [
            style(format!("- LID{}: {}", 1000, "to change"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 1500, "changed"))
                .green()
                .to_string(),
            format!("  LID{}: {}", 2000, "context 1"),
            format!("  LID{}: {}", 3000, "context 2"),
        ]
        .join("\n");
        assert_eq!(diff_start, expected_start);

        // Case 2: Change at the end
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context 1".to_string());
        old_lines.insert(2000, "context 2".to_string());
        old_lines.insert(3000, "to change".to_string());

        let mut new_lines = BTreeMap::new();
        new_lines.insert(1000, "context 1".to_string());
        new_lines.insert(2000, "context 2".to_string());
        new_lines.insert(3500, "changed".to_string());

        let diff_end = generate_custom_diff(&old_lines, &new_lines);
        let expected_end = [
            format!("  LID{}: {}", 1000, "context 1"),
            format!("  LID{}: {}", 2000, "context 2"),
            style(format!("- LID{}: {}", 3000, "to change"))
                .red()
                .to_string(),
            style(format!("+ LID{}: {}", 3500, "changed"))
                .green()
                .to_string(),
        ]
        .join("\n");
        assert_eq!(diff_end, expected_end);
    }

    // --- Start of added tests ---

    #[test]
    fn test_patch_replace_invalid_range_start_after_end() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID3000".to_string(),
            end_lid: "LID1000".to_string(),
            content: vec!["new".to_string()],
        }];

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

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID999".to_string(), // Does not exist
            end_lid: "LID2000".to_string(),
            content: vec!["new".to_string()],
        }];

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

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID1000".to_string(),
            end_lid: "LID9999".to_string(), // Does not exist
            content: vec!["new".to_string()],
        }];

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

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID1000".to_string(),
            end_lid: "LID1000".to_string(),
            content: vec!["new first".to_string()],
        }];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.get_full_content(), "new first\nline 2\nline 3");
    }

    #[test]
    fn test_patch_replace_last_line() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID3000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["new last".to_string()],
        }];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 3);
        assert_eq!(state.get_full_content(), "line 1\nline 2\nnew last");
    }

    #[test]
    fn test_patch_replace_entire_file() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID1000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec!["all new".to_string()],
        }];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 1);
        assert_eq!(state.get_full_content(), "all new");
    }

    #[test]
    fn test_patch_delete_entire_file() {
        let content = "line 1\nline 2\nline 3";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::ReplaceRange {
            start_lid: "LID1000".to_string(),
            end_lid: "LID3000".to_string(),
            content: vec![],
        }];
        state.apply_patch(&patch).unwrap();

        assert_eq!(state.lines.len(), 0);
        assert_eq!(state.get_full_content(), "");
    }

    #[test]
    fn test_patch_insert_at_end() {
        let content = "line 1\nline 2";
        let (_tmp_dir, file_path) = setup_test_file(content);
        let mut state = FileState::new(file_path, content);

        let patch = vec![PatchOperation::Insert {
            after_lid: "LID2000".to_string(),
            content: vec!["new last line".to_string()],
        }];
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
        let json_unknown_op = r#"[["d", "LID1000"]]"#;
        let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_unknown_op);
        assert!(result.is_err());

        // Incorrect number of args for "r"
        let json_wrong_args_r = r#"[["r", "LID1000"]]"#;
        let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_wrong_args_r);
        assert!(result.is_err());

        // Incorrect number of args for "i"
        let json_wrong_args_i = r#"[["i"]]"#;
        let result: Result<Vec<PatchOperation>, _> = serde_json::from_str(json_wrong_args_i);
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

    #[test]
    fn test_diff_whitespace_change_is_neutral() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "context before".to_string());
        old_lines.insert(2000, "my_function()".to_string());
        old_lines.insert(3000, "context after".to_string());

        let mut new_lines = old_lines.clone();
        // Change the indentation of the middle line
        new_lines.insert(2000, "  my_function()".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        let expected_lines = [
            format!("  LID{}: {}", 1000, "context before"),
            // The changed line should be neutral, not +/-
            format!("  LID{}: {}", 2000, "  my_function()"),
            format!("  LID{}: {}", 3000, "context after"),
        ]
        .join("\n");

        // Explicitly check that we don't have the add/remove lines
        assert!(!diff.contains("- LID2000: my_function()"));
        assert!(!diff.contains("+ LID2000:   my_function()"));

        assert_eq!(diff, expected_lines);
    }

    #[test]
    fn test_diff_internal_whitespace_is_neutral() {
        let mut old_lines = BTreeMap::new();
        old_lines.insert(1000, "fn my_func  (foo: &str) {}".to_string());

        let mut new_lines = BTreeMap::new();
        // The only change is the double space to a single space
        new_lines.insert(1000, "fn my_func (foo: &str) {}".to_string());

        let diff = generate_custom_diff(&old_lines, &new_lines);

        // Should be treated as a whitespace-only change (neutral)
        let expected_lines = [format!("  LID{}: {}", 1000, "fn my_func (foo: &str) {}")];
        let expected_diff = expected_lines.join("\n");

        assert_eq!(diff, expected_diff);
    }
}
