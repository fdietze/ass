use crate::{config::Config, file_editor::is_path_editable};
use anyhow::{Result, anyhow};
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::Deserialize;
use std::cmp::min;
use std::fs;
use std::path::Path;

#[derive(Deserialize, Debug)]
pub struct FileReadArgs {
    pub file_path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}

pub fn read_file_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "read_file".to_string(),
            description: Some(
                "Reads a file, optionally from a start line to an end line.
The output will be prefixed with line numbers."
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": {
                        "type": "string",
                        "description": "The relative path to the file to be read."
                    },
                    "start_line": {
                        "type": "integer",
                        "description": "The 1-indexed, inclusive, starting line number. Defaults to the beginning of the file."
                    },
                    "end_line": {
                        "type": "integer",
                        "description": "The 1-indexed, inclusive, ending line number. Defaults to the end of the file."
                    }
                },
                "required": ["file_path"]
            }),
        },
    }
}

pub fn execute_read_file(args: &FileReadArgs, config: &Config) -> Result<String> {
    let path_to_read = Path::new(&args.file_path);

    is_path_editable(path_to_read, &config.editable_paths)?;

    let content = fs::read_to_string(path_to_read)?;
    let lines: Vec<&str> = content.lines().collect();
    let line_count = lines.len();

    // If the file is empty, return a specific message
    if line_count == 0 {
        return Ok(format!("# File '{}' is empty.", path_to_read.display()));
    }

    let start_line = args.start_line.unwrap_or(1);
    let mut end_line = args.end_line.unwrap_or(line_count);

    if start_line == 0 {
        return Err(anyhow!(
            "Error: Line numbers are 1-indexed, but start_line was 0."
        ));
    }
    if start_line > end_line {
        return Err(anyhow!(
            "Error: start_line ({start_line}) cannot be greater than end_line ({end_line})."
        ));
    }
    if start_line > line_count {
        return Err(anyhow!(
            "Error: start_line ({start_line}) is out of bounds. The file '{}' has only {line_count} lines.",
            path_to_read.display()
        ));
    }
    end_line = min(end_line, line_count);

    let zero_based_start = start_line - 1;
    let mut zero_based_end = end_line;

    let mut truncated = false;
    let max_read_lines = config.max_read_lines as usize;
    if zero_based_end - zero_based_start > max_read_lines {
        zero_based_end = zero_based_start + max_read_lines;
        truncated = true;
    }

    let max_line_number_width = end_line.to_string().len();

    let selected_lines: Vec<String> = lines[zero_based_start..min(zero_based_end, line_count)]
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line_number = zero_based_start + i + 1;
            format!("{line_number: >max_line_number_width$} | {line}")
        })
        .collect();

    let mut output = selected_lines.join("\n");
    if truncated {
        output.push_str("\n... (file content truncated)");
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::io::Write;
    use tempfile::Builder;

    fn setup_test_file(content: &str) -> (tempfile::TempDir, String) {
        let tmp_dir = Builder::new().prefix("test-file-reader").tempdir().unwrap();
        let file_path = tmp_dir.path().join("test_file.txt");
        let mut file = fs::File::create(&file_path).unwrap();
        write!(file, "{content}").unwrap();
        (tmp_dir, file_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_read_full_file() {
        let (tmp_dir, file_path) = setup_test_file("line 1\nline 2\nline 3");
        let config = Config {
            editable_paths: vec![tmp_dir.path().to_str().unwrap().to_string()],
            ..Default::default()
        };
        let args = FileReadArgs {
            file_path,
            start_line: None,
            end_line: None,
        };

        let result = execute_read_file(&args, &config).unwrap();
        let expected = "1 | line 1\n2 | line 2\n3 | line 3";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_read_line_range() {
        let (tmp_dir, file_path) = setup_test_file("1\n2\n3\n4\n5");
        let config = Config {
            editable_paths: vec![tmp_dir.path().to_str().unwrap().to_string()],
            ..Default::default()
        };
        let args = FileReadArgs {
            file_path,
            start_line: Some(2),
            end_line: Some(4),
        };

        let result = execute_read_file(&args, &config).unwrap();
        let expected = "2 | 2\n3 | 3\n4 | 4";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_truncation() {
        let (tmp_dir, file_path) = setup_test_file("1\n2\n3\n4\n5");
        let config = Config {
            editable_paths: vec![tmp_dir.path().to_str().unwrap().to_string()],
            max_read_lines: 3,
            ..Default::default()
        };
        let args = FileReadArgs {
            file_path,
            start_line: None,
            end_line: None,
        };

        let result = execute_read_file(&args, &config).unwrap();
        let expected = "1 | 1\n2 | 2\n3 | 3\n... (file content truncated)";
        assert_eq!(result, expected);
    }

    #[test]
    fn test_start_line_out_of_bounds() {
        let (tmp_dir, file_path) = setup_test_file("1\n2");
        let config = Config {
            editable_paths: vec![tmp_dir.path().to_str().unwrap().to_string()],
            ..Default::default()
        };
        let args = FileReadArgs {
            file_path,
            start_line: Some(5),
            end_line: Some(5),
        };

        let result = execute_read_file(&args, &config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("start_line (5) is out of bounds")
        );
    }

    #[test]
    fn test_disallowed_path() {
        let (_tmp_dir, file_path) = setup_test_file("content");
        let config = Config {
            editable_paths: vec!["/some/other/path".to_string()],
            ..Default::default()
        };
        let args = FileReadArgs {
            file_path,
            start_line: None,
            end_line: None,
        };

        let result = execute_read_file(&args, &config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not within any of the allowed editable paths")
        );
    }

    #[test]
    fn test_empty_file() {
        let (tmp_dir, file_path) = setup_test_file("");
        let config = Config {
            editable_paths: vec![tmp_dir.path().to_str().unwrap().to_string()],
            ..Default::default()
        };
        let args = FileReadArgs {
            file_path: file_path.clone(),
            start_line: None,
            end_line: None,
        };

        let result = execute_read_file(&args, &config).unwrap();
        assert_eq!(
            result,
            format!("# File '{}' is empty.", Path::new(&file_path).display())
        );
    }
}
