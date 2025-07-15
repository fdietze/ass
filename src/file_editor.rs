use anyhow::{Result, anyhow};
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Serialize, Deserialize, Debug)]
pub struct FileEditArgs {
    pub file_path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub replacement_content: String,
}

pub fn file_edit_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "edit_file".to_string(),
            description: Some(
                "Edits a file by replacing a range of lines.
To insert content, set end_line to be start_line - 1.
To delete content, provide an empty replacement_content string."
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The relative path to the file to be edited."
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "The 1-indexed starting line number of the range to be replaced."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "The 1-indexed ending line number of the range to be replaced. To insert, set this to start_line - 1."
                    },
                    "replacement_content": {
                        "type": "string",
                        "description": "The new content to write to the file. To delete, this should be an empty string."
                    }
                },
                "required": ["file_path", "start_line", "end_line", "replacement_content"]
            }),
        },
    }
}

/// Checks if a given file path is within the allowed editable directories.
///
/// This function canonicalizes both the file path and the allowed directory paths
/// to prevent directory traversal attacks and ensure a correct comparison.
fn is_path_editable(path_to_edit: &Path, editable_paths: &[String]) -> Result<()> {
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

/// Applies a line-based edit to a file.
///
/// This function reads a file, replaces a specified range of lines (or inserts/deletes),
/// and writes the content back to the file. It performs bounds checking.
fn apply_edit(
    path_to_edit: &Path,
    start_line: usize,
    end_line: usize,
    replacement_content: &str,
) -> Result<()> {
    if start_line == 0 {
        return Err(anyhow!(
            "Error: Line numbers are 1-indexed, but start_line was 0."
        ));
    }

    let file = fs::File::open(path_to_edit)?;
    let reader = BufReader::new(file);
    let lines: Vec<String> = reader.lines().collect::<Result<_, _>>()?;
    let line_count = lines.len();

    let is_insert = end_line + 1 == start_line;

    if !is_insert && start_line > end_line {
        return Err(anyhow!(
            "Error: start_line ({}) cannot be greater than end_line ({}).",
            start_line,
            end_line
        ));
    }

    if start_line > line_count + 1 {
        return Err(anyhow!(
            "Error: start_line ({}) is out of bounds. The file '{}' has only {} lines.",
            start_line,
            path_to_edit.display(),
            line_count
        ));
    }

    if !is_insert && end_line > line_count {
        return Err(anyhow!(
            "Error: end_line ({}) is out of bounds. The file '{}' has only {} lines.",
            end_line,
            path_to_edit.display(),
            line_count
        ));
    }

    // --- Edit Logic ---
    let mut new_lines = Vec::new();
    let zero_based_start = start_line - 1;

    // Add lines before the edit
    new_lines.extend_from_slice(&lines[0..zero_based_start]);

    // Add the new content
    if !replacement_content.is_empty() {
        new_lines.extend(replacement_content.lines().map(|l| l.to_string()));
    }

    // Add lines after the edit
    if !is_insert {
        let zero_based_end = end_line;
        if zero_based_end < line_count {
            new_lines.extend_from_slice(&lines[zero_based_end..]);
        }
    } else {
        // For an insert, we add back all the original lines from the insertion point
        if zero_based_start < line_count {
            new_lines.extend_from_slice(&lines[zero_based_start..]);
        }
    }

    // --- File Writing ---
    let new_content = new_lines.join("\n");
    fs::write(path_to_edit, new_content)?;

    Ok(())
}

pub fn execute_file_edit(args: &FileEditArgs, editable_paths: &[String]) -> Result<String> {
    let path_to_edit = Path::new(&args.file_path);

    is_path_editable(path_to_edit, editable_paths)?;

    apply_edit(
        path_to_edit,
        args.start_line,
        args.end_line,
        &args.replacement_content,
    )?;

    Ok(format!("Successfully edited '{}'.", args.file_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::Builder;

    // Helper to create a temporary directory and a file within it
    fn setup_test_file(content: &str) -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-file-editor").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        write!(file, "{content}").unwrap();
        (tmp_dir, file_path.to_str().unwrap().to_string())
    }

    // --- Unit tests for the internal logic ---

    mod permission_tests {
        use super::*;

        #[test]
        fn allows_path_in_editable_dir() {
            let (tmp_dir, file_path) = setup_test_file("content");
            let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];
            let result = is_path_editable(Path::new(&file_path), &editable_paths);
            assert!(result.is_ok());
        }

        #[test]
        fn disallows_path_outside_editable_dir() {
            let (_tmp_dir, file_path) = setup_test_file("content");
            let another_dir = Builder::new().prefix("another").tempdir().unwrap();
            let editable_paths = vec![another_dir.path().to_str().unwrap().to_string()];
            let result = is_path_editable(Path::new(&file_path), &editable_paths);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("not within any of the allowed editable paths")
            );
        }

        #[test]
        fn allows_path_in_subdirectory() {
            let tmp_dir = Builder::new().prefix("test-subdir").tempdir().unwrap();
            let sub_dir = tmp_dir.path().join("sub");
            fs::create_dir(&sub_dir).unwrap();
            let file_path = sub_dir.join("sub_file.txt");
            fs::write(&file_path, "sub content").unwrap();

            let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];
            let result = is_path_editable(Path::new(&file_path), &editable_paths);
            assert!(result.is_ok());
        }

        #[test]
        fn errors_on_non_existent_path() {
            let editable_paths = vec![".".to_string()];
            let result = is_path_editable(Path::new("non_existent_file.txt"), &editable_paths);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("No such file or directory")
            );
        }
    }

    mod edit_logic_tests {
        use super::*;

        #[test]
        fn test_apply_replace_single_line() {
            let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
            let path = Path::new(&file_path);
            apply_edit(path, 2, 2, "replacement").unwrap();
            let content = fs::read_to_string(path).unwrap();
            assert_eq!(content, "line 1\nreplacement\nline 3");
        }

        #[test]
        fn test_apply_insert_in_middle() {
            let (_tmp_dir, file_path) = setup_test_file("line 1\nline 3");
            let path = Path::new(&file_path);
            apply_edit(path, 2, 1, "line 2").unwrap();
            let content = fs::read_to_string(path).unwrap();
            assert_eq!(content, "line 1\nline 2\nline 3");
        }

        #[test]
        fn test_apply_delete_lines() {
            let (_tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
            let path = Path::new(&file_path);
            apply_edit(path, 2, 2, "").unwrap();
            let content = fs::read_to_string(path).unwrap();
            assert_eq!(content, "line 1\nline 3");
        }

        #[test]
        fn test_apply_out_of_bounds_error() {
            let (_tmp_dir, file_path) = setup_test_file("one line");
            let path = Path::new(&file_path);
            let result = apply_edit(path, 3, 3, "...");
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("start_line (3) is out of bounds")
            );
        }
    }

    // --- Integration tests for the public-facing function ---

    #[test]
    fn test_replace_single_line() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 2,
            end_line: 2,
            replacement_content: "replacement".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let result = execute_file_edit(&args, &editable_paths);
        assert!(result.is_ok());

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "line 1\nreplacement\nline 3");
    }

    #[test]
    fn test_replace_multiple_lines() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3\nline 4");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 2,
            end_line: 3,
            replacement_content: "new content".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        execute_file_edit(&args, &editable_paths).unwrap();

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "line 1\nnew content\nline 4");
    }

    #[test]
    fn test_delete_lines() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 2,
            end_line: 2,
            replacement_content: "".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        execute_file_edit(&args, &editable_paths).unwrap();

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "line 1\nline 3");
    }

    #[test]
    fn test_insert_at_beginning() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 2");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 1,
            end_line: 0,
            replacement_content: "new line".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        execute_file_edit(&args, &editable_paths).unwrap();

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "new line\nline 1\nline 2");
    }

    #[test]
    fn test_insert_in_middle() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 3");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 2,
            end_line: 1,
            replacement_content: "line 2".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        execute_file_edit(&args, &editable_paths).unwrap();

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "line 1\nline 2\nline 3");
    }

    #[test]
    fn test_insert_at_end() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 2");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 3,
            end_line: 2,
            replacement_content: "line 3".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        execute_file_edit(&args, &editable_paths).unwrap();

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "line 1\nline 2\nline 3");
    }

    #[test]
    fn test_error_on_non_whitelisted_file() {
        let (_tmp_dir, file_path) = setup_test_file("some content");
        let args = FileEditArgs {
            file_path,
            start_line: 1,
            end_line: 1,
            replacement_content: "nope".to_string(),
        };
        let another_tmp_dir = Builder::new().prefix("another-dir").tempdir().unwrap();
        let editable_paths = vec![another_tmp_dir.path().to_str().unwrap().to_string()];

        let result = execute_file_edit(&args, &editable_paths);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not within any of the allowed editable paths")
        );
    }

    #[test]
    fn test_error_on_non_existent_file() {
        let args = FileEditArgs {
            file_path: "non_existent_file.txt".to_string(),
            start_line: 1,
            end_line: 1,
            replacement_content: "wont work".to_string(),
        };
        let editable_paths = vec![".".to_string()];

        let result = execute_file_edit(&args, &editable_paths);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("No such file or directory")
        );
    }

    #[test]
    fn test_error_start_line_out_of_bounds() {
        let (tmp_dir, file_path) = setup_test_file("one line");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 3,
            end_line: 3,
            replacement_content: "...".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let result = execute_file_edit(&args, &editable_paths);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("start_line (3) is out of bounds")
        );
    }
    #[test]
    fn test_error_end_line_out_of_bounds() {
        let (tmp_dir, file_path) = setup_test_file("one line\ntwo lines");
        let args = FileEditArgs {
            file_path: file_path.clone(),
            start_line: 1,
            end_line: 5,
            replacement_content: "...".to_string(),
        };
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let result = execute_file_edit(&args, &editable_paths);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("end_line (5) is out of bounds")
        );
    }

    #[test]
    fn test_edit_in_subdirectory_is_allowed() {
        let tmp_dir = Builder::new().prefix("test-subdir").tempdir().unwrap();
        let sub_dir = tmp_dir.path().join("sub");
        fs::create_dir(&sub_dir).unwrap();
        let file_path = sub_dir.join("sub_file.txt");
        fs::write(&file_path, "sub content").unwrap();

        let args = FileEditArgs {
            file_path: file_path.to_str().unwrap().to_string(),
            start_line: 1,
            end_line: 1,
            replacement_content: "new sub content".to_string(),
        };
        // Allow editing in the parent directory
        let editable_paths = vec![tmp_dir.path().to_str().unwrap().to_string()];

        let result = execute_file_edit(&args, &editable_paths);
        assert!(result.is_ok());

        let content = fs::read_to_string(file_path).unwrap();
        assert_eq!(content, "new sub content");
    }
}
