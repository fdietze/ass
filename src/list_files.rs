use crate::{config::Config, path_expander, permissions, tools::Tool};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use openrouter_api::models::tool::FunctionDescription;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::file_state_manager::FileStateManager;

/// The main validation and planning logic for the `list_files` tool.
/// Ensures the path is an accessible, existing directory.
fn plan_list_files(args: &ListFilesArgs, config: &Config) -> Result<()> {
    let path_to_list = Path::new(&args.path);

    permissions::is_path_accessible(path_to_list, &config.accessible_paths)?;

    if !path_to_list.is_dir() {
        return Err(anyhow!(
            "Validation failed: The provided path '{}' is not a directory or does not exist.",
            path_to_list.display()
        ));
    }
    Ok(())
}

#[derive(Deserialize, Debug, Serialize)]
pub struct ListFilesArgs {
    pub path: String,
}

pub struct ListFilesTool;

#[async_trait]
impl Tool for ListFilesTool {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn schema(&self) -> FunctionDescription {
        FunctionDescription {
            name: "list_files".to_string(),
            description: Some(
                "Lists all files in a given directory recursively, respecting gitignore and other ignore rules."
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
        }
    }

    fn preview(
        &self,
        args: &Value,
        config: &Config,
        _fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let args: ListFilesArgs = serde_json::from_value(args.clone())?;
        plan_list_files(&args, config)?;
        execute_list_files(&args, config)
    }

    async fn execute(
        &self,
        args: &Value,
        config: &Config,
        _fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let args: ListFilesArgs = serde_json::from_value(args.clone())?;
        // The plan has already validated the path is a directory and accessible.
        plan_list_files(&args, config)?;
        execute_list_files(&args, config)
    }
}

pub fn execute_list_files(args: &ListFilesArgs, config: &Config) -> Result<String> {
    let path_to_list = Path::new(&args.path);

    // Initial validation is now done in the planner.
    // We can proceed with the assumption that the path is a valid directory.

    let expansion_result =
        path_expander::expand_and_validate(&[args.path.clone()], &config.ignored_paths);

    if expansion_result.files.is_empty() {
        return Ok(format!(
            "# No files found in '{}'. It might be empty or all files are ignored.",
            path_to_list.display()
        ));
    }

    let header = format!("Files in `{}`:\n", path_to_list.display());
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
            accessible_paths: vec![root_path.to_str().unwrap().to_string()],
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

        // Test planner
        assert!(plan_list_files(&args, &config).is_ok());

        // Test executor
        let result = execute_list_files(&args, &config).unwrap();

        assert!(result.contains("file1.txt"));
        assert!(result.contains("sub_dir/file2.rs"));
        assert!(!result.contains("ignored.log"));
        assert!(!result.contains("ignored_dir"));
    }

    #[test]
    fn test_respects_ignore_rules() {
        let (tmp_dir, config) = setup_test_dir();
        let sub_dir_path = tmp_dir.path().join("sub_dir");
        let args = ListFilesArgs {
            path: sub_dir_path.to_str().unwrap().to_string(),
        };

        assert!(plan_list_files(&args, &config).is_ok());
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
        config.accessible_paths = vec!["/some/other/safe/path".to_string()];

        let args = ListFilesArgs {
            path: tmp_dir.path().to_str().unwrap().to_string(),
        };

        let result = plan_list_files(&args, &config);
        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("is not allowed"));
    }

    #[test]
    fn test_path_is_not_a_directory() {
        let (tmp_dir, config) = setup_test_dir();
        let file_path = tmp_dir.path().join("file1.txt");
        let args = ListFilesArgs {
            path: file_path.to_str().unwrap().to_string(),
        };

        let result = plan_list_files(&args, &config);
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

        assert!(plan_list_files(&args, &config).is_ok());
        let result = execute_list_files(&args, &config).unwrap();
        assert!(result.contains("No files found"));
    }

    #[test]
    fn test_tool_preview() {
        let tool = ListFilesTool;
        let mut config = Config::default();
        let temp = Builder::new().prefix("test-preview").tempdir().unwrap();
        let src_path = temp.path().join("src");
        fs::create_dir(&src_path).unwrap();
        config.accessible_paths = vec![temp.path().to_str().unwrap().to_string()];
        let args = serde_json::json!({ "path": src_path.to_str().unwrap() });

        let fsm = Arc::new(Mutex::new(FileStateManager::new()));
        let preview = tool.preview(&args, &config, fsm.clone()).unwrap();
        assert!(preview.contains("No files found"));

        // Now add a file and check again
        let file_path = src_path.join("test.txt");
        fs::write(file_path, "content").unwrap();
        let preview_with_file = tool.preview(&args, &config, fsm).unwrap();
        assert!(preview_with_file.contains("test.txt"));
        assert!(preview_with_file.contains("Files in"));
    }
}
