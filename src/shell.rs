//! # Shell Tool
//!
//! This module provides the `execute_shell_command` tool, which allows the agent
//! to run arbitrary shell commands.

use crate::config::Config;
use crate::permissions;
use crate::tools::Tool;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use console::style;
use openrouter_api::models::tool::FunctionDescription;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use crate::file_state_manager::FileStateManager;

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ShellCommandArgs {
    pub command: String,
    pub workdir: Option<String>,
}

pub struct ShellTool;

#[async_trait]
impl Tool for ShellTool {
    fn name(&self) -> &'static str {
        "execute_shell_command"
    }

    fn schema(&self) -> FunctionDescription {
        FunctionDescription {
            name: "execute_shell_command".to_string(),
            description: Some(
                "Executes a shell command.
This tool is powerful and can have side effects. Use it with caution.
The command will be executed in the current working directory of the application.
Live output from the command will be printed to the console. The final captured output (stdout and stderr) will be returned to you."
                    .to_string(),
            ),
            strict: Some(true),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to execute."
                    },
                    "workdir": {
                        "type": ["string", "null"],
                        "description": "The working directory to run the command in. Defaults to the current working directory."
                    }
                },
                "additionalProperties": false,
                "required": ["command", "workdir"]
            }),
        }
    }

    fn preview(
        &self,
        args: &Value,
        _config: &Config,
        _fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let args: ShellCommandArgs = serde_json::from_value(args.clone())?;
        let mut output = vec![];
        if let Some(workdir) = args.workdir {
            output.push(format!("Workdir: {workdir}"));
        }
        output.push(format!("$ {}", style(args.command).bold()));
        Ok(output.join("\n"))
    }

    async fn execute(
        &self,
        args: &Value,
        _config: &Config,
        _fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let args: ShellCommandArgs = serde_json::from_value(args.clone())?;
        execute_shell_command(&args.command, args.workdir.as_deref()).await
    }

    fn is_safe_for_auto_execute(&self, args: &Value, config: &Config) -> Result<bool> {
        let args: ShellCommandArgs = serde_json::from_value(args.clone())?;

        // Check command prefix
        if permissions::is_command_allowed(&args.command, &config.allowed_command_prefixes).is_err()
        {
            return Ok(false);
        }

        // Check working directory
        if let Some(workdir) = &args.workdir {
            if permissions::is_path_accessible(Path::new(workdir), &config.accessible_paths)
                .is_err()
            {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

pub async fn execute_shell_command(command: &str, workdir: Option<&str>) -> Result<String> {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);

    if let Some(dir) = workdir {
        cmd.current_dir(dir);
    }

    let mut child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("Failed to capture stderr"))?;

    let mut stdout_reader = BufReader::new(stdout).lines();
    let mut stderr_reader = BufReader::new(stderr).lines();

    let mut output = String::new();
    let mut stdout_done = false;
    let mut stderr_done = false;

    loop {
        tokio::select! {
            // Always listen on stderr first or concurrently.
            line = stderr_reader.next_line(), if !stderr_done => {
                match line {
                    Ok(Some(line)) => {
                        eprintln!("{line}");
                        output.push_str(&line);
                        output.push('\n');
                    }
                    Ok(None) => stderr_done = true,
                    Err(e) => return Err(anyhow!("Error reading stderr: {e}")),
                }
            }
            line = stdout_reader.next_line(), if !stdout_done => {
                match line {
                    Ok(Some(line)) => {
                        println!("{line}");
                        output.push_str(&line);
                        output.push('\n');
                    }
                    Ok(None) => stdout_done = true,
                    Err(e) => return Err(anyhow!("Error reading stdout: {e}")),
                }
            }
        }
        if stdout_done && stderr_done {
            break;
        }
    }

    let status = child.wait().await?;
    let exit_message = if let Some(code) = status.code() {
        format!("Exit code: {code}")
    } else {
        "Process terminated by signal".to_string()
    };

    let styled_exit_message = style(&exit_message).bold().to_string();
    println!("\n{styled_exit_message}");

    output.push('\n');
    output.push_str(&exit_message);

    Ok(output)
}
