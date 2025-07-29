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

    println!("Backend: {:?}", config.backend);
    println!("Model: {}", config.model);

    let mut tool_manager = ToolManager::new();
    tool_manager.register(Box::new(tools::FileCreatorTool));
    tool_manager.register(Box::new(tools::FileEditorTool));
    tool_manager.register(Box::new(tools::FileReaderTool));
    tool_manager.register(Box::new(tools::ListFilesTool));
    tool_manager.register(Box::new(tools::ShellTool));
    let tool_manager = Arc::new(tool_manager);

    let mut app = ui::App::new(config.clone(), client, tool_manager);

    // Only process system prompt if one is configured
    if let Some(system_prompt) = &app.config.system_prompt {
        let system_prompt_content = {
            let mut fsm = app.file_state_manager.lock().unwrap();
            prompt_builder::expand_file_mentions(system_prompt, &app.config, &mut fsm)?
        };

        if app.config.show_system_prompt {
            println!("[{}]", style("system").blue());
            println!("{system_prompt}"); // Print the original, un-expanded prompt

            // Display collapsed summary for files mentioned in the system prompt
            let enrichments = enricher::extract_enrichments(system_prompt);
            if !enrichments.mentioned_files.is_empty() {
                let expansion_result = path_expander::expand_and_validate(
                    &enrichments.mentioned_files,
                    &app.config.ignored_paths,
                );

                let summaries: Vec<String> = expansion_result
                    .files
                    .iter()
                    .filter_map(|file_path| {
                        let mut fsm = app.file_state_manager.lock().unwrap();
                        match fsm.open_file(file_path) {
                            Ok(file_state) => {
                                let total_lines = file_state.lines.len();
                                let filename = std::path::Path::new(file_path)
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy();
                                Some(format!("[{filename} ({total_lines} lines)]"))
                            }
                            Err(_) => None,
                        }
                    })
                    .collect();

                if !summaries.is_empty() {
                    println!("{}", style("Attached files:").dim());
                    for summary in summaries {
                        println!("{}", style(summary).dim());
                    }
                }
            }
        }

        let system_message = Message {
            role: "system".to_string(),
            content: system_prompt_content,
            name: None,
            tool_calls: None,
            tool_call_id: None,
        };
        app.messages.push(system_message);
    }
    app.run(&cli.prompt.unwrap_or_default()).await?;

    Ok(())
}
