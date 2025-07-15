use crate::{
    config::Config,
    file_editor::{FileEditArgs, execute_file_edit},
    file_reader::{FileReadArgs, execute_read_file},
    list_files::{ListFilesArgs, execute_list_files},
    shell::{ShellCommandArgs, execute_shell_command},
};
use openrouter_api::types::chat::Message;

pub fn handle_tool_calls(response_message: &Message, config: &Config) -> Vec<Message> {
    let mut tool_messages = Vec::new();

    if let Some(tool_calls) = &response_message.tool_calls {
        for tool_call in tool_calls {
            let tool_call_id = Some(tool_call.id.clone());
            let function_name = &tool_call.function_call.name;

            let tool_message = match function_name.as_str() {
                "execute_shell_command" => {
                    match serde_json::from_str::<ShellCommandArgs>(
                        &tool_call.function_call.arguments,
                    ) {
                        Ok(args) => {
                            match execute_shell_command(
                                &args.command,
                                &config.allowed_command_prefixes,
                            ) {
                                Ok(output) => Message {
                                    role: "tool".to_string(),
                                    content: output,
                                    tool_call_id,
                                    name: None,
                                    tool_calls: None,
                                },
                                Err(e) => Message {
                                    role: "tool".to_string(),
                                    content: format!("Error executing command: {e}"),
                                    tool_call_id,
                                    name: None,
                                    tool_calls: None,
                                },
                            }
                        }
                        Err(e) => Message {
                            role: "tool".to_string(),
                            content: format!(
                                "Error: Invalid arguments provided for {function_name}: {e}"
                            ),
                            tool_call_id,
                            name: None,
                            tool_calls: None,
                        },
                    }
                }
                "edit_file" => {
                    match serde_json::from_str::<FileEditArgs>(&tool_call.function_call.arguments) {
                        Ok(args) => match execute_file_edit(&args, &config.editable_paths) {
                            Ok(output) => Message {
                                role: "tool".to_string(),
                                content: output,
                                tool_call_id,
                                name: None,
                                tool_calls: None,
                            },
                            Err(e) => Message {
                                role: "tool".to_string(),
                                content: format!("Error executing file edit: {e}"),
                                tool_call_id,
                                name: None,
                                tool_calls: None,
                            },
                        },
                        Err(e) => Message {
                            role: "tool".to_string(),
                            content: format!(
                                "Error: Invalid arguments provided for {function_name}: {e}"
                            ),
                            tool_call_id,
                            name: None,
                            tool_calls: None,
                        },
                    }
                }
                "read_file" => {
                    match serde_json::from_str::<FileReadArgs>(&tool_call.function_call.arguments) {
                        Ok(args) => match execute_read_file(&args, config) {
                            Ok(output) => Message {
                                role: "tool".to_string(),
                                content: output,
                                tool_call_id,
                                name: None,
                                tool_calls: None,
                            },
                            Err(e) => Message {
                                role: "tool".to_string(),
                                content: format!("Error reading file: {e}"),
                                tool_call_id,
                                name: None,
                                tool_calls: None,
                            },
                        },
                        Err(e) => Message {
                            role: "tool".to_string(),
                            content: format!(
                                "Error: Invalid arguments provided for {function_name}: {e}"
                            ),
                            tool_call_id,
                            name: None,
                            tool_calls: None,
                        },
                    }
                }
                "list_files" => {
                    match serde_json::from_str::<ListFilesArgs>(&tool_call.function_call.arguments)
                    {
                        Ok(args) => match execute_list_files(&args, config) {
                            Ok(output) => Message {
                                role: "tool".to_string(),
                                content: output,
                                tool_call_id,
                                name: None,
                                tool_calls: None,
                            },
                            Err(e) => Message {
                                role: "tool".to_string(),
                                content: format!("Error listing files: {e}"),
                                tool_call_id,
                                name: None,
                                tool_calls: None,
                            },
                        },
                        Err(e) => Message {
                            role: "tool".to_string(),
                            content: format!(
                                "Error: Invalid arguments provided for {function_name}: {e}"
                            ),
                            tool_call_id,
                            name: None,
                            tool_calls: None,
                        },
                    }
                }
                _ => Message {
                    role: "tool".to_string(),
                    content: format!("Error: Unknown tool '{function_name}'"),
                    tool_call_id,
                    name: None,
                    tool_calls: None,
                },
            };
            tool_messages.push(tool_message);
        }
    }

    tool_messages
}
