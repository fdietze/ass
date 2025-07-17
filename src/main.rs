use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use openrouter_api::{
    OpenRouterClient,
    types::chat::{ChatCompletionRequest, Message},
    utils,
};
use std::io::{self, Write};
use std::time::Duration;

mod cli;
mod config;
mod enricher;
mod file_editor;
mod file_reader;
mod file_state;
mod list_files;
mod path_expander;
mod prompt_builder;
mod shell;
mod streaming_executor;
mod tool_executor;
mod ui;

use crate::file_state::FileStateManager;
use crate::prompt_builder::expand_file_mentions;
use crate::shell::shell_tool_schema;
use crate::ui::pretty_print_message;

fn wait_for_enter() -> Result<()> {
    let prompt = "\nPress Enter to continue...".dimmed().to_string();
    print!("{prompt}");
    io::stdout().flush()?;
    let mut buffer = String::new();
    io::stdin().read_line(&mut buffer)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let config = config::load_or_create()?;
    let mut file_state_manager = FileStateManager::new();

    let system_prompt_content =
        expand_file_mentions(&config.system_prompt, &config, &mut file_state_manager).await?;
    let mut next_prompt = cli.prompt;

    let api_key = utils::load_api_key_from_env().expect("OPENROUTER_API_KEY not set");
    let or_client = OpenRouterClient::new()
        .with_base_url("https://openrouter.ai/api/v1/")?
        .with_timeout(Duration::from_secs(config.timeout_seconds))
        .with_api_key(api_key)?;

    let mut messages: Vec<Message> = vec![Message {
        role: "system".to_string(),
        content: system_prompt_content,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    }];

    let tools = vec![
        shell_tool_schema(),
        file_editor::edit_file_tool_schema(),
        file_reader::read_file_tool_schema(),
        list_files::list_files_tool_schema(),
    ];

    println!("Model: {}", config.model);

    'main_loop: loop {
        let final_prompt =
            expand_file_mentions(&next_prompt, &config, &mut file_state_manager).await?;

        let user_message = Message {
            role: "user".to_string(),
            content: final_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };

        print!("{}", pretty_print_message(&user_message));
        messages.push(user_message);

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

            let response_message =
                streaming_executor::stream_and_collect_response(&or_client, request).await?;

            messages.push(response_message.clone());

            if let Some(tool_calls) = &response_message.tool_calls {
                for tool_call in tool_calls {
                    let function_name = &tool_call.function_call.name;
                    println!("\n[{}]", format!("tool: {function_name}").purple());
                    println!("{}", tool_call.function_call.arguments);

                    match wait_for_enter() {
                        Ok(()) => {}
                        Err(e) => {
                            println!("Error: {e:?}");
                            break 'main_loop;
                        }
                    }

                    let tool_message = tool_executor::handle_tool_call(
                        tool_call,
                        &config,
                        &mut file_state_manager,
                    );
                    messages.push(tool_message);
                }
            } else {
                // If no tool call, the LLM is giving its final answer for this turn
                break;
            }
        }

        print!("user> ");
        io::stdout().flush()?;
        let mut buffer = String::new();
        match io::stdin().read_line(&mut buffer) {
            Ok(0) => {
                println!("\nCTRL-D");
                break;
            }
            Ok(_) => {
                next_prompt = buffer.trim().to_string();
            }
            Err(error) => {
                println!("error: {error}");
                break;
            }
        }
    }

    println!("{messages:#?}");

    Ok(())
}
