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

use crate::config::Config;
use crate::file_state::FileState;
use crate::file_state_manager::FileStateManager;
use crate::patch::{InsertOp, PatchOperation, ReplaceOp};
use crate::permissions;
use crate::tools::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use openrouter_api::models::tool::FunctionDescription;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

// --- Tool-Facing Request Structs ---
// These structs define the public API of the `edit_file` tool.

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Anchor {
    pub lid: String,
    pub line_content: String,
}

#[derive(Deserialize, Debug, Serialize)]
pub struct TopLevelRequest {
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub inserts: Vec<InsertRequest>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub replaces: Vec<ReplaceRequest>,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub moves: Vec<MoveRequest>,
}

#[derive(Deserialize, Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Position {
    StartOfFile,
    EndOfFile,
    AfterAnchor,
    BeforeAnchor, // New position
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct InsertRequest {
    pub file_path: String,
    pub new_content: Vec<String>,
    pub at_position: Position,
    pub context_anchor: Option<Anchor>,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ReplaceRequest {
    pub file_path: String,
    pub anchor_range_begin: Option<Anchor>, // Renamed for inclusivity
    pub anchor_range_end: Option<Anchor>,   // Renamed for inclusivity
    pub new_content: Vec<String>,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MoveRequest {
    pub source_file_path: String,
    pub source_range_start_anchor: Option<Anchor>, // Renamed for inclusivity
    pub source_range_end_anchor: Option<Anchor>,   // Renamed for inclusivity
    pub dest_file_path: String,
    pub dest_at_position: Position,
    pub dest_context_anchor: Option<Anchor>,
}

/// Represents the successfully planned operations to be executed.
pub struct EditPlan {
    pub planned_ops: HashMap<PathBuf, Vec<PatchOperation>>,
}

pub struct FileEditorTool;

#[async_trait]
impl Tool for FileEditorTool {
    fn name(&self) -> &'static str {
        "edit_file"
    }

    fn schema(&self) -> FunctionDescription {
        FunctionDescription {
            name: "edit_file".to_string(),
            description: Some(
                r#"Atomically performs file edits using a robust, inclusive anchor system for move and replace operations.

**CRITICAL: For both `moves` and `replaces`, the specified anchors are INCLUSIVE. The anchor lines themselves and everything between them will be part of the operation.**

- To replace the entire file, set both `anchor_range_begin` and `anchor_range_end` to `null`.

**Execution Order**: 1. Moves, 2. Replaces, 3. Inserts.

**Operations**:
- `inserts`: Adds new lines, positioned relative to a context anchor.
- `replaces`: Replaces a range of lines from `anchor_range_begin` to `anchor_range_end`, including the anchor lines themselves.
- `moves`: Moves a range of lines from `source_range_start_anchor` to `source_range_end_anchor`, including the anchor lines themselves.
"#
                    .to_string(),
            ),
            strict: Some(true),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "moves": {
                        "type": "array",
                        "description": "A list of move operations to perform. The source range is INCLUSIVE.",
                        "items": {
                            "type": "object",
                            "title": "Move Operation",
                            "properties": {
                                "source_file_path": { "type": "string", "description": "Path of the file to move lines from." },
                                "source_range_start_anchor": {
                                    "type": ["object", "null"],
                                    "title": "Source Range Start Anchor (Inclusive)",
                                    "description": "An anchor for the FIRST line in the source range. This line WILL be part of the moved content.",
                                    "properties": {
                                        "lid": { "type": "string", "description": "The unique identifier (LID) of the anchor line." },
                                        "line_content": { "type": "string", "description": "The exact, single-line content of the anchor line." }
                                    },
                                    "additionalProperties": false,
                                    "required": ["lid", "line_content"]
                                },
                                "source_range_end_anchor": {
                                    "type": ["object", "null"],
                                    "title": "Source Range End Anchor (Inclusive)",
                                    "description": "An anchor for the LAST line in the source range. This line WILL be part of the moved content.",
                                    "properties": {
                                        "lid": { "type": "string", "description": "The unique identifier (LID) of the anchor line." },
                                        "line_content": { "type": "string", "description": "The exact, single-line content of the anchor line." }
                                    },
                                    "additionalProperties": false,
                                    "required": ["lid", "line_content"]
                                },
                                "dest_file_path": { "type": "string", "description": "Path of the file to move lines to." },
                                "dest_at_position": { "enum": ["start_of_file", "end_of_file", "after_anchor", "before_anchor"], "description": "Specifies where to insert the content in the destination file." },
                                "dest_context_anchor": {
                                    "type": ["object", "null"],
                                    "title": "Destination Context Anchor",
                                    "description": "An anchor to uniquely identify the destination line. Required only when 'dest_at_position' is 'after_anchor' or 'before_anchor'.",
                                    "properties": {
                                        "lid": { "type": "string", "description": "The unique identifier (LID) of the anchor line. Must be prefixed with 'lid-'. Example: 'lid-a1b2'." },
                                        "line_content": { "type": "string", "description": "The exact, single-line content of the anchor line.", "pattern": "^[^\r\n]*$" }
                                    },
                                    "additionalProperties": false,
                                    "required": ["lid", "line_content"]
                                }
                            },
                            "additionalProperties": false,
                            "required": ["source_file_path", "source_range_start_anchor", "source_range_end_anchor", "dest_file_path", "dest_at_position", "dest_context_anchor"]
                        }
                    },
                    "replaces": {
                        "type": "array",
                        "description": "A list of replace operations to perform. The range is INCLUSIVE.",
                        "items": {
                            "type": "object",
                            "title": "Replace Operation",
                            "properties": {
                                "file_path": { "type": "string", "description": "The relative path to the file to be modified." },
                                "anchor_range_begin": {
                                    "type": ["object", "null"],
                                    "title": "Range Begin Anchor (Inclusive)",
                                    "description": "An anchor for the FIRST line in the range to replace. This line WILL be replaced. If null, the range starts at the beginning of the file.",
                                    "properties": {
                                        "lid": { "type": "string", "description": "The unique identifier (LID) of the anchor line." },
                                        "line_content": { "type": "string", "description": "The exact, single-line content of the anchor line." }
                                    },
                                    "additionalProperties": false,
                                    "required": ["lid", "line_content"]
                                },
                                "new_content": { "type": "array", "items": { "type": "string" }, "description": "The new lines to replace the old range with. Use an empty array to delete." },
                                "anchor_range_end": {
                                    "type": ["object", "null"],
                                    "title": "Range End Anchor (Inclusive)",
                                    "description": "An anchor for the LAST line in the range to replace. This line WILL be replaced. If null, the range extends to the end of the file.",
                                    "properties": {
                                        "lid": { "type": "string", "description": "The unique identifier (LID) of the anchor line." },
                                        "line_content": { "type": "string", "description": "The exact, single-line content of the anchor line." }
                                    },
                                    "additionalProperties": false,
                                    "required": ["lid", "line_content"]
                                }
                            },
                            "additionalProperties": false,
                            "required": ["file_path", "anchor_range_begin", "new_content", "anchor_range_end"]
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
                                "at_position": { "enum": ["start_of_file", "end_of_file", "after_anchor", "before_anchor"], "description": "Specifies where to insert the content." },
                                "context_anchor": {
                                    "type": ["object", "null"],
                                    "title": "Context Anchor",
                                    "description": "An anchor to uniquely identify the line to insert before or after. Required only when 'at_position' is 'after_anchor' or 'before_anchor'.",
                                    "properties": {
                                        "lid": { "type": "string", "description": "The unique identifier (LID) of the anchor line. Must be prefixed with 'lid-'. Example: 'lid-a1b2'." },
                                        "line_content": { "type": "string", "description": "The exact, single-line content of the anchor line.", "pattern": "^[^\r\n]*$" }
                                    },
                                    "additionalProperties": false,
                                    "required": ["lid", "line_content"]
                                },
                                "new_content": { "type": "array", "items": { "type": "string" }, "description": "The new lines of content to insert." }
                            },
                            "additionalProperties": false,
                            "required": ["file_path", "at_position", "context_anchor", "new_content"]
                        }
                    }
                },
                "additionalProperties": false,
                "required": ["moves", "replaces", "inserts"]
            }),
        }
    }

    /// This method acts as a full dry run, validating arguments and showing the
    /// intended changes without actually modifying any state on disk.
    fn preview(
        &self,
        args: &Value,
        config: &Config,
        fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let args: TopLevelRequest = serde_json::from_value(args.clone())?;
        let mut manager = fsm.lock().unwrap();
        create_diff_preview(&args, &mut manager, &config.accessible_paths)
    }

    /// Executes the tool's primary function.
    ///
    /// On success, this method returns a concise, machine-readable summary of
    /// the changes, including new file hashes. This output is for the LLM.
    ///
    /// Any output intended for the user during execution (e.g., live command output)
    /// should be printed directly to stdout within this method.
    async fn execute(
        &self,
        args: &Value,
        config: &Config,
        fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let args: TopLevelRequest = serde_json::from_value(args.clone())?;
        let mut manager = fsm.lock().unwrap();
        execute_file_operations(&args, &mut manager, &config.accessible_paths)
    }

    fn is_safe_for_auto_execute(&self, args: &Value, config: &Config) -> Result<bool> {
        let args: TopLevelRequest = serde_json::from_value(args.clone())?;

        for req in &args.inserts {
            if permissions::is_path_accessible(Path::new(&req.file_path), &config.accessible_paths)
                .is_err()
            {
                return Ok(false);
            }
        }

        for req in &args.replaces {
            if permissions::is_path_accessible(Path::new(&req.file_path), &config.accessible_paths)
                .is_err()
            {
                return Ok(false);
            }
        }

        for req in &args.moves {
            if permissions::is_path_accessible(
                Path::new(&req.source_file_path),
                &config.accessible_paths,
            )
            .is_err()
            {
                return Ok(false);
            }
            if permissions::is_path_accessible(
                Path::new(&req.dest_file_path),
                &config.accessible_paths,
            )
            .is_err()
            {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

static WHITESPACE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

/// Collapses all whitespace sequences to a single space and trims the string.
fn collapse_whitespace(s: &str) -> String {
    WHITESPACE_REGEX.replace_all(s.trim(), " ").to_string()
}

/// Validates a line anchor against the current file state.
/// Checks that a line with the given LID exists, its content matches,
/// and the random suffix in the LID matches the stored suffix.
fn validate_anchor(
    file_state: &FileState,
    lid_str: &str,
    expected_content: &str,
    op_name: &str,
    anchor_name: &str,
) -> Result<()> {
    let (lid, expected_suffix) = FileState::parse_lid(lid_str)?;
    match file_state.lines.get(&lid) {
        Some((actual_content, actual_suffix)) => {
            if &expected_suffix != actual_suffix {
                return Err(anyhow!(
                    "Validation failed for '{op_name}': {anchor_name} suffix mismatch for LID '{lid_str}'. The line content is correct, but the file has been modified. Please re-read the file to get the latest LIDs."
                ));
            }
            let collapsed_actual = collapse_whitespace(actual_content);
            let collapsed_expected = collapse_whitespace(expected_content);

            if collapsed_actual != collapsed_expected {
                return Err(anyhow!(
                    "Validation failed for '{op_name}': {anchor_name} content mismatch for LID '{lid_str}'.\n\
                    The line content provided in your request does not match the current content of the file.\n\n\
                    Expected content (from your request):\n  > {expected_content}\n\n\
                    Actual content (in the file):\n  > {actual_content}"
                ));
            }
        }
        None => {
            // The LID wasn't found. Let's search for the line content to provide a better error message.
            let collapsed_expected = collapse_whitespace(expected_content);

            let found_line_info = file_state
                .lines
                .iter()
                .enumerate()
                .find(|(_, (_, (content, _)))| collapse_whitespace(content) == collapsed_expected);

            if let Some((position, (index, (_actual_content, suffix)))) = found_line_info {
                let line_number = position + 1;
                let total_lines = file_state.lines.len();

                // 5 lines before, the line itself, 4 lines after = 10 lines total
                let start_line_num = line_number.saturating_sub(5).max(1);
                let end_line_num = (line_number + 4).min(total_lines);

                let all_lines_vec: Vec<_> = file_state.lines.iter().collect();

                // slice indices are 0-based.
                let context_slice = &all_lines_vec[(start_line_num - 1)..end_line_num];

                let context_details = context_slice
                    .iter()
                    .enumerate()
                    .map(|(i, (f_index, (content, current_suffix)))| {
                        let current_line_number = start_line_num + i;
                        let lid = FileState::display_lid(f_index, current_suffix);
                        let indicator = if current_line_number == line_number {
                            ">"
                        } else {
                            " "
                        };
                        format!("{indicator} {current_line_number:<4} {lid}: {content}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                let new_lid = FileState::display_lid(index, suffix);
                return Err(anyhow!(
                    "Validation failed for '{op_name}': {anchor_name} LID '{lid_str}' not found in file '{}'.\n\
                    However, the line content was found with a new LID '{new_lid}'. The file was likely modified externally.\n\
                    Please use the new LIDs from the context below to form your request.\n\n\
                    Context around the found line:\n---\n{}\n---",
                    file_state.path.display(),
                    context_details
                ));
            }

            // If the content is also not found, fall back to the original error.
            return Err(anyhow!(
                "Validation failed for '{op_name}': {anchor_name} LID '{lid_str}' not found in file '{}'.",
                file_state.path.display()
            ));
        }
    }
    Ok(())
}

/// Validates the request and plans the necessary file operations.
pub fn plan_file_operations(
    args: &TopLevelRequest,
    file_state_manager: &mut FileStateManager,
    accessible_paths: &[String],
) -> Result<EditPlan> {
    let mut planned_ops: HashMap<PathBuf, Vec<PatchOperation>> = HashMap::new();
    let mut validation_errors: Vec<anyhow::Error> = Vec::new();

    // The order here is fixed and documented for the LLM: moves, replaces, inserts.

    // Plan Moves
    for (i, req) in args.moves.iter().enumerate() {
        let result: Result<((PathBuf, PatchOperation), (PathBuf, PatchOperation))> = (|| {
            permissions::is_path_accessible(Path::new(&req.source_file_path), accessible_paths)?;
            permissions::is_path_accessible(Path::new(&req.dest_file_path), accessible_paths)?;

            let (source_path, content_to_transfer, delete_op) = {
                let source_state = file_state_manager.open_file(&req.source_file_path)?;

                let start_anchor = req
                    .source_range_start_anchor
                    .as_ref()
                    .ok_or_else(|| anyhow!("`source_range_start_anchor` is required."))?;
                let end_anchor = req
                    .source_range_end_anchor
                    .as_ref()
                    .ok_or_else(|| anyhow!("`source_range_end_anchor` is required."))?;

                validate_anchor(
                    source_state,
                    &start_anchor.lid,
                    &start_anchor.line_content,
                    "move",
                    "source_range_start_anchor",
                )?;
                validate_anchor(
                    source_state,
                    &end_anchor.lid,
                    &end_anchor.line_content,
                    "move",
                    "source_range_end_anchor",
                )?;

                let start_lid = FileState::parse_lid(&start_anchor.lid)?.0;
                let end_lid = FileState::parse_lid(&end_anchor.lid)?.0;

                let content = source_state.get_content_in_range(&start_lid, &end_lid)?;
                let op = PatchOperation::Replace(ReplaceOp {
                    start_lid,
                    end_lid,
                    content: vec![],
                });
                (source_state.path.clone(), content, op)
            };

            let dest_state = file_state_manager.open_file(&req.dest_file_path)?;
            let after_lid = match req.dest_at_position {
                Position::StartOfFile => None,
                Position::EndOfFile => dest_state.lines.last_key_value().map(|(k, _)| k.clone()),
                Position::AfterAnchor => {
                    let anchor = req.dest_context_anchor.as_ref().ok_or_else(|| {
                        anyhow!("`dest_context_anchor` is required for `after_anchor` position.")
                    })?;
                    validate_anchor(
                        dest_state,
                        &anchor.lid,
                        &anchor.line_content,
                        "move",
                        "dest_context_anchor",
                    )?;
                    Some(FileState::parse_lid(&anchor.lid)?.0)
                }
                Position::BeforeAnchor => {
                    let anchor = req.dest_context_anchor.as_ref().ok_or_else(|| {
                        anyhow!("`dest_context_anchor` is required for `before_anchor` position.")
                    })?;
                    validate_anchor(
                        dest_state,
                        &anchor.lid,
                        &anchor.line_content,
                        "move",
                        "dest_context_anchor",
                    )?;
                    let anchor_lid = FileState::parse_lid(&anchor.lid)?.0;
                    // To insert *before* the anchor, we need the LID of the line *preceding* the anchor.
                    dest_state
                        .lines
                        .range(..anchor_lid)
                        .next_back()
                        .map(|(k, _)| k.clone())
                }
            };

            let insert_op = PatchOperation::Insert(InsertOp {
                after_lid,
                content: content_to_transfer,
            });
            Ok((
                (source_path, delete_op),
                (dest_state.path.clone(), insert_op),
            ))
        })();

        match result {
            Ok(((source_path, delete_op), (dest_path, insert_op))) => {
                planned_ops.entry(source_path).or_default().push(delete_op);
                planned_ops.entry(dest_path).or_default().push(insert_op);
            }
            Err(e) => {
                validation_errors.push(anyhow!(
                    "Move request #{i} (source: '{}', dest: '{}'): {e}",
                    req.source_file_path,
                    req.dest_file_path
                ));
            }
        }
    }

    // Plan Replaces
    for (i, req) in args.replaces.iter().enumerate() {
        let result: Result<(PathBuf, PatchOperation)> = (|| {
            permissions::is_path_accessible(Path::new(&req.file_path), accessible_paths)?;

            let file_state = file_state_manager.open_file(&req.file_path)?;

            let start_lid = if let Some(anchor) = &req.anchor_range_begin {
                validate_anchor(
                    file_state,
                    &anchor.lid,
                    &anchor.line_content,
                    "replace",
                    "anchor_range_begin",
                )?;
                Some(FileState::parse_lid(&anchor.lid)?.0)
            } else {
                // No start anchor means start of file
                file_state.lines.first_key_value().map(|(k, _)| k.clone())
            };

            let end_lid = if let Some(anchor) = &req.anchor_range_end {
                validate_anchor(
                    file_state,
                    &anchor.lid,
                    &anchor.line_content,
                    "replace",
                    "anchor_range_end",
                )?;
                Some(FileState::parse_lid(&anchor.lid)?.0)
            } else {
                // No end anchor means end of file
                file_state.lines.last_key_value().map(|(k, _)| k.clone())
            };

            let new_content_with_suffixes: Vec<(String, String)> = req
                .new_content
                .iter()
                .map(|line| (line.clone(), crate::file_state::generate_random_suffix()))
                .collect();

            let internal_op = match (start_lid, end_lid) {
                (Some(start), Some(end)) => {
                    // Standard inclusive range replace
                    PatchOperation::Replace(ReplaceOp {
                        start_lid: start,
                        end_lid: end,
                        content: new_content_with_suffixes,
                    })
                }
                _ => {
                    // This handles cases where the file is empty and both LIDs are None.
                    // We treat it as an insert into an empty file.
                    PatchOperation::Insert(InsertOp {
                        after_lid: None,
                        content: new_content_with_suffixes,
                    })
                }
            };

            Ok((file_state.path.clone(), internal_op))
        })();

        match result {
            Ok((path, op)) => planned_ops.entry(path).or_default().push(op),
            Err(e) => {
                validation_errors.push(anyhow!(
                    "Replace request #{i} (file: '{}'): {e}",
                    req.file_path
                ));
            }
        }
    }

    // Plan Inserts
    for (i, req) in args.inserts.iter().enumerate() {
        let result: Result<(PathBuf, PatchOperation)> = (|| {
            permissions::is_path_accessible(Path::new(&req.file_path), accessible_paths)?;
            let file_state = file_state_manager.open_file(&req.file_path)?;

            let after_lid = match req.at_position {
                Position::StartOfFile => None,
                Position::EndOfFile => file_state.lines.last_key_value().map(|(k, _)| k.clone()),
                Position::AfterAnchor => {
                    let anchor = req.context_anchor.as_ref().ok_or_else(|| {
                        anyhow!("`context_anchor` is required for `after_anchor` position.")
                    })?;
                    validate_anchor(
                        file_state,
                        &anchor.lid,
                        &anchor.line_content,
                        "insert",
                        "context_anchor",
                    )?;
                    Some(FileState::parse_lid(&anchor.lid)?.0)
                }
                Position::BeforeAnchor => {
                    let anchor = req.context_anchor.as_ref().ok_or_else(|| {
                        anyhow!("`context_anchor` is required for `before_anchor` position.")
                    })?;
                    validate_anchor(
                        file_state,
                        &anchor.lid,
                        &anchor.line_content,
                        "insert",
                        "context_anchor",
                    )?;
                    let anchor_lid = FileState::parse_lid(&anchor.lid)?.0;
                    // To insert *before* the anchor, we need the LID of the line *preceding* the anchor.
                    // We search backwards from the anchor's LID to find the key of the previous line.
                    file_state
                        .lines
                        .range(..anchor_lid)
                        .next_back()
                        .map(|(k, _)| k.clone())
                }
            };

            let new_content_with_suffixes: Vec<(String, String)> = req
                .new_content
                .iter()
                .map(|line| (line.clone(), crate::file_state::generate_random_suffix()))
                .collect();

            let internal_op = PatchOperation::Insert(InsertOp {
                after_lid,
                content: new_content_with_suffixes,
            });
            Ok((file_state.path.clone(), internal_op))
        })();

        match result {
            Ok((path, op)) => planned_ops.entry(path).or_default().push(op),
            Err(e) => {
                validation_errors.push(anyhow!(
                    "Insert request #{i} (file: '{}'): {e}",
                    req.file_path
                ));
            }
        }
    }

    if !validation_errors.is_empty() {
        let error_messages: Vec<String> = validation_errors.iter().map(|e| e.to_string()).collect();
        return Err(anyhow!(
            "Validation failed with {} error(s):\n- {}",
            validation_errors.len(),
            error_messages.join("\n- ")
        ));
    }

    Ok(EditPlan { planned_ops })
}

/// The main execution function for the `edit_file` tool.
pub fn execute_file_operations(
    args: &TopLevelRequest,
    file_state_manager: &mut FileStateManager,
    accessible_paths: &[String],
) -> Result<String> {
    if args.inserts.is_empty() && args.replaces.is_empty() && args.moves.is_empty() {
        return Ok("No file operations provided in the tool call.".to_string());
    }

    let plan = plan_file_operations(args, file_state_manager, accessible_paths)?;

    let mut results = Vec::new();

    // --- Phase 2: Execute the consolidated plan ---
    for (path, operations) in plan.planned_ops {
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

fn create_diff_preview(
    args: &TopLevelRequest,
    file_state_manager: &mut FileStateManager,
    accessible_paths: &[String],
) -> Result<String> {
    if args.inserts.is_empty() && args.replaces.is_empty() && args.moves.is_empty() {
        return Ok("No file edits will be performed.".to_string());
    }

    let plan = plan_file_operations(args, file_state_manager, accessible_paths)?;

    if plan.planned_ops.is_empty() {
        return Ok("No file operations would be performed after validation.".to_string());
    }

    let mut final_summary = Vec::new();
    if plan.planned_ops.len() > 1 {
        final_summary.push(format!("Edit {} files:", plan.planned_ops.len()));
    }

    for (path, operations) in &plan.planned_ops {
        let file_path_str = path.to_string_lossy();
        let file_state = file_state_manager.get_file_state_mut(&file_path_str)?;
        let diff = file_state.calculate_patch_diff(operations)?;

        final_summary.push(format!("{file_path_str} (diff):\n```\n{diff}\n```\n"));
    }

    Ok(final_summary.join("\n"))
}

#[cfg(test)]
#[path = "file_editor_tests.rs"]
mod file_editor_tests;
