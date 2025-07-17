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

use crate::file_state::{FileStateManager, PatchArgs};
use anyhow::{Result, anyhow};
use colored::Colorize;
use openrouter_api::models::tool::{FunctionDescription, Tool};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

/// Generates the tool schema for the `edit_file` tool.
///
/// ### Reasoning
/// A detailed and clear description is paramount. It acts as a form of "prompt engineering"
/// for the tool-use part of the LLM's brain. It explicitly tells the model what format
/// to use, what the operations mean, and provides a concrete example. This significantly
/// increases the reliability of the LLM's output.
pub fn edit_file_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "edit_file".to_string(),
            description: Some(
                "Edits a file using a line-based patch protocol (LIF-Patch).
IMPORTANT: The file's content (with LIDs and lif_hash) MUST be in your context before you can use this tool. If it's not, use `read_file` first.

**Strategy**:
- **Think in hunks**: A good patch is like a `git diff` hunk. Prefer to replace a whole logical block (like a function, `if` statement, or `for` loop) if you're making multiple changes within it. This is more robust than many small, scattered edits.
- **Refactor in one go**: When refactoring, apply all related changes to a file in a single tool call. For example, if you rename a function and a variable inside it, do it with one `patch` array, not two separate tool calls.
- **Avoid large, unchanged blocks**: While replacing hunks is good, don't replace hundreds of lines if only a few are changing. Find a balance.

**Operations**:
- **Replace/Delete**: `[\"r\", start_lid, end_lid, [\"new content\"]]`. To delete, provide an empty array for `new_content`.
- **Insert**: `[\"i\", after_lid, [\"new content\"]]`. Use `_START_OF_FILE_` for `after_lid` to insert at the beginning.

**Important**:
- Line identifiers (LIDs) MUST be the strings from when the file was read (e.g., 'LID1000'). NEVER use integer line numbers.
- The `lif_hash` MUST match the hash from when the file was last read.

**Example of a good 'hunk' patch**:
`{\"file_path\":\"src/main.rs\",\"lif_hash\":\"a1b2c3d4...\",\"patch\":[[\"r\",\"LID5000\",\"LID9000\",[\"fn new_function_signature() -> Result<()> {\", \"    // ... new function body ...\", \"    Ok(())\", \"}\"]]]}`"
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The relative path to the file to be patched."
                    },
                    "lif_hash": {
                        "type": "string",
                        "description": "The SHA-1 hash of the file state that this patch applies to. Must match the hash from when the file was last read."
                    },
                    "patch": {
                        "type": "array",
                        "description": "An array of patch operations, either replace/delete or insert.",
                        "items": {
                            "oneOf": [
                                {
                                    "type": "array",
                                    "description": "Replace a range of lines, like a 'diff hunk'. This is best for updating a whole function body or other logical block.",
                                    "prefixItems": [
                                        { "const": "r", "description": "Operation code for 'replace'." },
                                        { "type": "string", "description": "The starting line identifier. MUST be a string like 'LID1000'. DO NOT use an integer line number." },
                                        { "type": "string", "description": "The ending line identifier. MUST be a string like 'LID2000'. DO NOT use an integer line number. For a single line, start_lid and end_lid are the same." },
                                        {
                                            "type": "array",
                                            "items": { "type": "string" },
                                            "description": "The new lines of content to replace the specified range."
                                        }
                                    ],
                                    "minItems": 4,
                                    "maxItems": 4,
                                    "additionalItems": false
                                },
                                {
                                    "type": "array",
                                    "description": "Insert a new block of lines after a specific line. Use `_START_OF_FILE_` as the `after_lid` to insert at the top of the file.",
                                    "prefixItems": [
                                        { "const": "i", "description": "Operation code for 'insert'." },
                                        { "type": "string", "description": "The line identifier after which to insert. MUST be a string like 'LID3000' or '_START_OF_FILE_'. DO NOT use an integer line number." },
                                        {
                                            "type": "array",
                                            "items": { "type": "string" },
                                            "description": "The new lines of content to insert."
                                        }
                                    ],
                                    "minItems": 3,
                                    "maxItems": 3,
                                    "additionalItems": false
                                }
                            ]
                        }
                    }
                },
                "required": ["file_path", "lif_hash", "patch"]
            }),
        },
    }
}

/// Checks if a given file path is within the allowed editable directories.
/// This is a security measure to prevent the agent from editing unintended files.
pub fn is_path_editable(path_to_edit: &Path, editable_paths: &[String]) -> Result<()> {
    let canonical_path_to_edit = path_to_edit.canonicalize()?;

    let is_allowed = editable_paths.iter().any(|p| {
        if let Ok(canonical_editable_path) = Path::new(p).canonicalize() {
            canonical_path_to_edit.starts_with(canonical_editable_path)
        } else {
            false
        }
    });

    if !is_allowed {
        return Err(anyhow!(
            "Error: File '{}' is not within any of the allowed editable paths: {:?}.",
            path_to_edit.display(),
            editable_paths
        ));
    }

    Ok(())
}

/// Generates a colorized, human-readable diff between the old and new file states.
///
/// ### Reasoning
/// Providing a clear diff as the tool's output serves two purposes:
/// 1.  **User Feedback**: It shows the user exactly what changes the agent made.
/// 2.  **LLM Confirmation**: It gives the LLM a confirmation of the result of its action,
///     allowing it to verify if the edit was successful or if it needs to try again.
fn generate_custom_diff(
    old_lines: &BTreeMap<u64, String>,
    new_lines: &BTreeMap<u64, String>,
) -> String {
    let mut diff_lines = Vec::new();
    let old_keys: BTreeSet<_> = old_lines.keys().collect();
    let new_keys: BTreeSet<_> = new_lines.keys().collect();

    for &key in old_keys.difference(&new_keys) {
        diff_lines.push(
            format!("- LID{}: {}", key, old_lines[key])
                .red()
                .to_string(),
        );
    }
    for &key in new_keys.difference(&old_keys) {
        diff_lines.push(
            format!("+ LID{}: {}", key, new_lines[key])
                .green()
                .to_string(),
        );
    }
    for &key in new_keys.intersection(&old_keys) {
        if old_lines[key] != new_lines[key] {
            diff_lines.push(
                format!("- LID{}: {}", key, old_lines[key])
                    .red()
                    .to_string(),
            );
            diff_lines.push(
                format!("+ LID{}: {}", key, new_lines[key])
                    .green()
                    .to_string(),
            );
        }
    }

    if diff_lines.is_empty() {
        "No changes detected.".to_string()
    } else {
        diff_lines.join("\n")
    }
}

/// The main execution function for the `edit_file` tool.
///
/// This function orchestrates the entire patch process:
/// 1.  Validates the file path is editable.
/// 2.  Retrieves the current `FileState` from the `FileStateManager`.
/// 3.  Performs the critical hash check to ensure state consistency.
/// 4.  Applies the patch to the `FileState`.
/// 5.  Generates a diff of the changes.
/// 6.  Writes the new file content to disk.
/// 7.  Returns the generated diff and the new `lif_hash` to the LLM.
pub fn execute_file_patch(
    args: &PatchArgs,
    file_state_manager: &mut FileStateManager,
    editable_paths: &[String],
) -> Result<String> {
    let path_to_edit = Path::new(&args.file_path);
    is_path_editable(path_to_edit, editable_paths)?;

    let (diff, final_content, new_hash) = {
        let file_state = file_state_manager.open_file(&args.file_path)?;

        if args.lif_hash != file_state.lif_hash {
            return Err(anyhow!(
                "Hash mismatch for file '{}'. The file has changed since it was last read. Please read the file again before patching.",
                args.file_path
            ));
        }

        let old_lines = file_state.lines.clone();
        file_state.apply_patch(&args.patch)?;
        let diff = generate_custom_diff(&old_lines, &file_state.lines);
        let final_content = file_state.get_full_content();
        let new_hash = file_state.lif_hash.clone();
        (diff, final_content, new_hash)
    };

    // The file_state is out of scope now, releasing the mutable borrow on file_state_manager.
    // We write the changes to disk. The in-memory state in the manager is already updated.
    std::fs::write(path_to_edit, &final_content)?;

    Ok(format!(
        "Patch applied successfully. New lif_hash: {new_hash}. Changes:\n{diff}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_state::{FileStateManager, PatchOperation};
    use std::fs;
    use tempfile::Builder;

    fn setup_test_file(content: &str) -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-patcher-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        fs::write(&file_path, content).unwrap();
        (tmp_dir, file_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_execute_patch_successfully() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_hash = initial_state.lif_hash.clone();

        let args = PatchArgs {
            file_path: file_path.clone(),
            lif_hash: initial_hash.clone(),
            patch: vec![PatchOperation::Insert {
                after_lid: "LID1000".to_string(),
                content: vec!["line 2".to_string()],
            }],
        };

        let result = execute_file_patch(&args, &mut manager, &editable_paths);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Patch applied successfully."));
        assert!(output.contains("New lif_hash:"));
        assert!(output.contains(&"+ LID1500: line 2".green().to_string()));

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");

        // Verify that the manager's state has the new hash and it's returned in the output
        let final_state = manager.open_file(&file_path).unwrap();
        assert_ne!(final_state.lif_hash, initial_hash);
        assert!(output.contains(&final_state.lif_hash));
    }

    #[test]
    fn test_execute_patch_hash_mismatch() {
        let (_tmp_dir, file_path) = setup_test_file("line 1");
        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let args = PatchArgs {
            file_path: file_path.clone(),
            lif_hash: "wrong_hash".to_string(),
            patch: vec![],
        };

        let result = execute_file_patch(&args, &mut manager, &editable_paths);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Hash mismatch"));
    }
}
