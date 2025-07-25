use anyhow::Result;
use console::style;
use openrouter_api::models::tool::{FunctionDescription, Tool};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

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
pub async fn execute_shell_command(command: &str, allowed_prefixes: &[String]) -> Result<String> {
    // --- Whitelist Check ---
    if !allowed_prefixes
        .iter()
        .any(|prefix| command.starts_with(prefix))
    {
        println! {"{}", style(format!("Warning: The command `{command}` is not in the configured whitelist: {allowed_prefixes:?}. Executing anyway.")).yellow()};
    }

    // --- Execution ---
    println!("> {}", style(command).bold());

    // To interleave stdout and stderr, we redirect stderr to stdout (2>&1).
    // This gives us a single, combined output stream, just like in a terminal.
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to take stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("failed to take stderr"))?;

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let mut output_lines = Vec::new();

    loop {
        tokio::select! {
            line = stdout_reader.next_line() => {
                if let Some(line) = line? {
                    println!("{line}");
                    output_lines.push(line);
                } else {
                    break;
                }
            },
            line = stderr_reader.next_line() => {
                if let Some(line) = line? {
                    eprintln!("{line}");
                    output_lines.push(line);
                }
            },
            else => break,
        }
    }

    let status = child.wait().await?;
    let mut interleaved_output = output_lines.join("\n");

    // --- Output Handling ---
    let exit_code_string = status.to_string();
    let exit_code_colored = if status.success() {
        style(&exit_code_string).green()
    } else {
        style(&exit_code_string).red()
    };
    println!("{exit_code_colored}");

    // Append the uncolored status to the output for the LLM
    if !interleaved_output.is_empty() {
        interleaved_output.push('\n');
    }
    interleaved_output.push_str(&exit_code_string);

    Ok(interleaved_output)
}
