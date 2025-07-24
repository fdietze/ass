//! # File Editor Tool
//!
//! This module provides the `edit_file` tool. Its primary responsibilities are:
//!
//! 1.  **Schema Definition**: Defines the JSON schema for the `edit_file` tool. This schema
//!     is sent to the LLM, instructing it on how to format its edit requests. It uses
//!     dedicated "buckets" (`inserts`, `replaces`, etc.) for clarity.
//!
//! 2.  **Request Handling & Validation**: Implements `execute_file_operations`, the function
//!     that orchestrates the entire editing process. It receives a batch of requested
//!     operations from the LLM, grouped by type.
//!
//! 3.  **Anchor Validation**: Before translating requests into internal commands, it performs
//!     the crucial **LineAnchor** validation. This is the core safety mechanism.
//!
//! 4.  **Translation**: Validated requests are translated into simple, internal `PatchOperation`
//!     primitives, which are then passed to the `FileState` module for execution.

use crate::file_state::FileState;
use crate::file_state_manager::FileStateManager;
use crate::patch::{InsertOp, PatchOperation, ReplaceOp};
use crate::permissions;
use anyhow::{Result, anyhow};
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// --- Tool-Facing Request Structs ---
// These structs define the public API of the `edit_file` tool.

#[derive(Deserialize, Debug)]
pub struct TopLevelRequest {
    #[serde(default)]
    pub inserts: Vec<InsertRequest>,
    #[serde(default)]
    pub replaces: Vec<ReplaceRequest>,
    #[serde(default)]
    pub moves: Vec<MoveCopyRequest>,
    #[serde(default)]
    pub copies: Vec<MoveCopyRequest>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    StartOfFile,
    EndOfFile,
    AfterAnchor,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct InsertRequest {
    pub file_path: String,
    pub new_content: Vec<String>,
    pub at_position: Position,
    pub anchor_lid: Option<String>,
    pub anchor_content: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct ReplaceRequest {
    pub file_path: String,
    pub start_lid: String,
    pub start_content: String,
    pub end_lid: String,
    pub end_content: String,
    pub new_content: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct MoveCopyRequest {
    pub op: String, // "move" or "copy"
    pub source_file_path: String,
    pub source_start_lid: String,
    pub source_start_content: String,
    pub source_end_lid: String,
    pub source_end_content: String,
    pub dest_file_path: String,
    pub dest_at_position: Position,
    pub dest_anchor_lid: Option<String>,
    pub dest_anchor_content: Option<String>,
}

pub fn edit_file_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "edit_file".to_string(),
            description: Some(
                r#"Atomically performs a series of file editing operations using a robust anchor-based system.
All operations are planned based on the files' initial state. Line Anchors (LID + content) MUST be valid at the beginning of the tool call.

**Execution Order**: Operations are always executed in a fixed order: 1. Copies, 2. Moves, 3. Replaces, 4. Inserts.

**Line Anchors**: An anchor is a combination of a line's unique identifier (`lid`) and its exact `content`. Both must be provided and must match the file exactly for an operation to succeed.

**Operations**:
- `inserts`: Adds new lines. Position can be `start_of_file`, `end_of_file`, or `after_anchor`.
- `replaces`: Replaces a range of lines. To delete, provide an empty `new_content` array.
- `moves` / `copies`: Transfers blocks of lines. Prefer `move` over `copy` + `delete`."#
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "copies": {
                        "type": "array",
                        "description": "A list of copy operations to perform.",
                        "items": {
                            "type": "object",
                            "title": "Copy Operation",
                            "properties": {
                                "op": { "const": "copy" },
                                "source_file_path": { "type": "string", "description": "Path of the file to copy lines from." },
                                "source_start_lid": { "type": "string", "description": "LID of the first line in the source range." },
                                "source_start_content": { "type": "string", "description": "Exact content of the source start line." },
                                "source_end_lid": { "type": "string", "description": "LID of the last line in the source range." },
                                "source_end_content": { "type": "string", "description": "Exact content of the source end line." },
                                "dest_file_path": { "type": "string", "description": "Path of the file to copy lines to." },
                                "dest_at_position": { "enum": ["start_of_file", "end_of_file", "after_anchor"], "description": "Specifies where to insert the content in the destination file." },
                                "dest_anchor_lid": { "type": "string", "description": "The LID of the destination anchor line. Required only when 'dest_at_position' is 'after_anchor'." },
                                "dest_anchor_content": { "type": "string", "description": "The exact content of the destination anchor line. Required only when 'dest_at_position' is 'after_anchor'." }
                            },
                            "required": ["op", "source_file_path", "source_start_lid", "source_start_content", "source_end_lid", "source_end_content", "dest_file_path", "dest_at_position"]
                        }
                    },
                    "moves": {
                                    "type": "array",
                        "description": "A list of move operations to perform.",
                                    "items": {
                                        "type": "object",
                            "title": "Move Operation",
                                                "properties": {
                                "op": { "const": "move" },
                                "source_file_path": { "type": "string", "description": "Path of the file to move lines from." },
                                "source_start_lid": { "type": "string", "description": "LID of the first line in the source range." },
                                "source_start_content": { "type": "string", "description": "Exact content of the source start line." },
                                "source_end_lid": { "type": "string", "description": "LID of the last line in the source range." },
                                "source_end_content": { "type": "string", "description": "Exact content of the source end line." },
                                "dest_file_path": { "type": "string", "description": "Path of the file to move lines to." },
                                "dest_at_position": { "enum": ["start_of_file", "end_of_file", "after_anchor"], "description": "Specifies where to insert the content in the destination file." },
                                "dest_anchor_lid": { "type": "string", "description": "The LID of the destination anchor line. Required only when 'dest_at_position' is 'after_anchor'." },
                                "dest_anchor_content": { "type": "string", "description": "The exact content of the destination anchor line. Required only when 'dest_at_position' is 'after_anchor'." }
                            },
                            "required": ["op", "source_file_path", "source_start_lid", "source_start_content", "source_end_lid", "source_end_content", "dest_file_path", "dest_at_position"]
                        }
                    },
                    "replaces": {
                        "type": "array",
                        "description": "A list of replace operations to perform.",
                        "items": {
                            "type": "object",
                            "title": "Replace Operation",
                            "properties": {
                                "file_path": { "type": "string", "description": "The relative path to the file to be modified." },
                                "start_lid": { "type": "string", "description": "The LID of the first line in the range to replace." },
                                "start_content": { "type": "string", "description": "The exact content of the starting line." },
                                "end_lid": { "type": "string", "description": "The LID of the last line in the range to replace. For a single line, this is the same as 'start_lid'." },
                                "end_content": { "type": "string", "description": "The exact content of the ending line." },
                                "new_content": { "type": "array", "items": { "type": "string" }, "description": "The new lines to replace the old range with. Use an empty array to delete." }
                            },
                            "required": ["file_path", "start_lid", "start_content", "end_lid", "end_content", "new_content"]
                        }
                    },
                    "inserts": {
                        "type": "array",
                        "description": "A list of insert operations to perform.",
                        "items": {
                            "type": "object",
                            "title": "Insert Operation",
                            "properties": {
                                "file_path": { "type": "string", "description": "The relative path to the file to be modified." },
                                "new_content": { "type": "array", "items": { "type": "string" }, "description": "The new lines of content to insert." },
                                "at_position": { "enum": ["start_of_file", "end_of_file", "after_anchor"], "description": "Specifies where to insert the content." },
                                "anchor_lid": { "type": "string", "description": "The LID of the anchor line. Required only when 'at_position' is 'after_anchor'." },
                                "anchor_content": { "type": "string", "description": "The exact content of the anchor line. Required only when 'at_position' is 'after_anchor'." }
                            },
                            "required": ["file_path", "new_content", "at_position"]
                        }
                    }
                },
                "required": []
            }),
        },
    }
}

/// Validates a line anchor against the current file state.
/// Checks that a line with the given LID exists and its content matches byte-for-byte.
fn validate_anchor(
    file_state: &FileState,
    lid_str: &str,
    expected_content: &str,
    op_name: &str,
    anchor_name: &str,
) -> Result<()> {
    let lid = FileState::parse_index(lid_str)?;
    match file_state.lines.get(&lid) {
        Some(actual_content) => {
            if actual_content != expected_content {
                return Err(anyhow!(
                    "Validation failed for '{op_name}': {anchor_name} content mismatch for LID '{lid_str}'. Expected '{expected_content}', found '{actual_content}'."
                ));
            }
        }
        None => {
            return Err(anyhow!(
                "Validation failed for '{op_name}': {anchor_name} LID '{lid_str}' not found in file '{}'.",
                file_state.path.display()
            ));
        }
    }
    Ok(())
}

/// The main execution function for the `edit_file` tool.
pub fn execute_file_operations(
    args: &TopLevelRequest,
    file_state_manager: &mut FileStateManager,
    accessible_paths: &[String],
) -> Result<String> {
    let mut results = Vec::new();

    if args.inserts.is_empty()
        && args.replaces.is_empty()
        && args.moves.is_empty()
        && args.copies.is_empty()
    {
        return Ok("No file operations provided in the tool call.".to_string());
    }

    // A map from a canonical file path to its planned operations.
    let mut planned_ops: HashMap<PathBuf, Vec<PatchOperation>> = HashMap::new();

    // --- Phase 1: Plan and Validate all operations ---
    // The order here is fixed and documented for the LLM: copies, moves, replaces, inserts.
    let planning_result: Result<()> = (|| {
        // Plan Copies and Moves
        for req in args.copies.iter().chain(args.moves.iter()) {
            let op_name = &req.op;
            permissions::is_path_accessible(Path::new(&req.source_file_path), accessible_paths)?;
            permissions::is_path_accessible(Path::new(&req.dest_file_path), accessible_paths)?;

            let (source_path, content_to_transfer) = {
                let source_state = file_state_manager.open_file(&req.source_file_path)?;
                validate_anchor(
                    source_state,
                    &req.source_start_lid,
                    &req.source_start_content,
                    op_name,
                    "source_start_anchor",
                )?;
                validate_anchor(
                    source_state,
                    &req.source_end_lid,
                    &req.source_end_content,
                    op_name,
                    "source_end_anchor",
                )?;
                let content =
                    source_state.get_lines_in_range(&req.source_start_lid, &req.source_end_lid)?;
                (source_state.path.clone(), content)
            };

            let dest_state = file_state_manager.open_file(&req.dest_file_path)?;
            let after_lid = match req.dest_at_position {
                Position::StartOfFile => None,
                Position::EndOfFile => dest_state.lines.last_key_value().map(|(k, _)| k.clone()),
                Position::AfterAnchor => {
                    let lid_str = req.dest_anchor_lid.as_deref().ok_or_else(|| {
                        anyhow!("`dest_anchor_lid` is required for `after_anchor` position.")
                    })?;
                    let content = req.dest_anchor_content.as_deref().ok_or_else(|| {
                        anyhow!("`dest_anchor_content` is required for `after_anchor` position.")
                    })?;
                    validate_anchor(dest_state, lid_str, content, op_name, "dest_anchor")?;
                    Some(FileState::parse_index(lid_str)?)
                }
            };

            if req.op == "move" {
                let delete_op = PatchOperation::Replace(ReplaceOp {
                    start_lid: FileState::parse_index(&req.source_start_lid)?,
                    end_lid: FileState::parse_index(&req.source_end_lid)?,
                    content: vec![],
                });
                planned_ops.entry(source_path).or_default().push(delete_op);
            }

            let insert_op = PatchOperation::Insert(InsertOp {
                after_lid,
                content: content_to_transfer,
            });
            planned_ops
                .entry(dest_state.path.clone())
                .or_default()
                .push(insert_op);
        }

        // Plan Replaces
        for req in &args.replaces {
            permissions::is_path_accessible(Path::new(&req.file_path), accessible_paths)?;
            let file_state = file_state_manager.open_file(&req.file_path)?;
            validate_anchor(
                file_state,
                &req.start_lid,
                &req.start_content,
                "replace",
                "start_anchor",
            )?;
            validate_anchor(
                file_state,
                &req.end_lid,
                &req.end_content,
                "replace",
                "end_anchor",
            )?;

            let internal_op = PatchOperation::Replace(ReplaceOp {
                start_lid: FileState::parse_index(&req.start_lid)?,
                end_lid: FileState::parse_index(&req.end_lid)?,
                content: req.new_content.clone(),
            });
            planned_ops
                .entry(file_state.path.clone())
                .or_default()
                .push(internal_op);
        }

        // Plan Inserts
        for req in &args.inserts {
            permissions::is_path_accessible(Path::new(&req.file_path), accessible_paths)?;
            let file_state = file_state_manager.open_file(&req.file_path)?;

            let after_lid = match req.at_position {
                Position::StartOfFile => None,
                Position::EndOfFile => file_state.lines.last_key_value().map(|(k, _)| k.clone()),
                Position::AfterAnchor => {
                    let lid_str = req.anchor_lid.as_deref().ok_or_else(|| {
                        anyhow!("`anchor_lid` is required for `after_anchor` position.")
                    })?;
                    let content = req.anchor_content.as_deref().ok_or_else(|| {
                        anyhow!("`anchor_content` is required for `after_anchor` position.")
                    })?;
                    validate_anchor(file_state, lid_str, content, "insert", "anchor")?;
                    Some(FileState::parse_index(lid_str)?)
                }
            };

            let internal_op = PatchOperation::Insert(InsertOp {
                after_lid,
                content: req.new_content.clone(),
            });
            planned_ops
                .entry(file_state.path.clone())
                .or_default()
                .push(internal_op);
        }

        Ok(())
    })();

    planning_result?;

    // --- Phase 2: Execute the consolidated plan ---
    for (path, operations) in planned_ops {
        let file_path_str = path.to_string_lossy();
        let result: Result<String> = (|| {
            let file_state = file_state_manager.get_file_state_mut(&file_path_str)?;
            let initial_hash = file_state.get_short_hash().to_string();

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
