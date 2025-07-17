use colored::Colorize;
use fancy_regex::Regex;
use once_cell::sync::Lazy;
use openrouter_api::types::chat::Message;
use serde_json::Value;
use std::fmt::Write;

static LIF_HEADER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"---FILE: (?P<path>.+?) \(lif-hash: (?P<hash>[a-f0-9]+)\) \(lines \d+-\d+ of (?P<total_lines>\d+)\)---").unwrap()
});

/// Formats a message into a string with nice formatting.
/// It collapses file content blocks for user messages to reduce noise.
pub fn pretty_print_message(message: &Message) -> String {
    let mut buffer = String::new();

    let role_text = message.role.as_str();
    let role_colored = message.role.to_string().blue();
    writeln!(&mut buffer, "\n[{role_colored}]").unwrap();

    if !message.content.is_empty() {
        let should_collapse = role_text == "user" || role_text == "system";
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
                let total_lines: usize = caps["total_lines"].parse().unwrap_or(0);

                let content_start = mat.end();
                // We find the next occurrence of the delimiter to correctly capture the body,
                // even if the body itself contains something that looks like a delimiter.
                let next_delimiter_pos = LIF_HEADER_REGEX
                    .find_from_pos(&message.content, content_start)
                    .unwrap()
                    .map(|m| m.start())
                    .unwrap_or(message.content.len());

                let content_end = next_delimiter_pos;

                let summary = format!("[{} (hash: {}) ({} lines)]", path, &hash[..8], total_lines);
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
                if line.trim().starts_with('[') && line.contains("lines)]") {
                    writeln!(&mut buffer, "{}", line.dimmed()).unwrap();
                } else if role_text == "user" {
                    // Any other line in a user message is part of their prompt.
                    writeln!(&mut buffer, "{}", line.cyan()).unwrap();
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
        let uncolored_summary_part = format!("[{} (hash:", file1_path.display());
        assert!(
            output.contains(&uncolored_summary_part),
            "Output should contain the summary path.\nOutput was:\n---\n{output}\n---"
        );
        assert!(
            output.contains("(3 lines)"),
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
        let file_content = "line 1\nconst FAKE_HEADER: &str = \"---FILE: fake/path.rs (lines 1-1 of 1)---\";\nline 3";
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
            output.contains("(3 lines)"),
            "Summary should count all 3 lines, not be truncated by the fake delimiter."
        );
        assert_eq!(
            output.matches("lines)]").count(),
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
        let uncolored_summary_part = format!("[{} (hash:", file1_path.display());
        assert!(
            output.contains(&uncolored_summary_part),
            "Output for assistant should contain summary path.\nOutput:\n{output}"
        );
        assert!(
            output.contains("(2 lines)"),
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
}
