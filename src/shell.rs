use anyhow::Result;
use console::style;
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::process::Command;

/// Defines the arguments structure for our shell command tool.
/// The LLM will populate this structure.
#[derive(Serialize, Deserialize)]
pub struct ShellCommandArgs {
    pub command: String,
}

/// Creates the tool schema that tells the LLM how to call our shell command function.
pub fn shell_tool_schema() -> Tool {
    Tool::Function {
        function: FunctionDescription {
            name: "execute_shell_command".to_string(),
            description: Some(
                "Executes a shell command if it is on the whitelist.
Only use one command per call."
                    .to_string(),
            ),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute. For example: ls -l"
                    }
                },
                "required": ["command"]
            }),
        },
    }
}

/// Executes a shell command after ensuring it's on the whitelist.
pub fn execute_shell_command(command: &str, allowed_prefixes: &[String]) -> Result<String> {
    // --- Whitelist Check ---
    if !allowed_prefixes
        .iter()
        .any(|prefix| command.starts_with(prefix))
    {
        println!(
            "{}",
            style(format!(
                "Warning: The command `{command}` is not in the configured whitelist: {allowed_prefixes:?}."
            ))
            .yellow()
        );
        print!("Do you want to execute it anyway? (y/N) ");
        io::stdout().flush()?;

        let mut user_input = String::new();
        io::stdin().read_line(&mut user_input)?;

        if user_input.trim().to_lowercase() != "y" {
            return Err(anyhow::anyhow!("Command execution aborted by user."));
        }
    }

    // --- Execution ---
    // To interleave stdout and stderr, we redirect stderr to stdout (2>&1).
    // This gives us a single, combined output stream, just like in a terminal.
    let command_with_redirect = format!("{command} 2>&1");
    let output = Command::new("sh")
        .arg("-c")
        .arg(command_with_redirect)
        .output()?;

    // --- Output Handling ---
    let interleaved_output = String::from_utf8_lossy(&output.stdout);
    let status = output.status;

    let command_bold = style(command).bold();
    let exit_code_colored = if status.success() {
        style(status.to_string()).green()
    } else {
        style(status.to_string()).red()
    };

    let mut result = format!("> {command_bold}");

    if !interleaved_output.trim().is_empty() {
        result.push_str(&format!("\n{interleaved_output}"));
    }

    result.push_str(&format!("\n{exit_code_colored}"));

    Ok(result)
}
