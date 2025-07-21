use crate::{enricher, file_state_manager::FileStateManager, path_expander};
use anyhow::Result;
use console::style;
use std::path::Path;

pub async fn expand_file_mentions(
    original_prompt: &str,
    config: &crate::config::Config,
    file_state_manager: &mut FileStateManager,
) -> Result<String> {
    let enrichments = enricher::extract_enrichments(original_prompt);
    if enrichments.mentioned_files.is_empty() {
        return Ok(original_prompt.to_string());
    }

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
                    directory_listings.push_str(&format!("- {file}\n"));
                }
            }
        }
    }

    let expansion_result =
        path_expander::expand_and_validate(&enrichments.mentioned_files, &config.ignored_paths);

    for not_found_path in &expansion_result.not_found {
        eprintln!(
            "{} Could not find file: {}",
            style("Warning:").yellow(),
            not_found_path
        );
    }

    let mut attached_files_content = String::new();
    for file_path in &expansion_result.files {
        match file_state_manager.open_file(file_path) {
            Ok(file_state) => {
                attached_files_content.push_str(&file_state.get_lif_representation());
                attached_files_content.push('\n');
            }
            Err(e) => eprintln!(
                "{} Failed to open file state for {}: {}",
                style("Error:").red(),
                file_path,
                e
            ),
        }
    }

    if attached_files_content.is_empty()
        && expansion_result.not_found.is_empty()
        && directory_listings.is_empty()
    {
        return Ok(original_prompt.to_string());
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
        final_prompt.push_str(&format!(
            "\nNote: The following files were mentioned but could not be found and are not included: {}\n",
            expansion_result.not_found.join(", ")
        ));
    }

    Ok(final_prompt)
}
