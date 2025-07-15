use anyhow::Result;
use clap::Parser;
use openrouter_api::{
    OpenRouterClient,
    types::chat::{ChatCompletionRequest, Message},
    utils,
};
use std::time::Duration;

mod cli;
mod config;
mod enricher;
mod path_expander;
mod prompt_builder;
mod shell;
mod tool_executor;
mod ui;
use crate::prompt_builder::build_user_prompt;
use crate::shell::shell_tool_schema;
use crate::ui::pretty_print_message;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let config = config::load_or_create()?;

    let original_prompt = match cli.command {
        cli::Commands::Agent { prompt } => prompt,
    };

    let final_prompt = build_user_prompt(&original_prompt).await?;

    let api_key = utils::load_api_key_from_env().expect("OPENROUTER_API_KEY not set");
    let or_client = OpenRouterClient::new()
        .with_base_url("https://openrouter.ai/api/v1/")?
        .with_timeout(Duration::from_secs(config.timeout_seconds))
        .with_api_key(api_key)?;

    let mut messages: Vec<Message> = vec![
        Message {
            role: "system".to_string(),
            content: config.system_prompt.clone(),
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
        Message {
            role: "user".to_string(),
            content: final_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    let tools = vec![shell_tool_schema()];

    for message in &messages {
        pretty_print_message(message);
    }

    for _i in 0..config.max_iterations {
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

            if response_message.tool_calls.is_some() {
                let tool_messages = tool_executor::handle_tool_calls(&response_message, &config);
                for tool_message in tool_messages {
                    pretty_print_message(&tool_message);
                    messages.push(tool_message);
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
