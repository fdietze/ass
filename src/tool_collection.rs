//! # Tool Collection
//!
//! The `ToolCollection` is the central hub for discovering, previewing, and executing tools.
//! It maintains a registry of all available tools and dispatches calls to the appropriate
//! implementation based on the tool name.

use crate::{config::Config, file_state_manager::FileStateManager, tools::Tool};
use anyhow::{Result, anyhow};
use console::style;
use openrouter_api::{
    models::tool::{Tool as ApiTool, ToolCall},
    types::chat::Message,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use strip_ansi_escapes::strip_str;

/// A collection responsible for registering and dispatching tool calls.
pub struct ToolCollection {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolCollection {
    /// Creates a new, empty `ToolCollection`.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Registers a new tool with the collection.
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Gathers the schemas of all registered tools to be sent to the LLM.
    pub fn get_all_schemas(&self) -> Vec<ApiTool> {
        self.tools
            .values()
            .map(|tool| ApiTool::Function {
                function: tool.schema(),
            })
            .collect()
    }

    /// Generates a preview for a tool call.
    pub async fn preview_tool_call(
        &self,
        tool_call: &ToolCall,
        config: &Config,
        fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String> {
        let function_name = &tool_call.function_call.name;
        let arguments = &tool_call.function_call.arguments;
        let args_value: Value = serde_json::from_str(arguments)?;

        if config.debug_tool_calls {
            let pretty_args = serde_json::to_string_pretty(&args_value)?;
            println!("{}", style(pretty_args).dim());
        }

        let tool = self
            .tools
            .get(function_name)
            .ok_or_else(|| anyhow!("Unknown tool: {function_name}"))?;

        tool.preview(&args_value, config, fsm)
    }

    /// Checks if a tool call is safe for automatic execution.
    pub fn is_safe_for_auto_execute(&self, tool_call: &ToolCall, config: &Config) -> Result<bool> {
        let function_name = &tool_call.function_call.name;
        let arguments = &tool_call.function_call.arguments;
        let args_value: Value = serde_json::from_str(arguments)?;

        let tool = self
            .tools
            .get(function_name)
            .ok_or_else(|| anyhow!("Unknown tool: {function_name}"))?;

        tool.is_safe_for_auto_execute(&args_value, config)
    }

    /// Executes a tool call and returns the result as a `Message`.
    /// This function is designed to always succeed from the caller's perspective,
    /// returning a `Message`. Any failures in tool lookup, argument parsing,
    /// or execution are captured and returned within the `content` of the
    /// `tool` role message.
    pub async fn execute_tool_call(
        &self,
        tool_call: &ToolCall,
        config: &Config,
        fsm: Arc<Mutex<FileStateManager>>,
    ) -> Message {
        let function_name = &tool_call.function_call.name;
        let arguments = &tool_call.function_call.arguments;

        let result = async {
            let tool = self
                .tools
                .get(function_name)
                .ok_or_else(|| anyhow!("Unknown tool: {function_name}"))?;

            let args_value: Value = serde_json::from_str(arguments)
                .map_err(|e| anyhow!("Failed to parse JSON arguments: {e}"))?;

            tool.execute(&args_value, config, fsm).await
        }
        .await;

        let message_content = match result {
            Ok(output) => {
                if config.debug_tool_calls {
                    println!(
                        "{}",
                        style(format!("Tool output:\n{}", output.clone())).dim()
                    );
                }

                strip_str(&output)
            }
            Err(e) => {
                let error_message = e.to_string();
                eprintln!(
                    "{}",
                    style(format!(
                        "Error executing tool `{function_name}`: {error_message}"
                    ))
                    .red()
                );
                error_message
            }
        };

        Message {
            role: "tool".to_string(),
            content: message_content,
            name: Some(function_name.to_string()),
            tool_call_id: Some(tool_call.id.clone()),
            tool_calls: None,
        }
    }
}

impl Default for ToolCollection {
    fn default() -> Self {
        Self::new()
    }
}
