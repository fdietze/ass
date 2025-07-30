use crate::{enricher, file_state_manager::FileStateManager, path_expander};
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Default, PartialEq)]
pub struct PromptData {
    pub final_prompt: String,
    pub file_summaries: Vec<String>,
    pub warnings: Vec<String>,
    pub has_mentions: bool,
}

pub fn process_prompt(
    original_prompt: &str,
    config: &crate::config::Config,
    file_state_manager: &mut FileStateManager,
) -> Result<PromptData> {
    let enrichments = enricher::extract_enrichments(original_prompt);
    if enrichments.mentioned_files.is_empty() {
        return Ok(PromptData {
            final_prompt: original_prompt.to_string(),
            ..Default::default()
        });
    }

    let mut warnings = Vec::new();
    let mut file_summaries = Vec::new();

    let mut directory_listings = String::new();
    for mentioned_path_str in &enrichments.mentioned_files {
        let path = Path::new(mentioned_path_str);
        if path.is_dir() {
            let expansion = path_expander::expand_and_validate(
                &[mentioned_path_str.clone()],
                &config.ignored_paths,
            );
            if !expansion.files.is_empty() {
                directory_listings.push_str(&format!(
                    "\nAttached directory listing for `{mentioned_path_str}`:\n"
                ));
                for file in expansion.files {
                    let file_path = Path::new(&file);
                    let filename = file_path.file_name().unwrap_or_default().to_string_lossy();
                    directory_listings.push_str(&format!("- {filename}\n"));
                }
            }
        }
    }

    let expansion_result =
        path_expander::expand_and_validate(&enrichments.mentioned_files, &config.ignored_paths);

    for not_found_path in &expansion_result.not_found {
        warnings.push(format!("Could not find file: {not_found_path}"));
    }

    let mut attached_files_content = String::new();
    for file_path in &expansion_result.files {
        match file_state_manager.open_file(file_path) {
            Ok(file_state) => {
                let total_lines = file_state.lines.len();
                let filename = std::path::Path::new(file_path)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                file_summaries.push(format!("[{filename} ({total_lines} lines)]"));
                attached_files_content.push_str(&file_state.display_lif_contents());
                attached_files_content.push('\n');
            }
            Err(e) => warnings.push(format! {
                "Failed to open file state for {file_path}: {e}"
            }),
        }
    }

    if attached_files_content.is_empty()
        && expansion_result.not_found.is_empty()
        && directory_listings.is_empty()
    {
        return Ok(PromptData {
            final_prompt: original_prompt.to_string(),
            warnings,
            file_summaries,
            has_mentions: !enrichments.mentioned_files.is_empty(),
        });
    }

    let mut final_prompt = String::new();
    final_prompt.push_str(original_prompt);

    if !directory_listings.is_empty() {
        final_prompt.push('\n');
        final_prompt.push_str(&directory_listings);
    }

    if !attached_files_content.is_empty() {
        final_prompt.push_str(&format!("\n\n{}\n", "Attached file contents:"));
        final_prompt.push_str(&attached_files_content);
    }

    if !expansion_result.not_found.is_empty() {
        final_prompt.push_str(&format! {
            "\nNote: The following files were mentioned but could not be found and are not included: {}\n",
            expansion_result.not_found.join(", ")
        });
    }

    Ok(PromptData {
        final_prompt,
        file_summaries,
        warnings,
        has_mentions: !enrichments.mentioned_files.is_empty(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_process_prompt_integration() {
        // Setup: Create a temporary directory and some test files
        let dir = tempdir().unwrap();
        let file1_path = dir.path().join("file1.txt");
        let dir2_path = dir.path().join("another_dir");
        fs::create_dir(&dir2_path).unwrap();
        let file3_path = dir2_path.join("file3.log");

        fs::write(&file1_path, "Hello, world!").unwrap();
        fs::write(&file3_path, "Log entry").unwrap();

        let file1_path_str = file1_path.to_str().unwrap();
        let dir2_path_str = dir2_path.to_str().unwrap();

        // Setup: Config and FileStateManager
        let config = config::Config::default();
        let mut fsm = FileStateManager::new();

        // The prompt with all kinds of mentions
        let original_prompt = format! {
            "Please look at @{file1_path_str}, check the directory @{dir2_path_str}, and what about @nonexistent.txt?"
        };

        // Act: Process the prompt
        let result = process_prompt(&original_prompt, &config, &mut fsm).unwrap();

        // Assert: Check the results
        assert!(result.has_mentions);

        // Assert: Warnings for nonexistent file
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("Could not find file: nonexistent.txt"));

        // Assert: Summaries for existing files and directories
        assert_eq!(result.file_summaries.len(), 2);
        assert!(
            result
                .file_summaries
                .contains(&"[file1.txt (1 lines)]".to_string())
        );
        assert!(
            result
                .file_summaries
                .contains(&"[file3.log (1 lines)]".to_string())
        );

        // Assert: Final prompt contains all parts
        assert!(result.final_prompt.starts_with(&original_prompt));
        assert!(
            result
                .final_prompt
                .contains("Attached directory listing for")
        );
        assert!(result.final_prompt.contains("- file3.log"));
        assert!(result.final_prompt.contains("Attached file contents:"));
        assert!(result.final_prompt.contains("Hello, world!"));
        assert!(result.final_prompt.contains("nonexistent.txt"));
    }
}
