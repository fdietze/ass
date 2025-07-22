//! # File Creator Tool
//!
//! This module provides the `create_file` tool, allowing the agent to create new files.

use crate::file_state_manager::FileStateManager;
use crate::permissions;
use anyhow::{Result, anyhow};
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::Deserialize;
use std::fs;
use std::path::Path;

/// Specifies a single file to be created in a batch operation.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct CreateFileSpec {
    /// The path for the new file to be created.
    pub file_path: String,
    /// The initial content of the new file, as a list of strings (lines).
    pub content: Vec<String>,
}

/// Represents the arguments for the `create_file` tool.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
pub struct CreateFileArgs {
    /// A list of files to be created.
    pub files: Vec<CreateFileSpec>,
}

pub fn create_file_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "create_file".to_string(),
            description: Some(
                "Creates one or more new files with specified content.
This operation is atomic: if any file creation fails, no files from the batch are created.
Returns the representation (content with line indexes and a hash) for each successfully created file. This information is required for any subsequent edits to the new files within the same session.

**Output Format**:
The output for each created file has the same format as the `read_file` tool, including a unique Line Identifier (LID) for each line.

Example `create_file` output for a new file `new.txt`:
```
File: new.txt | Hash: a1b2c3d4 | Lines: 1-2/2
1    80: first line
2    c0: second line
```

**How to Use the Output**:
When you want to edit this new file later, you must use the LIDs (`80`, `c0`) and the hash (`a1b2c3d4`) in your `edit_file` call.

If the file content comes from another file, instead of creating a file with the content, create an empty file and then use the move operation to move specific lines from the source file to the new file.
"
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "files": {
                        "type": "array",
                        "description": "A list of one or more files to create.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "file_path": {
                                    "type": "string",
                                    "description": "The relative path for the new file."
                                },
                                "content": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "The initial lines of content for the new file."
                                }
                            },
                            "required": ["file_path", "content"]
                        }
                    }
                },
                "required": ["files"]
            }),
        },
    }
}

pub fn execute_create_files(
    args: &CreateFileArgs,
    file_state_manager: &mut FileStateManager,
    accessible_paths: &[String],
) -> Result<String> {
    if args.files.is_empty() {
        return Ok("No files were specified for creation.".to_string());
    }

    let mut results = Vec::new();
    for spec in &args.files {
        let result = (|| {
            let path_to_create = Path::new(&spec.file_path);

            if path_to_create.exists() {
                return Err(anyhow!(
                    "File '{}' already exists. Use 'edit_file' to modify it.",
                    path_to_create.display()
                ));
            }

            permissions::is_path_accessible(path_to_create, accessible_paths)?;

            if let Some(parent) = path_to_create.parent() {
                fs::create_dir_all(parent)?;
            }

            let content = spec.content.join("\n");
            fs::write(path_to_create, &content)?;

            let file_state = file_state_manager.open_file(&spec.file_path)?;
            Ok(file_state.get_lif_representation())
        })();

        match result {
            Ok(success_msg) => results.push(format!("File: {}\n{}", spec.file_path, success_msg)),
            Err(e) => results.push(format!("File: {}\nError: {}", spec.file_path, e)),
        }
    }
    Ok(results.join("\n\n---\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_state_manager::FileStateManager;
    use std::fs;
    use tempfile::Builder;

    fn setup_test_file(content: &str) -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-creator-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        fs::write(&file_path, content).unwrap();
        (tmp_dir, file_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_execute_create_file_successfully() {
        let tmp_dir = Builder::new().prefix("test-creator-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("new_file.txt");
        let file_path_str = file_path.to_str().unwrap().to_string();
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let args = CreateFileArgs {
            files: vec![CreateFileSpec {
                file_path: file_path_str.clone(),
                content: vec!["hello".to_string(), "world".to_string()],
            }],
        };

        let result = execute_create_files(&args, &mut manager, &accessible_paths).unwrap();

        assert!(result.contains(&format!("File: {file_path_str}")));
        let file_state = manager.open_file(&file_path_str).unwrap();
        let indexes: Vec<_> = file_state.lines.keys().map(|k| k.to_string()).collect();
        assert!(result.contains(&format!("{}: hello", indexes[0])));
        assert!(result.contains(&format!("{}: world", indexes[1])));
        assert!(result.contains("Hash:"));

        let disk_content = fs::read_to_string(file_path).unwrap();
        assert_eq!(disk_content, "hello\nworld");
    }

    #[test]
    fn test_execute_create_file_already_exists() {
        let (_tmp_dir, file_path) = setup_test_file("existing content");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![_tmp_dir.path().to_str().unwrap().to_string()];

        let args = CreateFileArgs {
            files: vec![CreateFileSpec {
                file_path: file_path.clone(),
                content: vec!["new content".to_string()],
            }],
        };

        let result = execute_create_files(&args, &mut manager, &accessible_paths).unwrap();
        assert!(result.contains("Error: File"));
        assert!(result.contains("already exists"));
    }

    #[test]
    fn test_execute_create_file_not_editable() {
        let tmp_dir = Builder::new().prefix("test-creator-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("new_file.txt");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec!["/some/other/dir".to_string()]; // Disallowed

        let args = CreateFileArgs {
            files: vec![CreateFileSpec {
                file_path: file_path.to_str().unwrap().to_string(),
                content: vec!["content".to_string()],
            }],
        };

        let result = execute_create_files(&args, &mut manager, &accessible_paths).unwrap();
        assert!(result.contains("is not allowed"));
    }

    #[test]
    fn test_create_multiple_files() {
        let tmp_dir = Builder::new().prefix("test-creator-").tempdir().unwrap();
        let file_path1 = tmp_dir.path().join("new_file1.txt");
        let file_path2 = tmp_dir.path().join("new_file2.txt");
        let mut manager = FileStateManager::new();
        let accessible_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let args = CreateFileArgs {
            files: vec![
                CreateFileSpec {
                    file_path: file_path1.to_str().unwrap().to_string(),
                    content: vec!["file 1".to_string()],
                },
                CreateFileSpec {
                    file_path: file_path2.to_str().unwrap().to_string(),
                    content: vec!["file 2".to_string()],
                },
            ],
        };

        let result = execute_create_files(&args, &mut manager, &accessible_paths).unwrap();

        // Check result string
        assert!(result.contains(&format!("File: {}", file_path1.to_str().unwrap())));
        assert!(result.contains(&format!("File: {}", file_path2.to_str().unwrap())));
        assert!(result.contains("---"));

        // Check disk content
        assert_eq!(fs::read_to_string(file_path1).unwrap(), "file 1");
        assert_eq!(fs::read_to_string(file_path2).unwrap(), "file 2");
    }
}
