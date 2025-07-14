use crate::shell::{ShellCommandArgs, execute_shell_command};
use openrouter_api::types::chat::Message;

pub fn handle_tool_calls(response_message: &Message) -> Vec<Message> {
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
                        Ok(args) => match execute_shell_command(&args.command) {
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
