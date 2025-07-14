use anyhow::Result;
use openrouter_api::{
    OpenRouterClient,
    types::chat::{ChatCompletionRequest, Message},
    utils,
};
use std::time::Duration;

mod shell;
mod ui;
use crate::shell::{ShellCommandArgs, execute_shell_command, shell_tool_schema};
use crate::ui::pretty_print_message;

// --- Constants ---
const MODEL_NAME: &str = "google/gemini-2.5-flash-preview";

#[tokio::main]
async fn main() -> Result<()> {
    // 1. Initialize LLM Client
    let api_key = utils::load_api_key_from_env().expect("OPENROUTER_API_KEY not set");
    let or_client = OpenRouterClient::new()
        .with_base_url("https://openrouter.ai/api/v1/")?
        .with_timeout(Duration::from_secs(120))
        .with_api_key(api_key)?;

    // 2. Prepare the Conversation
    let mut messages: Vec<Message> = vec![
        Message {
            role: "system".to_string(),
            content: "You are a helpful assistant that can execute shell commands.
When asked to perform a task, use the available `execute_shell_command` tool.
When you have the final answer, provide it directly without using a tool."
                .to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        Message {
            role: "user".to_string(),
            content:
                "List the files in the current directory, then show me the contents of Cargo.toml"
                    .to_string(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let tools = vec![shell_tool_schema()];

    // 3. Start the Interaction Loop
    for message in &messages {
        pretty_print_message(message);
    }

    for _i in 0..5 {
        // Limit to 5 iterations to prevent runaway execution
        let request = ChatCompletionRequest {
            model: MODEL_NAME.to_string(),
            messages: messages.clone(),
            tools: Some(tools.clone()),
            stream: None,
            response_format: None,
            provider: None,
            models: None,
            transforms: None,
        };

        let response = or_client.chat()?.chat_completion(request).await?;

        if let Some(choice) = response.choices.first() {
            let response_message = choice.message.clone();
            pretty_print_message(&response_message);
            messages.push(response_message.clone());

            // Check if the LLM wants to call a tool
            if let Some(tool_calls) = response_message.tool_calls {
                for tool_call in tool_calls {
                    if tool_call.function_call.name == "execute_shell_command" {
                        // Parse the arguments the LLM provided
                        let args: ShellCommandArgs =
                            serde_json::from_str(&tool_call.function_call.arguments)?;

                        // Execute the command and get the result
                        let tool_result = execute_shell_command(&args.command)?;

                        // Add the tool's response to the message history
                        let tool_message = Message {
                            role: "tool".to_string(),
                            content: tool_result,
                            tool_call_id: Some(tool_call.id),
                            name: None,
                            tool_calls: None,
                        };
                        pretty_print_message(&tool_message);
                        messages.push(tool_message);
                    }
                }
            } else {
                // If no tool call, the LLM is giving its final answer
                break;
            }
        } else {
            println!("Error: No response from LLM.");
            break;
        }
    }

    Ok(())
}
