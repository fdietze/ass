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

use crate::diff;
use crate::patch::PatchOperation;
use anyhow::{Result, anyhow};
use fractional_index::FractionalIndex;
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Represents a 1-indexed, inclusive range of lines.
#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RangeSpec {
    pub start_line: usize,
    pub end_line: usize,
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
    pub lines: BTreeMap<FractionalIndex, String>,
    /// The current SHA-1 hash of the LIF content, used for state synchronization.
    /// This hash acts as a version identifier for the file's state.
    pub lif_hash: String,
    /// Whether the original file content ended with a newline.
    pub(crate) ends_with_newline: bool,
}

/// Generates a colorized, human-readable diff between the old and new file states.
impl FileState {
    /// Creates a new `FileState` from a file path and its raw string content.
    /// This function generates the initial LIDs and computes the first hash.
    pub fn new(path: PathBuf, content: &str) -> Self {
        let mut lines = BTreeMap::new();
        let mut last_index: Option<FractionalIndex> = None;

        for line_content in content.lines() {
            // `FractionalIndex::new` with `None` for the second argument generates an index after the first.
            let new_index = FractionalIndex::new(last_index.as_ref(), None).unwrap();
            lines.insert(new_index.clone(), line_content.to_string());
            last_index = Some(new_index);
        }

        let mut initial_state = Self {
            path,
            lines,
            lif_hash: String::new(), // Placeholder
            ends_with_newline: content.ends_with('\n'),
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
                    let after_index = if op.after_lid == "_START_OF_FILE_" {
                        None
                    } else {
                        Some(Self::parse_index(&op.after_lid)?)
                    };

                    let next_index = temp_lines
                        .range((
                            after_index
                                .as_ref()
                                .map_or(std::ops::Bound::Unbounded, |k| {
                                    std::ops::Bound::Excluded(k)
                                }),
                            std::ops::Bound::Unbounded,
                        ))
                        .next()
                        .map(|(k, _)| k.clone());

                    let mut last_gen_index = after_index;
                    for line_content in op.content.iter() {
                        let new_index =
                            FractionalIndex::new(last_gen_index.as_ref(), next_index.as_ref())
                                .unwrap();
                        temp_lines.insert(new_index.clone(), line_content.clone());
                        last_gen_index = Some(new_index);
                    }
                }
                PatchOperation::Replace(op) => {
                    let start_index = Self::parse_index(&op.start_lid)?;
                    let end_index = Self::parse_index(&op.end_lid)?;

                    if !temp_lines.contains_key(&start_index) {
                        return Err(anyhow!(
                            "start_lid '{}' does not exist in file",
                            op.start_lid
                        ));
                    }
                    if !temp_lines.contains_key(&end_index) {
                        return Err(anyhow!("end_lid '{}' does not exist in file", op.end_lid));
                    }
                    if start_index > end_index {
                        return Err(anyhow!(
                            "start_lid '{}' cannot be numerically greater than end_lid '{}'",
                            op.start_lid,
                            op.end_lid
                        ));
                    }

                    // Keys to remove
                    temp_lines.retain(|k, _| k < &start_index || k > &end_index);

                    let after_index_for_insert =
                        temp_lines.range(..&start_index).next_back().map(|(k, _)| k);

                    let next_index_for_insert = temp_lines
                        .range(&start_index..)
                        .next()
                        .map(|(k, _)| k.clone());

                    let mut last_gen_index = after_index_for_insert.cloned();
                    for line_content in op.content.iter() {
                        let new_index = FractionalIndex::new(
                            last_gen_index.as_ref(),
                            next_index_for_insert.as_ref(),
                        )
                        .unwrap();
                        temp_lines.insert(new_index.clone(), line_content.clone());
                        last_gen_index = Some(new_index);
                    }
                }
            }
        }

        self.lines = temp_lines;

        // After modifying the lines, we need to update the `ends_with_newline` flag
        // before we recalculate the hash.
        if let Some(last_line) = self.lines.values().last() {
            self.ends_with_newline = last_line.is_empty();
        } else {
            // If there are no lines, it's an empty file. An empty file is treated
            // as not having a trailing newline for hashing purposes.
            self.ends_with_newline = false;
        }

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
        let mut content = self
            .lines
            .values()
            .cloned()
            .collect::<Vec<String>>()
            .join("\n");

        if self.ends_with_newline && !self.lines.is_empty() {
            content.push('\n');
        }

        content
    }

    /// Generates the complete LIF representation of the file to be sent to the LLM.
    /// This includes the header with the file path and the crucial `lif_hash`.
    pub fn get_lif_representation(&self) -> String {
        self.get_lif_string_for_ranges(None)
    }

    /// Generates a LIF representation for specific line ranges.
    ///
    /// This is the canonical way to display file content to the LLM. It generates
    /// a consistent header and formats the selected lines with their LIDs.
    /// If `ranges` is `None` or empty, it renders the entire file.
    pub fn get_lif_string_for_ranges(&self, ranges: Option<&[RangeSpec]>) -> String {
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

        let total_lines = self.lines.len();

        let (lines_header_part, body) = match ranges {
            None | Some([]) => {
                // Full file read
                let header = format!("1-{total_lines}/{total_lines}");
                let content_str = self
                    .lines
                    .iter()
                    .enumerate()
                    .map(|(i, (index, content))| {
                        let line_num = i + 1;
                        format!("{line_num:<5}{}: {content}", index.to_string())
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                (header, content_str)
            }
            Some(merged_ranges) => {
                // Ranged read
                let header = format!(
                    "{}/{}",
                    merged_ranges
                        .iter()
                        .map(|r| format!("{}-{}", r.start_line, r.end_line))
                        .collect::<Vec<_>>()
                        .join(", "),
                    total_lines
                );

                let mut content_parts = Vec::new();
                let all_lines: Vec<_> = self.lines.iter().collect();

                for (i, range) in merged_ranges.iter().enumerate() {
                    if i > 0 {
                        content_parts.push("...".to_string());
                    }
                    // Clamp ranges to be within the bounds of the file
                    let start_idx = range.start_line.saturating_sub(1).min(total_lines);
                    let end_idx = range.end_line.saturating_sub(1).min(total_lines);

                    if start_idx > end_idx {
                        continue;
                    }

                    let range_content = all_lines[start_idx..=end_idx]
                        .iter()
                        .enumerate()
                        .map(|(line_offset, (index, content))| {
                            let line_num = start_idx + line_offset + 1;
                            format!("{line_num:<5}{}: {content}", index.to_string())
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    content_parts.push(range_content);
                }
                (header, content_parts.join("\n"))
            }
        };

        let header = format!(
            "File: {} | Hash: {} | Lines: {}",
            relative_path.display(),
            short_hash,
            lines_header_part
        );

        format!("{header}\n{body}")
    }

    /// Extracts the content of lines within a given LID range, inclusive.
    pub fn get_lines_in_range(
        &self,
        start_lid_str: &str,
        end_lid_str: &str,
    ) -> Result<Vec<String>> {
        let start_lid = Self::parse_index(start_lid_str)?;
        let end_lid = Self::parse_index(end_lid_str)?;

        if start_lid > end_lid {
            return Err(anyhow!(
                "start_lid '{start_lid_str}' cannot be after end_lid '{end_lid_str}'"
            ));
        }

        let lines_in_range: Vec<String> = self
            .lines
            .range(start_lid.clone()..=end_lid.clone())
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
    pub(crate) fn calculate_hash(content: &str) -> String {
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
    pub(crate) fn get_lif_content_for_hashing(&self) -> String {
        let mut content = self
            .lines
            .iter()
            .map(|(index, content)| format!("{}: {}", index.to_string(), content))
            .collect::<Vec<String>>()
            .join("\n");
        if self.ends_with_newline {
            content.push('\n');
        }
        content
    }

    /// Parses a string like "LID1234" into its numeric form `1234`.
    pub fn parse_index(index_str: &str) -> Result<FractionalIndex> {
        FractionalIndex::from_string(index_str)
            .map_err(|_| anyhow!("Invalid index format: {index_str}"))
    }
}
