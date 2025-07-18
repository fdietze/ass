use console::style;
use fancy_regex::Regex;
use once_cell::sync::Lazy;
use openrouter_api::types::chat::Message;
use serde_json::Value;
use std::fmt::Write;

static LIF_HEADER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"File: (?P<path>.+?) \| Hash: (?P<hash>[a-f0-9]+) \| Lines: (?P<start>\d+)-(?P<end>\d+)/(?P<total_lines>\d+)",
    )
    .unwrap()
});

/// Formats a message into a string with nice formatting.
/// It collapses file content blocks for user messages to reduce noise.
pub fn pretty_print_message(message: &Message) -> String {
    let mut buffer = String::new();

    let role_text = message.role.as_str();
    let role_colored = style(message.role.to_string()).blue();
    writeln!(&mut buffer, "\n[{role_colored}]").unwrap();

    if !message.content.is_empty() {
        let should_collapse =
            role_text == "user" || role_text == "system" || role_text == "assistant";
        let mut final_content = String::new();

        if should_collapse {
            let mut last_match_end = 0;
            for mat in LIF_HEADER_REGEX.find_iter(&message.content) {
                let mat = mat.unwrap();
                // Append text between the last match and this one
                final_content.push_str(&message.content[last_match_end..mat.start()]);

                let caps = LIF_HEADER_REGEX
                    .captures(&message.content[mat.start()..])
                    .unwrap()
                    .unwrap();
                let path = &caps["path"];
                let hash = &caps["hash"];
                let start: usize = caps["start"].parse().unwrap_or(0);
                let end: usize = caps["end"].parse().unwrap_or(0);
                let total_lines: usize = caps["total_lines"].parse().unwrap_or(0);

                let num_lines_in_body = if total_lines == 0 {
                    0 // Handles empty files which have Lines: 0-0/0
                } else {
                    end.saturating_sub(start) + 1
                };

                // The body content starts after the header line's newline.
                let body_start = if let Some(pos) = message.content[mat.end()..].find('\n') {
                    mat.end() + pos + 1
                } else {
                    message.content.len()
                };

                let content_end = if num_lines_in_body == 0 {
                    // For empty files or files with only a header, the "content" ends right after the header's newline.
                    body_start
                } else {
                    // Find the end of the body by finding the newline of the last line.
                    // This is more robust than scanning for the next header, which caused issues with
                    // multiple files being concatenated and trailing prompt text being swallowed.
                    message.content[body_start..]
                        .match_indices('\n')
                        .nth(num_lines_in_body - 1)
                        .map(|(i, _)| body_start + i) // The end is the position of the newline
                        .unwrap_or(message.content.len())
                };

                let summary = format!("[File: {path} | Hash: {hash} | {total_lines} lines]");
                final_content.push_str(&summary);

                last_match_end = content_end;
            }
            // Append any remaining text after the last match
            final_content.push_str(&message.content[last_match_end..]);
        } else {
            final_content.push_str(&message.content);
        }

        // The tool executor now handles printing its own colored output directly.
        // So we don't print the content for the 'tool' role here, as it would be a duplicate.
        if role_text != "tool" {
            for line in final_content.lines() {
                // Heuristic: If a line starts with `[` and contains the unique string "lines)]",
                // it's a summary line we generated.
                if line.trim().starts_with("[File: ") && line.contains("lines]") {
                    writeln!(&mut buffer, "{}", style(line).dim()).unwrap();
                } else if role_text == "user" {
                    // Any other line in a user message is part of their prompt.
                    writeln!(&mut buffer, "{}", style(line).cyan()).unwrap();
                } else {
                    // Assistant messages are printed without additional coloring.
                    writeln!(&mut buffer, "{line}").unwrap();
                }
            }
        }
    }
    buffer
}

pub fn pretty_print_json(json_string: &str) -> String {
    match serde_json::from_str::<Value>(json_string) {
        Ok(value) => {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| json_string.to_string())
        }
        Err(_) => json_string.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config, file_state::FileStateManager, prompt_builder::expand_file_mentions,
    };
    use std::fs;
    use tempfile::Builder;

    #[tokio::test]
    async fn test_build_and_collapse_prompt() {
        // --- Arrange ---
        let tmp_dir = Builder::new().prefix("test-collapse").tempdir().unwrap();
        let root = tmp_dir.path();

        let file1_path = root.join("test_file.txt");
        let file1_content = "line 1\nline 2\nline 3";
        fs::write(&file1_path, file1_content).unwrap();

        let config = Config::default();
        let original_prompt = format!("Check this file: @{}", file1_path.display());

        let mut file_state_manager = FileStateManager::new();

        // --- Act ---
        let expanded_prompt =
            expand_file_mentions(&original_prompt, &config, &mut file_state_manager)
                .await
                .unwrap();

        let message = Message {
            role: "user".to_string(),
            content: expanded_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        let output = pretty_print_message(&message);

        // --- Assert ---
        let uncolored_summary_part = format!("[File: {} | Hash:", file1_path.display());
        assert!(
            output.contains(&uncolored_summary_part),
            "Output should contain the summary path.\nOutput was:\n---\n{output}\n---"
        );
        assert!(
            output.contains("| 3 lines]"),
            "Output should contain the line count.\nOutput was:\n---\n{output}\n---"
        );
        assert!(
            !output.contains("LID1000"),
            "Output should not contain raw LIF content."
        );
    }

    #[tokio::test]
    async fn test_collapse_file_containing_file_header() {
        // --- Arrange ---
        let tmp_dir = Builder::new().prefix("test-delimiter-").tempdir().unwrap();
        let file_path = tmp_dir.path().join("file_with_delimiter.rs");
        let file_content = "line 1\nconst FAKE_HEADER: &str = \"File: fake/path.rs | Hash: fakehash | Lines: 1-1/1\";\nline 3";
        fs::write(&file_path, file_content).unwrap();

        let config = Config::default();
        let original_prompt = format!("Check this file: @{}", file_path.display());
        let mut file_state_manager = FileStateManager::new();

        // --- Act ---
        let expanded_prompt =
            expand_file_mentions(&original_prompt, &config, &mut file_state_manager)
                .await
                .unwrap();

        let message = Message {
            role: "user".to_string(),
            content: expanded_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        let output = pretty_print_message(&message);

        // --- Assert ---
        assert!(
            output.contains("| 3 lines]"),
            "Summary should count all 3 lines, not be truncated by the fake delimiter."
        );
        assert_eq!(
            output.matches("lines]").count(),
            1,
            "There should be exactly one file summary block."
        );
    }

    #[tokio::test]
    async fn test_collapse_assistant_message_with_file_content() {
        // --- Arrange ---
        let tmp_dir = Builder::new()
            .prefix("test-collapse-asst")
            .tempdir()
            .unwrap();
        let root = tmp_dir.path();

        let file1_path = root.join("test_file_assistant.txt");
        let file1_content = "assistant line 1\nassistant line 2";
        fs::write(&file1_path, file1_content).unwrap();

        let config = Config::default();
        let mut file_state_manager = FileStateManager::new();

        // Simulate an expanded prompt in the assistant's message content
        let expanded_content = expand_file_mentions(
            &format!("File: @{}", file1_path.display()),
            &config,
            &mut file_state_manager,
        )
        .await
        .unwrap();

        let message = Message {
            role: "assistant".to_string(),
            content: expanded_content,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        // --- Act ---
        let output = pretty_print_message(&message);

        // --- Assert ---
        let uncolored_summary_part = format!("[File: {} | Hash:", file1_path.display());
        assert!(
            output.contains(&uncolored_summary_part),
            "Output for assistant should contain summary path.\nOutput:\n{output}"
        );
        assert!(
            output.contains("| 2 lines]"),
            "Output for assistant should contain line count.\nOutput:\n{output}"
        );
        assert!(
            !output.contains("LID1000"),
            "Output for assistant should not contain raw LIF content."
        );
        assert!(
            !output.contains("assistant line 1"),
            "Output for assistant should not contain raw file content."
        );
    }

    #[tokio::test]
    async fn test_collapse_multiple_files_and_preserve_trailing_text() {
        // --- Arrange ---
        let tmp_dir = Builder::new()
            .prefix("test-multi-collapse")
            .tempdir()
            .unwrap();
        let root = tmp_dir.path();

        let file1_path = root.join("file1.txt");
        fs::write(&file1_path, "line a\nline b").unwrap();

        let file2_path = root.join("file2.txt");
        fs::write(&file2_path, "line x\nline y").unwrap();

        let config = Config::default();
        let original_prompt = format!(
            "Check @{} and @{}. This text should be preserved",
            file1_path.display(),
            file2_path.display()
        );

        let mut file_state_manager = FileStateManager::new();

        // --- Act ---
        let expanded_prompt =
            expand_file_mentions(&original_prompt, &config, &mut file_state_manager)
                .await
                .unwrap();

        let message = Message {
            role: "user".to_string(),
            content: expanded_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        let output = pretty_print_message(&message);

        // --- Assert ---
        // 1. Check that both file summaries are present
        let file1_summary = format!("[File: {} |", file1_path.display());
        let file2_summary = format!("[File: {} |", file2_path.display());
        assert!(output.contains(&file1_summary));
        assert!(output.contains(&file2_summary));

        // 2. Check that the original prompt text (that appears before the attachments) is preserved
        assert!(output.contains("This text should be preserved"));

        // 3. Check that the summaries are on separate lines, which is the core of the bug fix.
        // The pretty_print_message function splits the processed content by lines to format it.
        // If the summaries were concatenated, this count would be 1.
        let summary_lines_count = output
            .lines()
            .filter(|line| line.contains("[File:") && line.contains("lines]"))
            .count();
        assert_eq!(
            summary_lines_count, 2,
            "Should find two separate summary lines for the two files."
        );

        // 4. Ensure original file content is gone
        assert!(!output.contains("LID1000"));
        assert!(!output.contains("line a"));
        assert!(!output.contains("line x"));
    }
}
