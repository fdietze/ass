use anyhow::Result;
use clap::Parser;
use openrouter_api::{
    OpenRouterClient,
    types::chat::{ChatCompletionRequest, Message},
    utils,
};
use std::time::Duration;

mod config;
mod shell;
mod ui;
use crate::shell::{ShellCommandArgs, execute_shell_command, shell_tool_schema};
use crate::ui::pretty_print_message;

/// A simple command-line agent
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Parser, Debug)]
enum Commands {
    /// Run the agent with a prompt
    Agent {
        /// The prompt for the agent
        prompt: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let prompt = match cli.command {
        Commands::Agent { prompt } => prompt,
    };

    // 0. Load Configuration
    let config = config::load_or_create()?;

    // 1. Initialize LLM Client
    let api_key = utils::load_api_key_from_env().expect("OPENROUTER_API_KEY not set");
    let or_client = OpenRouterClient::new()
        .with_base_url("https://openrouter.ai/api/v1/")?
        .with_timeout(Duration::from_secs(config.timeout_seconds))
        .with_api_key(api_key)?;

    // 2. Prepare the Conversation
    let mut messages: Vec<Message> = vec![
        Message {
            role: "system".to_string(),
            content: config.system_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        Message {
            role: "user".to_string(),
            content: prompt,
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

    for _i in 0..config.max_iterations {
        // Limit to 5 iterations to prevent runaway execution
        let request = ChatCompletionRequest {
            model: config.model.clone(),
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
