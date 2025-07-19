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

use crate::file_state::{FileStateManager, PatchOperation};
use anyhow::{Result, anyhow};
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::{Deserialize, Deserializer};
use std::fs;
use std::path::Path;

/// Represents the arguments for a single file patch operation within a batch.
#[derive(Deserialize, Debug)]
pub struct PatchArgs {
    /// The path to the file that the patch should be applied to.
    pub file_path: String,
    /// The hash of the file state (`lif_hash`) that the LLM is basing this patch on.
    /// This is the key to preventing state desynchronization.
    pub lif_hash: String,
    /// A sequence of operations that constitute the patch.
    pub patch: Vec<PatchOperation>,
}

/// Represents the arguments for a single file creation operation.
#[derive(Deserialize, Debug)]
pub struct CreateFileArgs {
    /// The path to the file to be created.
    pub file_path: String,
    /// The initial content of the new file, as a list of strings (lines).
    pub content: Vec<String>,
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
pub struct FileOperationArgs {
    /// A list of patch operations to be applied to one or more existing files.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub edits: Vec<PatchArgs>,
    /// A list of creation operations for new files.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub creates: Vec<CreateFileArgs>,
}

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
                "Creates new files or edits existing ones using a line-based patch protocol (LIF-Patch). LIDs are stable across edits.

It can create and edit multiple files in a single call.

**Operations**:
- **Create File**: Use the `creates` property to make one or more new files with initial content.
- **Edit File**: Use the `edits` property to apply patches to existing files.

**Patch Operations for `edits`**:
- **Replace/Delete**: `[\"r\", start_lid, end_lid, [\"new content\"]]`. To delete, provide an empty `content` array.
- **Insert**: `[\"i\", after_lid, [\"new content\"]]`. Use `_START_OF_FILE_` for `after_lid` to insert at the beginning.

**IMPORTANT**: You get the required `lif_hash` and LIDs from file attachments, a `read_file` call, or the result of a previous `edit_file` call (including file creation). After a successful edit, the LIDs of unchanged lines remain valid for subsequent edits. You DO NOT need to re-read the file. If you just edited or created a file, **use the new `lif_hash` from the result** for your next operation. Only use `read_file` if the file isn't in your context or an edit failed because of a hash mismatch.

**Strategy**:
- **Think in hunks**: For edits, a good patch is like a `git diff` hunk. Prefer to replace a whole logical block (like a function or `if` statement).
- **Refactor in one go**: Apply all related changes (creations and edits) in a single tool call. For example, create a new module and then add it to `main.rs` in one step.

**Rules**:
- Line identifiers (LIDs) MUST be the strings from when the file was read (e.g., 'LID1000'). NEVER use the integer line numbers shown in the file view.
- The `lif_hash` for an edit MUST match the hash from when the file was last read, attached or edited.

**Example of a mixed operation**:
`{\"creates\":[{\"file_path\":\"src/new_helper.rs\",\"content\":[\"pub fn helper() {}\"]}],\"edits\":[{\"file_path\":\"src/main.rs\",\"lif_hash\":\"a1b2c3d4\",\"patch\":[[\"i\",\"LID1000\",[\"mod new_helper;\"]]]}]}`"
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "creates": {
                        "type": "array",
                        "description": "An array of file creation operations.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "file_path": {
                                    "type": "string",
                                    "description": "The relative path for the new file to be created."
                                },
                                "content": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "The initial lines of content for the new file."
                                }
                            },
                            "required": ["file_path", "content"]
                        }
                    },
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
                        }
                    }
                },
                "required": []
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

/// Checks if creating a file at the given path is allowed.
/// It inspects the parent directory of the path to ensure it's within the editable scope.
pub fn is_creation_path_safe(path_to_create: &Path, editable_paths: &[String]) -> Result<()> {
    let parent_dir = path_to_create.parent().ok_or_else(|| {
        anyhow!(
            "Cannot create file '{}' because it has no parent directory.",
            path_to_create.display()
        )
    })?;

    // If the parent is empty, it means the path is relative to the current dir.
    let dir_to_check = if parent_dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        parent_dir
    };

    let canonical_parent_dir = dir_to_check.canonicalize().map_err(|e| {
        anyhow!(
            "Cannot resolve parent directory '{}': {}. It might not exist.",
            dir_to_check.display(),
            e
        )
    })?;

    let is_allowed = editable_paths.iter().any(|p| {
        if let Ok(canonical_editable_path) = Path::new(p).canonicalize() {
            canonical_parent_dir.starts_with(canonical_editable_path)
        } else {
            false
        }
    });

    if !is_allowed {
        return Err(anyhow!(
            "Error: Cannot create file '{}' because its directory is not within any of the allowed editable paths: {:?}.",
            path_to_create.display(),
            editable_paths
        ));
    }

    Ok(())
}

/// The main execution function for the `edit_file` tool.
///
/// This function orchestrates the entire patch process:
/// 1.  Validates the file path is editable.
/// 2.  Retrieves the current `FileState` from the `FileStateManager`.
/// 3.  Performs the critical hash check to ensure state consistency.
/// 4.  Delegates the patch application, diffing, and file writing to the `FileState`.
/// 5.  Returns the generated diff and the new short `lif_hash` to the LLM.
pub fn execute_file_operations(
    args: &FileOperationArgs,
    file_state_manager: &mut FileStateManager,
    editable_paths: &[String],
) -> Result<String> {
    let mut results = Vec::new();

    if args.creates.is_empty() && args.edits.is_empty() {
        return Ok("No file operations provided in the tool call.".to_string());
    }

    // --- Handle Creations First ---
    for create in &args.creates {
        let result = (|| {
            let path_to_create = Path::new(&create.file_path);

            if path_to_create.exists() {
                return Err(anyhow!(
                    "File '{}' already exists. Use an 'edits' operation to modify it.",
                    path_to_create.display()
                ));
            }

            is_creation_path_safe(path_to_create, editable_paths)?;

            if let Some(parent) = path_to_create.parent() {
                fs::create_dir_all(parent)?;
            }

            let content = create.content.join("\n");
            fs::write(path_to_create, &content)?;

            let file_state = file_state_manager.open_file(&create.file_path)?;
            let lif_representation = file_state.get_lif_representation();

            Ok(format!("File created successfully.\n{lif_representation}"))
        })();

        match result {
            Ok(success_msg) => results.push(format!("File: {}\n{}", create.file_path, success_msg)),
            Err(e) => results.push(format!("File: {}\nError: {}", create.file_path, e)),
        }
    }

    // --- Handle Edits ---
    for edit in &args.edits {
        let result = (|| {
            let path_to_edit = Path::new(&edit.file_path);
            is_path_editable(path_to_edit, editable_paths)?;

            let file_state = file_state_manager.open_file(&edit.file_path)?;

            if edit.lif_hash != file_state.get_short_hash() {
                return Err(anyhow!(
                    "Hash mismatch. Expected hash '{}' but current hash is '{}'. The file has changed. Please read it again before patching.",
                    edit.lif_hash,
                    file_state.get_short_hash()
                ));
            }

            let diff = file_state.apply_and_write_patch(&edit.patch)?;
            let new_short_hash = file_state.get_short_hash().to_string();

            Ok(format!(
                "Patch from hash {old_hash} applied successfully. New lif_hash: {new_short_hash}. All LIDs from unchanged lines are still valid. Changes:\n{diff}",
                old_hash = edit.lif_hash,
                new_short_hash = new_short_hash,
                diff = diff,
            ))
        })();

        match result {
            Ok(success_msg) => results.push(format!("File: {}\n{}", edit.file_path, success_msg)),
            Err(e) => results.push(format!("File: {}\nError: {}", edit.file_path, e)),
        }
    }

    Ok(results.join("\n\n---\n\n"))
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
    fn test_execute_single_patch_successfully() {
        let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_path).unwrap();
        let initial_short_hash = initial_state.get_short_hash().to_string();

        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: initial_short_hash.clone(),
                patch: vec![PatchOperation::Insert {
                    after_lid: "LID1000".to_string(),
                    content: vec!["line 2".to_string()],
                }],
            }],
            creates: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &editable_paths);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Patch from hash"));
        assert!(output.contains(&file_path));

        let disk_content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(disk_content, "line 1\nline 2\nline 3");

        let final_state = manager.open_file(&file_path).unwrap();
        let final_short_hash = final_state.get_short_hash();
        assert_ne!(final_short_hash, initial_short_hash);
        assert!(output.contains(&format!("New lif_hash: {final_short_hash}")));
    }

    #[test]
    fn test_execute_patch_hash_mismatch() {
        let (_tmp_dir, file_path) = setup_test_file("line 1");
        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let args = FileOperationArgs {
            edits: vec![PatchArgs {
                file_path: file_path.clone(),
                lif_hash: "wrong_hash".to_string(),
                patch: vec![],
            }],
            creates: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &editable_paths);
        assert!(result.is_ok()); // The function itself doesn't error, it reports errors in the string
        let output = result.unwrap();
        assert!(output.contains("Error: Hash mismatch"));
        assert!(output.contains(&file_path));
    }

    #[test]
    fn test_execute_multiple_patches_with_partial_failure() {
        // --- Setup ---
        let (_tmp_dir, file1_path) = setup_test_file("file1 line1");
        let file2_path = _tmp_dir.path().join("file2.txt");
        fs::write(&file2_path, "file2 line1").unwrap();
        let file2_path_str = file2_path.to_str().unwrap().to_string();

        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let file1_initial_state = manager.open_file(&file1_path).unwrap();
        let file1_initial_hash = file1_initial_state.get_short_hash().to_string();

        let file2_initial_state = manager.open_file(&file2_path_str).unwrap();
        let file2_initial_hash = file2_initial_state.get_short_hash().to_string();

        // --- Args ---
        // Edit for file1 is valid
        let valid_edit = PatchArgs {
            file_path: file1_path.clone(),
            lif_hash: file1_initial_hash,
            patch: vec![PatchOperation::Insert {
                after_lid: "LID1000".to_string(),
                content: vec!["file1 line2".to_string()],
            }],
        };
        // Edit for file2 has a hash mismatch
        let invalid_edit = PatchArgs {
            file_path: file2_path_str.clone(),
            lif_hash: "wrong_hash".to_string(),
            patch: vec![],
        };

        let args = FileOperationArgs {
            edits: vec![valid_edit, invalid_edit],
            creates: vec![],
        };

        // --- Act ---
        let result = execute_file_operations(&args, &mut manager, &editable_paths).unwrap();

        // --- Assert ---
        // Check final string report
        assert!(result.contains(&format!("File: {file1_path}")));
        assert!(result.contains("Patch from hash"));
        assert!(result.contains(&format!("File: {file2_path_str}")));
        assert!(result.contains("Error: Hash mismatch"));

        // Check file1 on disk (should be changed)
        let file1_content = fs::read_to_string(&file1_path).unwrap();
        assert_eq!(file1_content, "file1 line1\nfile1 line2");

        // Check file2 on disk (should NOT be changed)
        let file2_content = fs::read_to_string(&file2_path).unwrap();
        assert_eq!(file2_content, "file2 line1");

        // Check hashes in manager
        let file1_final_hash = manager.open_file(&file1_path).unwrap().get_short_hash();
        assert!(result.contains(file1_final_hash));

        let file2_final_hash = manager.open_file(&file2_path_str).unwrap().get_short_hash();
        assert_eq!(file2_final_hash, file2_initial_hash); // Should be unchanged
    }

    #[test]
    fn test_execute_create_file_successfully() {
        let tmp_dir = Builder::new().prefix("test-creator-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("new_file.txt");
        let file_path_str = file_path.to_str().unwrap().to_string();
        let mut manager = FileStateManager::new();
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let args = FileOperationArgs {
            creates: vec![CreateFileArgs {
                file_path: file_path_str.clone(),
                content: vec!["hello".to_string(), "world".to_string()],
            }],
            edits: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &editable_paths).unwrap();

        assert!(result.contains("File created successfully."));
        assert!(result.contains("LID1000: hello"));
        assert!(result.contains("LID2000: world"));
        assert!(result.contains("Hash:"));

        let disk_content = fs::read_to_string(file_path).unwrap();
        assert_eq!(disk_content, "hello\nworld");
    }

    #[test]
    fn test_execute_create_file_already_exists() {
        let (_tmp_dir, file_path) = setup_test_file("existing content");
        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let args = FileOperationArgs {
            creates: vec![CreateFileArgs {
                file_path: file_path.clone(),
                content: vec!["new content".to_string()],
            }],
            edits: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &editable_paths).unwrap();
        assert!(result.contains("Error: File"));
        assert!(result.contains("already exists"));
    }

    #[test]
    fn test_execute_create_file_not_editable() {
        let tmp_dir = Builder::new().prefix("test-creator-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("new_file.txt");
        let mut manager = FileStateManager::new();
        let editable_paths = vec!["/some/other/dir".to_string()]; // Disallowed

        let args = FileOperationArgs {
            creates: vec![CreateFileArgs {
                file_path: file_path.to_str().unwrap().to_string(),
                content: vec!["content".to_string()],
            }],
            edits: vec![],
        };

        let result = execute_file_operations(&args, &mut manager, &editable_paths).unwrap();
        assert!(result.contains("is not within any of the allowed editable paths"));
    }

    #[test]
    fn test_execute_mixed_create_and_edit() {
        let (_tmp_dir, file_to_edit_path) = setup_test_file("edit this");
        let file_to_create_path = _tmp_dir.path().join("created.txt");
        let mut manager = FileStateManager::new();
        let editable_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let initial_state = manager.open_file(&file_to_edit_path).unwrap();
        let initial_hash = initial_state.get_short_hash().to_string();

        let args = FileOperationArgs {
            creates: vec![CreateFileArgs {
                file_path: file_to_create_path.to_str().unwrap().to_string(),
                content: vec!["new file content".to_string()],
            }],
            edits: vec![PatchArgs {
                file_path: file_to_edit_path.clone(),
                lif_hash: initial_hash,
                patch: vec![PatchOperation::ReplaceRange {
                    start_lid: "LID1000".to_string(),
                    end_lid: "LID1000".to_string(),
                    content: vec!["was edited".to_string()],
                }],
            }],
        };

        let result = execute_file_operations(&args, &mut manager, &editable_paths).unwrap();

        // Check create result
        assert!(result.contains("File created successfully."));
        assert!(fs::read_to_string(file_to_create_path).unwrap() == "new file content");

        // Check edit result
        assert!(result.contains("Patch from hash"));
        assert!(fs::read_to_string(file_to_edit_path).unwrap() == "was edited");
    }
}
