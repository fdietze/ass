use crate::{
    config::Config,
    file_creator::{CreateFileArgs, execute_create_files},
    file_editor::{FileOperationArgs, execute_file_operations},
    file_reader::{FileReadArgs, execute_read_file},
    file_state_manager::FileStateManager,
    list_files::{ListFilesArgs, execute_list_files},
    shell::{ShellCommandArgs, execute_shell_command},
};
use anyhow::Result;
use console::style;
use openrouter_api::models::tool::ToolCall;
use openrouter_api::types::chat::Message;
use std::sync::{Arc, Mutex};
use strip_ansi_escapes::strip_str;

pub async fn handle_tool_call(
    tool_call: &ToolCall,
    config: &Config,
    file_state_manager: Arc<Mutex<FileStateManager>>,
) -> Result<Message> {
    let function_name = &tool_call.function_call.name;

    let tool_call_id = Some(tool_call.id.clone());
    match function_name.as_str() {
        "execute_shell_command" => {
            let result = match serde_json::from_str::<ShellCommandArgs>(
                &tool_call.function_call.arguments,
            ) {
                Ok(args) => {
                    execute_shell_command(&args.command, &config.allowed_command_prefixes).await
                }
                Err(e) => Err(anyhow::anyhow!(
                    "Error: Invalid arguments provided for {function_name}: {e}"
                )),
            };

            let (colored_output, uncolored_output) = match result {
                Ok(output) => (output.clone(), strip_str(&output)),
                Err(e) => {
                    let error_msg = format!("Error executing command: {e}");
                    (style(error_msg.clone()).red().to_string(), error_msg)
                }
            };

            println!("{colored_output}");
            Ok(Message {
                role: "tool".to_string(),
                content: uncolored_output,
                name: Some(function_name.clone()),
                tool_call_id,
                tool_calls: None,
            })
        }
        "create_file" => {
            let args_result =
                serde_json::from_str::<CreateFileArgs>(&tool_call.function_call.arguments);
            let result = match args_result {
                Ok(args) => {
                    let accessible_paths = config.accessible_paths.clone();
                    let manager_clone = Arc::clone(&file_state_manager);
                    tokio::task::spawn_blocking(move || {
                        let mut manager = manager_clone.lock().unwrap();
                        execute_create_files(&args, &mut manager, &accessible_paths)
                    })
                    .await?
                }
                Err(e) => Err(anyhow::anyhow!(
                    "Error: Invalid arguments provided for {function_name}: {e}"
                )),
            };

            let (colored_output, uncolored_output) = match result {
                Ok(output) => (output.clone(), strip_str(&output)),
                Err(e) => {
                    let error_msg = format!("Error creating file: {e}");
                    (style(error_msg.clone()).red().to_string(), error_msg)
                }
            };

            println!("{colored_output}");
            Ok(Message {
                role: "tool".to_string(),
                content: uncolored_output,
                name: Some(function_name.clone()),
                tool_call_id,
                tool_calls: None,
            })
        }
        "edit_file" => {
            let args_result =
                serde_json::from_str::<FileOperationArgs>(&tool_call.function_call.arguments);
            let result = match args_result {
                Ok(args) => {
                    let accessible_paths = config.accessible_paths.clone();
                    let manager_clone = Arc::clone(&file_state_manager);
                    tokio::task::spawn_blocking(move || {
                        let mut manager = manager_clone.lock().unwrap();
                        execute_file_operations(&args, &mut manager, &accessible_paths)
                    })
                    .await?
                }
                Err(e) => Err(anyhow::anyhow!(
                    "Error: Invalid arguments provided for {function_name}: {e}"
                )),
            };
            let (colored_output, uncolored_output) = match result {
                Ok(output) => (output.clone(), strip_str(&output)),
                Err(e) => {
                    let error_msg = format!("Error executing file operation: {e}");
                    (style(error_msg.clone()).red().to_string(), error_msg)
                }
            };

            println!("{colored_output}");
            Ok(Message {
                role: "tool".to_string(),
                content: uncolored_output,
                name: Some(function_name.clone()),
                tool_call_id,
                tool_calls: None,
            })
        }
        "read_file" => {
            let args_result =
                serde_json::from_str::<FileReadArgs>(&tool_call.function_call.arguments);
            let result = match args_result {
                Ok(args) => {
                    let config_clone = config.clone();
                    let manager_clone = Arc::clone(&file_state_manager);
                    tokio::task::spawn_blocking(move || {
                        let mut manager = manager_clone.lock().unwrap();
                        execute_read_file(&args, &config_clone, &mut manager)
                    })
                    .await?
                }
                Err(e) => Err(anyhow::anyhow!(
                    "Error: Invalid arguments provided for {function_name}: {e}"
                )),
            };

            let (colored_output, uncolored_output) = match result {
                Ok(output) => (output.clone(), strip_str(&output)),
                Err(e) => {
                    let error_msg = format!("Error reading file: {e}");
                    (style(error_msg.clone()).red().to_string(), error_msg)
                }
            };

            println!("{colored_output}");
            Ok(Message {
                role: "tool".to_string(),
                content: uncolored_output,
                name: Some(function_name.clone()),
                tool_call_id,
                tool_calls: None,
            })
        }
        "list_files" => {
            let result =
                match serde_json::from_str::<ListFilesArgs>(&tool_call.function_call.arguments) {
                    Ok(args) => {
                        let config_clone = config.clone();
                        tokio::task::spawn_blocking(move || {
                            execute_list_files(&args, &config_clone)
                        })
                        .await?
                    }
                    Err(e) => Err(anyhow::anyhow!(
                        "Error: Invalid arguments provided for {function_name}: {e}"
                    )),
                };

            let (colored_output, uncolored_output) = match result {
                Ok(output) => (output.clone(), strip_str(&output)),
                Err(e) => {
                    let error_msg = format!("Error listing files: {e}");
                    (style(error_msg.clone()).red().to_string(), error_msg)
                }
            };
            println!("{colored_output}");
            Ok(Message {
                role: "tool".to_string(),
                content: uncolored_output,
                name: Some(function_name.clone()),
                tool_call_id,
                tool_calls: None,
            })
        }
        _ => Ok(Message {
            role: "tool".to_string(),
            content: format!("Error: Unknown tool '{function_name}'"),
            name: Some(function_name.clone()),
            tool_call_id,
            tool_calls: None,
        }),
    }
}
