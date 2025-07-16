use crate::{config::Config, file_editor::is_path_editable, path_expander};
use anyhow::{Result, anyhow};
use colored::Colorize;
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize, Debug)]
pub struct ListFilesArgs {
    pub path: String,
}

pub fn list_files_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "list_files".to_string(),
            description: Some(
                "Lists all files in a given directory, respecting gitignore and other ignore rules. It is recursive."
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "The path to the directory to list files from."
                    }
                },
                "required": ["path"]
            }),
        },
    }
}

pub fn execute_list_files(args: &ListFilesArgs, config: &Config) -> Result<String> {
    let path_to_list = Path::new(&args.path);

    is_path_editable(path_to_list, &config.editable_paths)?;

    if !path_to_list.is_dir() {
        return Err(anyhow!(
            "Error: The provided path '{}' is not a directory.",
            path_to_list.display()
        ));
    }

    let expansion_result =
        path_expander::expand_and_validate(&[args.path.clone()], &config.ignored_paths);

    if expansion_result.files.is_empty() {
        return Ok(format!(
            "# No files found in '{}'. It might be empty or all files are ignored.",
            path_to_list.display().to_string().cyan()
        ));
    }

    let header = format!(
        "List of files in `{}`:\n",
        path_to_list.display().to_string().blue()
    );
    let file_list = expansion_result.files.join("\n");

    Ok(format!("{header}{file_list}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::fs;
    use tempfile::Builder;

    fn setup_test_dir() -> (tempfile::TempDir, Config) {
        let tmp_dir = Builder::new().prefix("test-list-files").tempdir().unwrap();
        let root_path = tmp_dir.path();

        fs::write(root_path.join("file1.txt"), "content1").unwrap();
        fs::create_dir(root_path.join("sub_dir")).unwrap();
        fs::write(root_path.join("sub_dir/file2.rs"), "content2").unwrap();
        fs::write(root_path.join("sub_dir/ignored.log"), "log").unwrap();
        fs::create_dir(root_path.join("ignored_dir")).unwrap();
        fs::write(root_path.join("ignored_dir/another.txt"), "ignored").unwrap();

        let config = Config {
            editable_paths: vec![root_path.to_str().unwrap().to_string()],
            ignored_paths: vec!["*.log".to_string(), "ignored_dir/".to_string()],
            ..Default::default()
        };

        (tmp_dir, config)
    }

    #[test]
    fn test_list_files_successfully() {
        let (tmp_dir, config) = setup_test_dir();
        let args = ListFilesArgs {
            path: tmp_dir.path().to_str().unwrap().to_string(),
        };

        let result = execute_list_files(&args, &config).unwrap();

        assert!(result.contains("file1.txt"));
        assert!(result.contains("sub_dir/file2.rs"));
        assert!(!result.contains("ignored.log"));
        assert!(!result.contains("ignored_dir"));
    }

    #[test]
    fn test_respects_ignore_rules() {
        let (tmp_dir, config) = setup_test_dir();
        let args = ListFilesArgs {
            path: tmp_dir.path().join("sub_dir").to_str().unwrap().to_string(),
        };

        let result = execute_list_files(&args, &config).unwrap();

        assert!(result.contains("file2.rs"));
        assert!(
            !result.contains("ignored.log"),
            "Should have ignored the log file"
        );
    }

    #[test]
    fn test_disallowed_path() {
        let (tmp_dir, mut config) = setup_test_dir();
        config.editable_paths = vec!["/some/other/safe/path".to_string()];

        let args = ListFilesArgs {
            path: tmp_dir.path().to_str().unwrap().to_string(),
        };

        let result = execute_list_files(&args, &config);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("not within any of the allowed editable paths"));
    }

    #[test]
    fn test_path_is_not_a_directory() {
        let (tmp_dir, config) = setup_test_dir();
        let file_path = tmp_dir.path().join("file1.txt");
        let args = ListFilesArgs {
            path: file_path.to_str().unwrap().to_string(),
        };

        let result = execute_list_files(&args, &config);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("is not a directory")
        );
    }

    #[test]
    fn test_empty_directory() {
        let (tmp_dir, config) = setup_test_dir();
        let empty_dir_path = tmp_dir.path().join("empty_test_dir");
        fs::create_dir(&empty_dir_path).unwrap();

        let args = ListFilesArgs {
            path: empty_dir_path.to_str().unwrap().to_string(),
        };

        let result = execute_list_files(&args, &config).unwrap();
        assert!(result.contains("No files found"));
    }
}
