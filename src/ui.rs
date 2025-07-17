use colored::Colorize;
use fancy_regex::Regex;
use once_cell::sync::Lazy;
use openrouter_api::types::chat::Message;
use std::fmt::Write;

static LIF_HEADER_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"---FILE: (?P<path>.+?) \(lif-hash: (?P<hash>[a-f0-9]+)\) \(length: (?P<length>\d+)\)---\n").unwrap()
});

/// Formats a message into a string with nice formatting.
/// It collapses file content blocks for user messages to reduce noise.
pub fn pretty_print_message(message: &Message) -> String {
    let mut buffer = String::new();

    let role_text = message.role.as_str();
    let role_colored = message.role.to_string().blue();
    writeln!(&mut buffer, "\n[{role_colored}]").unwrap();

    // Only for assistant messages that are requesting a tool call.
    if let Some(tool_calls) = &message.tool_calls {
        let _formatted_calls = format_tool_calls_pretty(tool_calls);
        // The output from the tool executor already prints the tool call request,
        // so we don't print it twice. We just process the content.
        // We will just print the content associated with the message, if any.
    }

    if !message.content.is_empty() {
        let is_user = role_text == "user";
        let mut final_content = String::new();

        if is_user {
            let mut last_match_end = 0;
            for mat in LIF_HEADER_REGEX.find_iter(&message.content) {
                let mat = mat.unwrap();
                // Append text between the last match and this one
                final_content.push_str(&message.content[last_match_end..mat.start()].cyan());

                let caps = LIF_HEADER_REGEX
                    .captures(&message.content[mat.start()..])
                    .unwrap()
                    .unwrap();
                let path = &caps["path"];
                let hash = &caps["hash"];
                let length: usize = caps["length"].parse().unwrap_or(0);

                let content_start = mat.end();
                let content_end = content_start + length;
                let content = &message.content[content_start..content_end];

                let line_count = content.lines().filter(|l| !l.is_empty()).count();
                let summary = format!("[{} (hash: {}) ({} lines)]", path, &hash[..8], line_count);
                final_content.push_str(&summary.dimmed());

                last_match_end = content_end;
            }
            // Append any remaining text after the last match
            final_content.push_str(&message.content[last_match_end..].cyan());
        } else {
            final_content.push_str(&message.content);
        }

        // The tool executor now handles printing its own colored output directly.
        // So we don't print the content for the 'tool' role here, as it would be a duplicate.
        if role_text != "tool" {
            for line in final_content.lines() {
                // Pre-colored lines (our summaries) will not be re-colored.
                // Other user prompt lines will be cyan.
                // \x1B is the escape character for ANSI color codes.
                if line.starts_with('\x1B') {
                    writeln!(&mut buffer, "{line}").unwrap();
                } else if role_text == "user" {
                    writeln!(&mut buffer, "{}", line.cyan()).unwrap();
                } else {
                    writeln!(&mut buffer, "{line}").unwrap();
                }
            }
        }
    }
    buffer
}

/// Formats a vec of tool calls into a string with nice formatting.
pub fn format_tool_calls_pretty(tool_calls: &[openrouter_api::models::tool::ToolCall]) -> String {
    let mut buffer = String::new();
    for (i, tool_call) in tool_calls.iter().enumerate() {
        if i > 0 {
            writeln!(&mut buffer).unwrap();
        }
        let call_header = format!("Tool Call: `{}`", tool_call.function_call.name).blue();
        writeln!(&mut buffer, "{call_header}").unwrap();

        let args_str = &tool_call.function_call.arguments;

        let formatted_args = match serde_json::from_str::<serde_json::Value>(args_str) {
            Ok(json_value) => {
                serde_json::to_string_pretty(&json_value).unwrap_or_else(|_| args_str.to_string())
            }
            Err(_) => args_str.to_string(),
        };

        for line in formatted_args.lines() {
            writeln!(&mut buffer, "  {}", line.dimmed()).unwrap();
        }
    }
    buffer
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
        let file_content =
            "line 1\nconst FAKE_HEADER: &str = \"---FILE: fake/path.rs---\";\nline 3";
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
}
