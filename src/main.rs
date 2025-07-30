use anyhow::Result;
use clap::Parser;
use console::style;
use openrouter_api::{OpenRouterClient, types::chat::Message};
use std::sync::Arc;
use std::time::Duration;

use crate::tool_manager::ToolManager;

mod backend;
mod cli;
mod config;
mod diff;
mod enricher;
mod file_creator;
mod file_editor;
mod file_reader;
mod file_state;
mod file_state_manager;
mod list_files;
mod patch;
mod path_expander;
mod permissions;
mod prompt_builder;
mod shell;
mod streaming_executor;
mod tool_manager;
mod tools;
mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let config = config::load(&cli.overrides)?;

    let api_key = if let Some(env_var) = config.backend.api_key_env_var() {
        std::env::var(env_var)?
    } else {
        // TODO: only call .with_api_key if let Some(config.backend.api_key_env_var())
        "sk-or-v1-0000000000000000000000000000000000000000000000000000000000000000".to_string()
    };

    let client = OpenRouterClient::new()
        .with_base_url(&config.base_url)?
        .with_timeout(Duration::from_secs(config.timeout_seconds))
        .with_api_key(api_key)?;
    // Always print backend
    println!("Backend: {:?}", config.backend);

    let mut tool_manager = ToolManager::new();
    // Register tools
    tool_manager.register(Box::new(tools::FileCreatorTool));
    tool_manager.register(Box::new(tools::FileEditorTool));
    tool_manager.register(Box::new(tools::FileReaderTool));
    tool_manager.register(Box::new(tools::ListFilesTool));
    tool_manager.register(Box::new(tools::ShellTool));
    // Collect tool names
    let schemas = tool_manager.get_all_schemas();
    let tool_names: Vec<String> = schemas
        .iter()
        .map(|api_tool| match api_tool {
            openrouter_api::models::tool::Tool::Function { function } => function.name.clone(),
        })
        .collect();
    // If no initial user message, print tools and model
    if cli.prompt.clone().unwrap_or_default().is_empty() {
        println!("tools: {}", tool_names.join(", "));
        println!("model: {}", config.model);
    }
    let tool_manager = Arc::new(tool_manager);

    let mut app = ui::App::new(config.clone(), client, tool_manager);

    // Only process system prompt if one is configured
    if let Some(system_prompt) = &app.config.system_prompt {
        let prompt_data = {
            let mut fsm = app.file_state_manager.lock().unwrap();
            prompt_builder::process_prompt(system_prompt, &app.config, &mut fsm)?
        };

        if app.config.show_system_prompt {
            println!("[{}]", style("system").blue());
            println!("{system_prompt}"); // Print the original, un-expanded prompt

            if !prompt_data.file_summaries.is_empty() {
                println!("{}", style("Attached files:").dim());
                for summary in prompt_data.file_summaries {
                    println!("{}", style(summary).dim());
                }
            }

            for warning in prompt_data.warnings {
                eprintln!("{}", style(warning).yellow());
            }
        }

        let system_message = Message {
            role: "system".to_string(),
            content: prompt_data.final_prompt,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        app.messages.push(system_message);
    }
    app.run(&cli.prompt.unwrap_or_default()).await?;

    Ok(())
}
