use crate::{
    config::Config, file_creator, file_editor, file_reader, file_state_manager, list_files, shell,
};
use anyhow::Result;
use console::style;
use openrouter_api::models::tool::ToolCall;
use openrouter_api::types::chat::Message;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::sync::{Arc, Mutex};
use strip_ansi_escapes::strip_str;

async fn execute_tool<A, F, Fut>(
    args_str: &str,
    function_name: &str,
    tool_call_id: String,
    error_context: &str,
    execute_fn: F,
) -> Result<Message>
where
    A: DeserializeOwned + Send + 'static,
    F: FnOnce(A) -> Fut + Send + 'static,
    Fut: Future<Output = Result<String>> + Send,
{
    let args: A = match serde_json::from_str(args_str) {
        Ok(args) => args,
        Err(e) => {
            return Ok(Message {
                role: "tool".to_string(),
                content: format!("Error: Invalid arguments for {function_name}: {e}"),
                name: Some(function_name.to_string()),
                tool_call_id: Some(tool_call_id.clone()),
                tool_calls: None,
            });
        }
    };

    match execute_fn(args).await {
        Ok(output) => {
            println!("{output}");
            let uncolored_output = strip_str(&output);
            Ok(Message {
                role: "tool".to_string(),
                content: uncolored_output,
                name: Some(function_name.to_string()),
                tool_call_id: Some(tool_call_id.clone()),
                tool_calls: None,
            })
        }
        Err(e) => {
            let error_msg = format!("Error {error_context}: {e}");
            println!("{}", style(error_msg.clone()).red());
            Ok(Message {
                role: "tool".to_string(),
                content: error_msg,
                name: Some(function_name.to_string()),
                tool_call_id: Some(tool_call_id.clone()),
                tool_calls: None,
            })
        }
    }
}

pub async fn handle_tool_call(
    tool_call: &ToolCall,
    config: &Config,
    file_state_manager: Arc<Mutex<file_state_manager::FileStateManager>>,
) -> Result<Message> {
    let function_name = &tool_call.function_call.name;
    let tool_call_id = tool_call.id.clone();
    let arguments = &tool_call.function_call.arguments;

    match function_name.as_str() {
        "execute_shell_command" => {
            let config_clone = config.clone();
            execute_tool(
                arguments,
                function_name,
                tool_call_id,
                "executing command",
                |args: shell::ShellCommandArgs| async move {
                    shell::execute_shell_command(
                        &args.command,
                        &config_clone.allowed_command_prefixes,
                    )
                    .await
                },
            )
            .await
        }
        "create_file" => {
            let manager_clone = Arc::clone(&file_state_manager);
            let accessible_paths = config.accessible_paths.clone();
            execute_tool(
                arguments,
                function_name,
                tool_call_id,
                "creating file",
                |args: file_creator::CreateFileArgs| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut manager = manager_clone.lock().unwrap();
                        file_creator::execute_create_files(&args, &mut manager, &accessible_paths)
                    })
                    .await?
                },
            )
            .await
        }
        "edit_file" => {
            let manager_clone = Arc::clone(&file_state_manager);
            let accessible_paths = config.accessible_paths.clone();
            execute_tool(
                arguments,
                function_name,
                tool_call_id,
                "editing file",
                |args: file_editor::TopLevelRequest| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut manager = manager_clone.lock().unwrap();
                        file_editor::execute_file_operations(&args, &mut manager, &accessible_paths)
                    })
                    .await?
                },
            )
            .await
        }
        "read_file" => {
            let manager_clone = Arc::clone(&file_state_manager);
            let config_clone = config.clone();
            execute_tool(
                arguments,
                function_name,
                tool_call_id,
                "reading file",
                |args: file_reader::FileReadArgs| async move {
                    tokio::task::spawn_blocking(move || {
                        let mut manager = manager_clone.lock().unwrap();
                        file_reader::execute_read_file(&args, &config_clone, &mut manager)
                    })
                    .await?
                },
            )
            .await
        }
        "list_files" => {
            let config_clone = config.clone();
            execute_tool(
                arguments,
                function_name,
                tool_call_id,
                "listing files",
                |args: list_files::ListFilesArgs| async move {
                    tokio::task::spawn_blocking(move || {
                        list_files::execute_list_files(&args, &config_clone)
                    })
                    .await?
                },
            )
            .await
        }
        _ => Ok(Message {
            role: "tool".to_string(),
            content: format!("Error: Unknown tool '{function_name}'"),
            name: Some(function_name.clone()),
            tool_call_id: Some(tool_call_id),
            tool_calls: None,
        }),
    }
}
