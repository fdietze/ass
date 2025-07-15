use anyhow::Result;
use colored::Colorize;
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::{Deserialize, Serialize};
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
        let error_message = format!(
            "Error: Command '{command}' is not allowed. Only commands starting with {allowed_prefixes:?} are permitted."
        );
        println!("{}", error_message.red());
        return Err(anyhow::anyhow!(error_message));
    }

    // --- Execution ---
    let output = Command::new("sh").arg("-c").arg(command).output()?;

    // --- Output Handling ---
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let status = output.status;

    let command_bold = command.bold();
    let result = format!(
        "> {command_bold}\n# Stdout:\n{stdout}\n# Stderr:\n{stderr}\n# Exit Code: {status}",
    );

    Ok(result)
}
