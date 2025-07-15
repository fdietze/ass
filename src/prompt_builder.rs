use crate::{enricher, path_expander};
use anyhow::Result;
use colored::Colorize;

pub async fn build_user_prompt(
    original_prompt: &str,
    config: &crate::config::Config,
) -> Result<String> {
    let enrichments = enricher::extract_enrichments(original_prompt);
    if enrichments.mentioned_files.is_empty() {
        return Ok(original_prompt.to_string());
    }

    let expansion_result =
        path_expander::expand_and_validate(&enrichments.mentioned_files, &config.ignored_paths);

    for not_found_path in &expansion_result.not_found {
        eprintln!(
            "{} Could not find file: {}",
            "Warning:".yellow(),
            not_found_path
        );
    }

    let mut content_parts = Vec::new();
    for file_path in &expansion_result.files {
        match tokio::fs::read_to_string(file_path).await {
            Ok(content) => content_parts.push((file_path.clone(), content)),
            Err(e) => eprintln!(
                "{} Failed to read file {}: {}",
                "Error:".red(),
                file_path,
                e
            ),
        }
    }

    if content_parts.is_empty() && expansion_result.not_found.is_empty() {
        return Ok(original_prompt.to_string());
    }

    let mut final_prompt = String::new();

    if !content_parts.is_empty() {
        final_prompt.push_str("Attached file contents:\n");
        for (path, content) in content_parts {
            final_prompt.push_str(&format!("### `{path}`\n"));
            final_prompt.push_str("```\n");
            final_prompt.push_str(&format_with_line_numbers(&content));
            final_prompt.push_str("\n```\n---\n");
        }
    }

    if !expansion_result.not_found.is_empty() {
        final_prompt.push_str(&format!(
            "Note: The following files were mentioned but could not be found and are not included: {}\n",
            expansion_result.not_found.join(", ")
        ));
    }

    final_prompt.push('\n');
    final_prompt.push_str(original_prompt);

    Ok(final_prompt)
}

fn format_with_line_numbers(content: &str) -> String {
    let line_count = content.lines().count();
    if line_count == 0 {
        return String::new();
    }
    let max_line_number_width = line_count.to_string().len();

    content
        .lines()
        .enumerate()
        .map(|(i, line)| {
            let line_number = i + 1;
            format!("{line_number: >max_line_number_width$} | {line}")
        })
        .collect::<Vec<String>>()
        .join("\n")
}
