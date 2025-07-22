//! # File Patcher Tool
//!
//! This module provides the `edit_file` tool, which is the agent's interface to the
//! LIF-Patch protocol implemented in `file_state.rs`. Its primary responsibilities are:
//!
//! 1.  **Schema Definition**: Defines the JSON schema for the `edit_file` tool. This schema
//!     is sent to the LLM, instructing it on how to format its patch requests. The description
//!     within the schema is critical for the LLM to understand the protocol correctly.
//!
//! 2.  **Request Handling**: Implements `execute_file_patch`, the function that orchestrates
//!     the entire patching process.
//!
//! 3.  **State and Safety Checks**: Before applying a patch, it performs crucial checks:
//!     -   **Path Permissions**: Ensures the target file is within the user-configured `editable_paths`.
//!     -   **Hash Verification**: Compares the `lif_hash` from the LLM's request with the current
//!         hash stored in the `FileState`. This is the most important safety check, preventing
//!         edits on stale file versions.
//!
//! 4.  **Diff Generation**: After a patch is successfully applied, it generates a human-readable
//!     diff of the changes, which is then returned to the LLM and displayed to the user.

use crate::file_state::FileState;
use crate::file_state_manager::FileStateManager;
use crate::patch::{PatchOperation, ReplaceOperation};
use crate::permissions;
use anyhow::{Result, anyhow};
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::{Deserialize, Deserializer};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Represents the arguments for a single file patch operation within a batch.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct PatchArgs {
    /// The path to the file that the patch should be applied to.
    pub file_path: String,
    /// The hash of the file state (`lif_hash`) that the LLM is basing this patch on.
    /// This is the key to preventing state desynchronization.
    pub lif_hash: String,
    /// A sequence of operations that constitute the patch.
    pub patch: Vec<PatchOperation>,
}

/// Represents the arguments for a copy operation.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct CopyArgs {
    /// The path of the file to copy lines from.
    pub source_file_path: String,
    /// The lif_hash of the source file.
    pub source_lif_hash: String,
    /// The starting line identifier of the range to copy.
    pub source_start_lid: String,
    /// The ending line identifier of the range to copy.
    pub source_end_lid: String,
    /// The path of the file to copy lines to. Can be the same as source_file_path.
    pub dest_file_path: String,
    /// The lif_hash of the destination file.
    pub dest_lif_hash: String,
    /// The line identifier in the destination file after which to insert the content.
    pub dest_after_lid: String,
}

/// Represents the arguments for a move operation.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MoveArgs {
    /// The path of the file to move lines from.
    pub source_file_path: String,
    /// The lif_hash of the source file.
    pub source_lif_hash: String,
    /// The starting line identifier of the range to move.
    pub source_start_lid: String,
    /// The ending line identifier of the range to move.
    pub source_end_lid: String,
    /// The path of the file to move lines to. Can be the same as source_file_path.
    pub dest_file_path: String,
    /// The lif_hash of the destination file.
    pub dest_lif_hash: String,
    /// The line identifier in the destination file after which to insert the content.
    pub dest_after_lid: String,
}

/// Helper to deserialize a field that can be `null` into a default value.
fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Represents the arguments for the `edit_file` tool, which can handle multiple file creations and edits.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct FileOperationArgs {
    /// A list of patch operations to be applied to one or more existing files.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub edits: Vec<PatchArgs>,
    /// A list of copy operations.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub copies: Vec<CopyArgs>,
    /// A list of move operations.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub moves: Vec<MoveArgs>,
}

pub fn edit_file_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "edit_file".to_string(),
            description: Some(
                r#"Atomically edits, copies, or moves lines between **existing** files using a line-based protocol (LIF-Patch).

**IMPORTANT Execution Model**: All operations in a single call are **planned** based on the *initial state* of the files. The tool gathers all requested changes and applies them on a per-file basis. This means all LIDs (`source_start_lid`, `dest_after_lid`, etc.) and `lif_hash` values you provide MUST be valid in the files as they were *before* this tool call began.

**Prefer Moves Over Edits**: This is IMPORTANT: Always use the `moves` operation instead of `edits` where possible. For example: moving or extracting code. This avoids LLM spelling mistakes, saves tokens, and is more robust.

**Execution Order**: Operations are planned in this fixed order: 1. `copies`, 2. `moves`, 3. `edits`.

**Strategy for Complex Operations**:
- **Moving Multiple Blocks**: To move separate blocks, provide a separate `moves` object for each. To place them together, use the **exact same `dest_after_lid`** for each `moves` object.
- **Think in Hunks**: For `edits`, prefer replacing a whole logical block (like a function) instead of many small edits.
- **Parantheses**: Pay special attention to parantheses. Will they be still balanced after the edit?

**Patch Details for `edits`**:
- **Replace/Delete**: `{"op":"r", "start_lid":"index1", "end_lid":"index5", "content":["new"]}`. To delete, provide an empty `content` array.
- **Insert**: `{"op":"i", "after_lid":"index10", "content":["new"]}`. Use `_START_OF_FILE_` for `after_lid` to insert at the beginning.
- **Context for Safety**: The optional `context_before` and `context_after` fields are highly recommended to prevent errors if the file has changed unexpectedly.

**Rules**:
- Line identifiers (indexes) MUST be the strings from when the file was read.
- The `lif_hash` for any operation MUST match the hash from when the file was last read or edited.

**Example**:
`{"moves":[{"source_file_path":"a.rs","source_lif_hash":"h1","source_start_lid":"index20","source_end_lid":"index25","dest_file_path":"b.rs","dest_lif_hash":"h2","dest_after_lid":"index10"}],"edits":[{"file_path":"a.rs","lif_hash":"h1","patch":[{"op":"r","start_lid":"index2","end_lid":"index2","context_before":"// File a","content":["use b;"],"context_after":"fn main() {}"}]}]}`
"#
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "edits": {
                        "type": "array",
                        "description": "An array of patch operations to apply to existing files.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "file_path": {
                                    "type": "string",
                                    "description": "The relative path to the file to be patched."
                                },
                                "lif_hash": {
                                    "type": "string",
                                    "description": "The 8-character SHA-1 hash of the file state that this patch applies to. Must match the hash from when the file was last read."
                                },
                                "patch": {
                                    "type": "array",
                                    "description": "An array of patch operations for this specific file.",
                                    "items": {
                                        "type": "object",
                                        "required": ["op", "content"],
                                        "oneOf": [
                                            {
                                                "description": "Replace a range of lines, like a 'diff hunk'. This is best for updating a whole function body or other logical block.",
                                                "properties": {
                                                    "op": {"const": "r"},
                                                    "start_lid": {"type": "string", "description": "The starting line identifier."},
                                                    "end_lid": {"type": "string", "description": "The ending line identifier. For a single line, start_lid and end_lid are the same."},
                                                    "content": {"type": "array", "items": {"type": "string"}, "description": "The new lines of content to replace the specified range."},
                                                    "context_before": {"type": "string", "description": "The exact content of the line immediately before start_lid. Highly recommended for safety."},
                                                    "context_after": {"type": "string", "description": "The exact content of the line immediately after end_lid. Highly recommended for safety."}
                                                },
                                                "required": ["start_lid", "end_lid"]
                                            },
                                            {
                                                "description": "Insert a new block of lines after a specific line. Use `_START_OF_FILE_` as the `after_lid` to insert at the top of the file.",
                                                "properties": {
                                                    "op": {"const": "i"},
                                                    "after_lid": {"type": "string", "description": "The line identifier after which to insert. Use '_START_OF_FILE'."},
                                                    "content": {"type": "array", "items": {"type": "string"}, "description": "The new lines of content to insert."},
                                                    "context_before": {"type": "string", "description": "The exact content of the `after_lid` line. Highly recommended for safety."},
                                                    "context_after": {"type": "string", "description": "The exact content of the line immediately after `after_lid`. Highly recommended for safety."}
                                                },
                                                "required": ["after_lid"]
                                            }
                                        ]
                                    }
                                }
                            },
                            "required": ["file_path", "lif_hash", "patch"]
                        }
                    },
                    "copies": {
                        "type": "array",
                        "description": "An array of copy operations, executed after creates and before moves.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "source_file_path": { "type": "string", "description": "The path of the file to copy lines from." },
                                "source_lif_hash": { "type": "string", "description": "The lif_hash of the source file." },
                                "source_start_lid": { "type": "string", "description": "The starting line identifier of the range to copy." },
                                "source_end_lid": { "type": "string", "description": "The ending line identifier of the range to copy." },
                                "dest_file_path": { "type": "string", "description": "The path of the file to copy lines to." },
                                "dest_lif_hash": { "type": "string", "description": "The lif_hash of the destination file." },
                                "dest_after_lid": { "type": "string", "description": "The line identifier in the destination file after which to insert. Use `_START_OF_FILE_` for the beginning." }
                            },
                            "required": ["source_file_path", "source_lif_hash", "source_start_lid", "source_end_lid", "dest_file_path", "dest_lif_hash", "dest_after_lid"]
                        }
                    },
                    "moves": {
                        "type": "array",
                        "description": "An array of move operations, executed after copies and before edits.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "source_file_path": { "type": "string", "description": "The path of the file to move lines from." },
                                "source_lif_hash": { "type": "string", "description": "The lif_hash of the source file." },
                                "source_start_lid": { "type": "string", "description": "The starting line identifier of the range to move." },
                                "source_end_lid": { "type": "string", "description": "The ending line identifier of the range to move." },
                                "dest_file_path": { "type": "string", "description": "The path of the file to move lines to." },
                                "dest_lif_hash": { "type": "string", "description": "The lif_hash of the destination file." },
                                "dest_after_lid": { "type": "string", "description": "The line identifier in the destination file after which to insert. Use `_START_OF_FILE_` for the beginning." }
                            },
                            "required": ["source_file_path", "source_lif_hash", "source_start_lid", "source_end_lid", "dest_file_path", "dest_lif_hash", "dest_after_lid"]
                        }
                    }
                },
                "required": []
            }),
        },
    }
}

/// Strips the `...: ` prefix from a line, if present.
/// This makes the context check more robust, as the LLM often includes the LID
/// in the context string.
pub(crate) fn strip_prefix(line: &str) -> &str {
    if let Some(colon_pos) = line.find(':') {
        // Assume that if a colon is present, the part before it is a line identifier
        // that the LLM included, and we should ignore it for the context check.
        let after_colon = &line[colon_pos + 1..];
        return after_colon.trim_start();
    }
    line
}

/// Normalizes a string by collapsing and removing all whitespace.
pub(crate) fn normalize_whitespace(s: &str) -> String {
    s.split_whitespace().collect()
}

/// Verifies that the optional context lines in a patch operation match the actual file state.
pub(crate) fn verify_patch_context(
    operation: &PatchOperation,
    file_state: &FileState,
) -> Result<()> {
    match operation {
        PatchOperation::Insert(op) => {
            if let Some(ref provided_context_before) = op.context_before {
                if op.after_lid != "_START_OF_FILE_" {
                    let expected_content = strip_prefix(provided_context_before);
                    let after_lid_key = FileState::parse_index(&op.after_lid)?;
                    match file_state.lines.get(&after_lid_key) {
                        Some(actual_line) => {
                            if normalize_whitespace(actual_line)
                                != normalize_whitespace(expected_content)
                            {
                                return Err(anyhow!(
                                    "ContextBefore mismatch for insert after {}. AI provided '{}', but file has '{}'. (Whitespace-insensitive comparison failed)",
                                    op.after_lid,
                                    provided_context_before.trim(),
                                    actual_line.trim(),
                                ));
                            }
                        }
                        None => {
                            return Err(anyhow!(
                                "LID '{}' for contextBefore not found.",
                                op.after_lid
                            ));
                        }
                    }
                }
            }
            if let Some(ref provided_context_after) = op.context_after {
                let expected_content = strip_prefix(provided_context_after);
                let after_lid_key = if op.after_lid == "_START_OF_FILE_" {
                    None
                } else {
                    Some(FileState::parse_index(&op.after_lid)?)
                };

                let mut next_item_query = file_state.lines.range((
                    after_lid_key
                        .as_ref()
                        .map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded),
                    std::ops::Bound::Unbounded,
                ));

                match next_item_query.next() {
                    Some((lid, actual_line)) => {
                        if normalize_whitespace(actual_line)
                            != normalize_whitespace(expected_content)
                        {
                            return Err(anyhow!(
                                "ContextAfter mismatch for insert after {}. AI provided '{}', but file has '{}' at index {}. (Whitespace-insensitive comparison failed)",
                                op.after_lid,
                                provided_context_after.trim(),
                                actual_line.trim(),
                                lid.to_string()
                            ));
                        }
                    }
                    None => {
                        if !expected_content.is_empty() {
                            // If we expect empty context at EOF, it's fine.
                            return Err(anyhow!(
                                "ContextAfter mismatch: AI provided '{}' but found end of file.",
                                provided_context_after
                            ));
                        }
                    }
                }
            }
        }
        PatchOperation::Replace(op) => {
            let start_lid_key = FileState::parse_index(&op.start_lid)?;
            if let Some(ref provided_context_before) = op.context_before {
                let expected_content = strip_prefix(provided_context_before);
                match file_state.lines.range(..start_lid_key).next_back() {
                    Some((lid, actual_line)) => {
                        if normalize_whitespace(actual_line)
                            != normalize_whitespace(expected_content)
                        {
                            return Err(anyhow!(
                                "ContextBefore mismatch at {}. AI provided '{}', but file has '{}' at index {}. (Whitespace-insensitive comparison failed)",
                                op.start_lid,
                                provided_context_before.trim(),
                                actual_line.trim(),
                                lid.to_string()
                            ));
                        }
                    }
                    None => {
                        if !expected_content.is_empty() {
                            return Err(anyhow!(
                                "ContextBefore mismatch: AI provided '{}' but found start of file.",
                                provided_context_before
                            ));
                        }
                    }
                }
            }
            let end_lid_key = FileState::parse_index(&op.end_lid)?;
            if let Some(ref provided_context_after) = op.context_after {
                let expected_content = strip_prefix(provided_context_after);
                match file_state.lines.range(end_lid_key..).nth(1) {
                    Some((lid, actual_line)) => {
                        if normalize_whitespace(actual_line)
                            != normalize_whitespace(expected_content)
                        {
                            return Err(anyhow!(
                                "ContextAfter mismatch at {}. AI provided '{}', but file has '{}' at index {}. (Whitespace-insensitive comparison failed)",
                                op.end_lid,
                                provided_context_after.trim(),
                                actual_line.trim(),
                                lid.to_string()
                            ));
                        }
                    }
                    None => {
                        if !expected_content.is_empty() {
                            return Err(anyhow!(
                                "ContextAfter mismatch: AI provided '{}' but found end of file.",
                                provided_context_after
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn execute_file_operations(
    args: &FileOperationArgs,
    file_state_manager: &mut FileStateManager,
    accessible_paths: &[String],
) -> Result<String> {
    let mut results = Vec::new();

    if args.edits.is_empty() && args.copies.is_empty() && args.moves.is_empty() {
        return Ok("No file operations provided in the tool call.".to_string());
    }

    // A map from a canonical file path to its planned initial hash and operations.
    let mut planned_patches: HashMap<PathBuf, (String, Vec<PatchOperation>)> = HashMap::new();

    // Helper to add an operation to the plan. It canonicalizes the path and ensures
    // that all operations for a given file are based on the same initial lif_hash.
    fn add_op_to_plan(
        plan: &mut HashMap<PathBuf, (String, Vec<PatchOperation>)>,
        file_path_str: &str,
        lif_hash: &str,
        op: PatchOperation,
    ) -> Result<()> {
        let path = PathBuf::from(file_path_str);
        let canonical_path = path.canonicalize().map_err(|e| {
            anyhow!(
                "Failed to canonicalize path '{}': {}. File might not exist.",
                file_path_str,
                e
            )
        })?;

        let (expected_hash, ops) = plan
            .entry(canonical_path)
            .or_insert_with(|| (lif_hash.to_string(), Vec::new()));

        if expected_hash != lif_hash {
            return Err(anyhow!(
                "Inconsistent lif_hash provided for file '{}'. Expected '{}' but got '{}'. All operations for a single file must use the same initial hash.",
                file_path_str,
                expected_hash,
                lif_hash
            ));
        }
        ops.push(op);
        Ok(())
    }

    // --- Phase 1: Plan all operations ---
    // The logic inside this block tries to handle errors gracefully by adding them
    // to the `results` vector, allowing other, valid operations to proceed.
    let planning_result: Result<()> = {
        // --- Plan Copies ---
        for (i, copy) in args.copies.iter().enumerate() {
            let mut plan_copy = || {
                permissions::is_path_accessible(
                    Path::new(&copy.source_file_path),
                    accessible_paths,
                )?;
                permissions::is_path_accessible(Path::new(&copy.dest_file_path), accessible_paths)?;

                let source_state = file_state_manager.open_file(&copy.source_file_path)?;
                if source_state.get_short_hash() != copy.source_lif_hash {
                    return Err(anyhow!(
                        "Source hash mismatch for copy. Expected '{}', found '{}'.",
                        copy.source_lif_hash,
                        source_state.get_short_hash()
                    ));
                }

                let content_to_copy = source_state
                    .get_lines_in_range(&copy.source_start_lid, &copy.source_end_lid)?;

                let insert_op = PatchOperation::Insert(crate::patch::InsertOperation {
                    after_lid: copy.dest_after_lid.clone(),
                    content: content_to_copy,
                    context_before: None,
                    context_after: None,
                });

                add_op_to_plan(
                    &mut planned_patches,
                    &copy.dest_file_path,
                    &copy.dest_lif_hash,
                    insert_op,
                )
            };
            if let Err(e) = plan_copy() {
                results.push(format!("Error planning copy operation #{i}: {e}"));
            }
        }

        // --- Plan Moves ---
        for (i, mov) in args.moves.iter().enumerate() {
            let mut plan_move = || {
                permissions::is_path_accessible(
                    Path::new(&mov.source_file_path),
                    accessible_paths,
                )?;
                permissions::is_path_accessible(Path::new(&mov.dest_file_path), accessible_paths)?;

                let source_state = file_state_manager.open_file(&mov.source_file_path)?;
                if source_state.get_short_hash() != mov.source_lif_hash {
                    return Err(anyhow!(
                        "Source hash mismatch for move. Expected '{}', found '{}'.",
                        mov.source_lif_hash,
                        source_state.get_short_hash()
                    ));
                }

                let content_to_move =
                    source_state.get_lines_in_range(&mov.source_start_lid, &mov.source_end_lid)?;

                let delete_op = PatchOperation::Replace(ReplaceOperation {
                    start_lid: mov.source_start_lid.clone(),
                    end_lid: mov.source_end_lid.clone(),
                    content: vec![],
                    context_before: None,
                    context_after: None,
                });
                add_op_to_plan(
                    &mut planned_patches,
                    &mov.source_file_path,
                    &mov.source_lif_hash,
                    delete_op,
                )?;

                let insert_op = PatchOperation::Insert(crate::patch::InsertOperation {
                    after_lid: mov.dest_after_lid.clone(),
                    content: content_to_move,
                    context_before: None,
                    context_after: None,
                });
                add_op_to_plan(
                    &mut planned_patches,
                    &mov.dest_file_path,
                    &mov.dest_lif_hash,
                    insert_op,
                )
            };

            if let Err(e) = plan_move() {
                results.push(format!("Error planning move operation #{i}: {e}"));
            }
        }

        // --- Plan Edits ---
        for edit in &args.edits {
            let file_path_str = &edit.file_path;
            let mut plan_edit = || -> Result<()> {
                permissions::is_path_accessible(Path::new(file_path_str), accessible_paths)?;

                // For edits, we can do a preliminary hash check during planning
                // because all ops in a single `PatchArgs` use the same hash.
                let file_state = file_state_manager.open_file(file_path_str)?;
                if file_state.get_short_hash() != edit.lif_hash {
                    return Err(anyhow!(
                        "Hash mismatch. Expected '{}', found '{}'.",
                        edit.lif_hash,
                        file_state.get_short_hash()
                    ));
                }

                for op in &edit.patch {
                    add_op_to_plan(
                        &mut planned_patches,
                        file_path_str,
                        &edit.lif_hash,
                        op.clone(),
                    )?;
                }
                Ok(())
            };
            if let Err(e) = plan_edit() {
                results.push(format!("File: {file_path_str}\nError: {e}"));
            }
        }
        Ok(())
    };

    if let Err(e) = planning_result {
        results.push(format!("A fatal error occurred during planning: {e}"));
        return Ok(results.join("\n\n---\n\n"));
    }

    // --- Phase 2: Execute the consolidated plan ---
    for (path, (initial_hash, operations)) in planned_patches {
        let file_path_str = path.to_string_lossy();
        let result = (|| {
            let file_state = file_state_manager.open_file(&file_path_str)?;

            // Final, single hash check for the entire file operation batch
            if file_state.get_short_hash() != initial_hash {
                return Err(anyhow!(
                    "Hash mismatch for '{}'. Expected '{}', found '{}'. The file was modified externally.",
                    file_path_str,
                    initial_hash,
                    file_state.get_short_hash()
                ));
            }

            // Verify context for all planned operations before applying any
            for op in &operations {
                verify_patch_context(op, file_state)?;
            }

            // Apply all patches for this file at once
            let diff = file_state.apply_and_write_patch(&operations)?;
            let new_short_hash = file_state.get_short_hash();

            Ok(format!(
                "Patch from hash {initial_hash} applied successfully. New lif_hash: {new_short_hash}. Changes:\n{diff}"
            ))
        })();

        match result {
            Ok(success_msg) => results.push(format!("File: {file_path_str}\n{success_msg}")),
            Err(e) => results.push(format!("File: {file_path_str}\nError: {e}")),
        }
    }

    Ok(results.join("\n\n---\n\n"))
}
