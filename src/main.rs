use anyhow::Result;
use clap::Parser;
use console::style;
use openrouter_api::types::chat::Message;
use std::sync::Arc;

use alors::{agent::Agent, tool_collection::ToolCollection};

mod cli;
mod ui;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::Cli::parse();
    let config = alors::config::load(&cli.overrides)?;

    let client = alors::client::initialize_client(&config)?;
    // Always print backend
    println!("Backend: {:?}", config.backend);

    let mut tool_collection = ToolCollection::new();
    // Register tools
    tool_collection.register(Box::new(alors::tools::FileCreatorTool));
    tool_collection.register(Box::new(alors::tools::FileEditorTool));
    tool_collection.register(Box::new(alors::tools::FileReaderTool));
    tool_collection.register(Box::new(alors::tools::ListFilesTool));
    tool_collection.register(Box::new(alors::tools::ShellTool));
    // Collect tool names
    let schemas = tool_collection.get_all_schemas();
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
    let tool_collection = Arc::new(tool_collection);

    let mut agent = Agent::new(config.clone(), Some(client), tool_collection);

    // Only process system prompt if one is configured
    if let Some(system_prompt) = &agent.config.system_prompt {
        let prompt_data = {
            let mut fsm = agent.file_state_manager.lock().unwrap();
            alors::prompt_builder::process_prompt(system_prompt, &agent.config, &mut fsm)?
        };

        if agent.config.show_system_prompt {
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
        agent.messages.push(system_message);
    }

    let mut app = ui::App::new(agent);
    app.run(&cli.prompt.unwrap_or_default()).await?;

    Ok(())
}
