use crate::{
    config::Config,
    file_editor::execute_file_patch,
    file_reader::{FileReadArgs, execute_read_file},
    file_state::{FileStateManager, PatchArgs},
    list_files::{ListFilesArgs, execute_list_files},
    shell::{ShellCommandArgs, execute_shell_command},
};
use colored::Colorize;
use openrouter_api::types::chat::Message;
use std::io::{self, Write};
use strip_ansi_escapes::strip_str;

fn wait_for_enter() {
    print!("\n{}", "Press Enter to continue...".dimmed());
    io::stdout().flush().unwrap();
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer).unwrap();
    print!("\r");
}

pub fn handle_tool_calls(
    response_message: &Message,
    config: &Config,
    file_state_manager: &mut FileStateManager,
) -> Vec<Message> {
    let mut tool_messages = Vec::new();

    if let Some(tool_calls) = &response_message.tool_calls {
        for tool_call in tool_calls {
            let tool_call_id = Some(tool_call.id.clone());
            let function_name = &tool_call.function_call.name;
            println!("\n[{}]", format!("tool: {function_name}").purple());
            println!("{}", tool_call.function_call.arguments);

            wait_for_enter();

            let tool_message = match function_name.as_str() {
                "execute_shell_command" => {
                    let result = match serde_json::from_str::<ShellCommandArgs>(
                        &tool_call.function_call.arguments,
                    ) {
                        Ok(args) => {
                            execute_shell_command(&args.command, &config.allowed_command_prefixes)
                        }
                        Err(e) => Err(anyhow::anyhow!(
                            "Error: Invalid arguments provided for {function_name}: {e}"
                        )),
                    };

                    let (colored_output, uncolored_output) = match result {
                        Ok(output) => (output.clone(), strip_str(&output)),
                        Err(e) => {
                            let error_msg = format!("Error executing command: {e}");
                            (error_msg.red().to_string(), error_msg)
                        }
                    };

                    println!("{colored_output}");
                    Message {
                        role: "tool".to_string(),
                        content: uncolored_output,
                        tool_call_id,
                        name: None,
                        tool_calls: None,
                    }
                }
                "edit_file" => {
                    let result =
                        match serde_json::from_str::<PatchArgs>(&tool_call.function_call.arguments)
                        {
                            Ok(args) => execute_file_patch(
                                &args,
                                file_state_manager,
                                &config.editable_paths,
                            ),
                            Err(e) => Err(anyhow::anyhow!(
                                "Error: Invalid arguments provided for {function_name}: {e}"
                            )),
                        };

                    let (colored_output, uncolored_output) = match result {
                        Ok(output) => (output.clone(), strip_str(&output)),
                        Err(e) => {
                            let error_msg = format!("Error executing file patch: {e}");
                            (error_msg.red().to_string(), error_msg)
                        }
                    };

                    println!("{colored_output}");
                    Message {
                        role: "tool".to_string(),
                        content: uncolored_output,
                        tool_call_id,
                        name: None,
                        tool_calls: None,
                    }
                }
                "read_file" => {
                    let result = match serde_json::from_str::<FileReadArgs>(
                        &tool_call.function_call.arguments,
                    ) {
                        Ok(args) => execute_read_file(&args, config, file_state_manager),
                        Err(e) => Err(anyhow::anyhow!(
                            "Error: Invalid arguments provided for {function_name}: {e}"
                        )),
                    };

                    let (colored_output, uncolored_output) = match result {
                        Ok(output) => (output.clone(), strip_str(&output)),
                        Err(e) => {
                            let error_msg = format!("Error reading file: {e}");
                            (error_msg.red().to_string(), error_msg)
                        }
                    };

                    println!("{colored_output}");
                    Message {
                        role: "tool".to_string(),
                        content: uncolored_output,
                        tool_call_id,
                        name: None,
                        tool_calls: None,
                    }
                }
                "list_files" => {
                    let result = match serde_json::from_str::<ListFilesArgs>(
                        &tool_call.function_call.arguments,
                    ) {
                        Ok(args) => execute_list_files(&args, config),
                        Err(e) => Err(anyhow::anyhow!(
                            "Error: Invalid arguments provided for {function_name}: {e}"
                        )),
                    };

                    let (colored_output, uncolored_output) = match result {
                        Ok(output) => (output.clone(), strip_str(&output)),
                        Err(e) => {
                            let error_msg = format!("Error listing files: {e}");
                            (error_msg.red().to_string(), error_msg)
                        }
                    };

                    println!("{colored_output}");
                    Message {
                        role: "tool".to_string(),
                        content: uncolored_output,
                        tool_call_id,
                        name: None,
                        tool_calls: None,
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
