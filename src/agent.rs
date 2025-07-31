use crate::config::Config;
use crate::file_state_manager::FileStateManager;
use crate::prompt_builder;
use crate::streaming_executor;
use crate::tool_collection::ToolCollection;
use anyhow::Result;
use openrouter_api::models::tool::ToolCall;
use openrouter_api::types::chat::{ChatCompletionRequest, Message};
use openrouter_api::{OpenRouterClient, Ready};
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

#[derive(Debug)]
pub struct PromptData {
    pub final_prompt: String,
    pub file_summaries: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub enum AgentOutput {
    /// The agent has spawned an LLM task that needs to be awaited.
    PendingLLM(JoinHandle<Result<Option<Message>>>),
    /// The agent has produced a text response for the user.
    Message(Message),
    /// The agent wants to execute one or more tools and requires
    /// confirmation from the caller.
    ToolCalls(Vec<ToolCall>),
    /// The agent has nothing further to do at this time.
    Done,
}

pub struct Agent {
    pub client: Option<Arc<OpenRouterClient<Ready>>>,
    pub config: Config,
    pub messages: Vec<Message>,
    pub tool_collection: Arc<ToolCollection>,
    pub file_state_manager: Arc<Mutex<FileStateManager>>,
}

impl Agent {
    /// Creates a new `Agent` with its own state.
    pub fn new(
        config: Config,
        client: Option<OpenRouterClient<Ready>>,
        tool_collection: Arc<ToolCollection>,
    ) -> Self {
        Self {
            client: client.map(Arc::new),
            config,
            messages: Vec::new(),
            file_state_manager: Arc::new(Mutex::new(FileStateManager::new())),
            tool_collection,
        }
    }

    /// Processes a raw user prompt, expanding file paths and generating context.
    /// This method does NOT modify the agent's message history.
    pub fn prepare_prompt(&self, prompt: &str) -> Result<PromptData> {
        let mut fsm = self.file_state_manager.lock().unwrap();
        let prompt_data = prompt_builder::process_prompt(prompt, &self.config, &mut fsm)?;
        Ok(PromptData {
            final_prompt: prompt_data.final_prompt,
            file_summaries: prompt_data.file_summaries,
            warnings: prompt_data.warnings,
        })
    }

    /// Takes a user prompt, runs the LLM, and returns a handle to the streaming task.
    pub fn step_streaming(&mut self, prompt: String) -> Result<AgentOutput> {
        let request = self.prepare_request(prompt)?;

        if let Some(request) = request {
            let client = self.client.as_ref().unwrap().clone();
            let handle = tokio::spawn(async move {
                streaming_executor::stream_and_collect_response(&client, request).await
            });
            Ok(AgentOutput::PendingLLM(handle))
        } else {
            Ok(AgentOutput::Done)
        }
    }

    /// Takes a user prompt, runs the LLM, and returns the final `AgentOutput`.
    pub async fn step_non_streaming(&mut self, prompt: String) -> Result<AgentOutput> {
        let request = self.prepare_request(prompt)?;

        if let Some(request) = request {
            let client = self.client.as_ref().unwrap().clone();
            let response =
                streaming_executor::collect_response_non_streaming(&client, request).await?;
            if let Some(message) = response {
                self.messages.push(message.clone());
                if let Some(tool_calls) = message.tool_calls {
                    Ok(AgentOutput::ToolCalls(tool_calls))
                } else {
                    Ok(AgentOutput::Message(message))
                }
            } else {
                Ok(AgentOutput::Done)
            }
        } else {
            Ok(AgentOutput::Done)
        }
    }

    /// Executes a list of approved tool calls and returns the resulting
    /// messages.
    pub async fn execute_tool_calls(&mut self, tool_calls: Vec<ToolCall>) -> Result<Vec<Message>> {
        let mut result_messages = Vec::new();
        for tool_call in tool_calls {
            let result_msg = self
                .tool_collection
                .execute_tool_call(&tool_call, &self.config, self.file_state_manager.clone())
                .await;
            self.messages.push(result_msg.clone());
            result_messages.push(result_msg);
        }
        Ok(result_messages)
    }

    // --- Private Helper Functions ---

    fn prepare_request(&mut self, prompt: String) -> Result<Option<ChatCompletionRequest>> {
        if !prompt.is_empty() {
            let user_message = Message {
                role: "user".to_string(),
                content: prompt,
                name: None,
                tool_calls: None,
                tool_call_id: None,
            };
            self.messages.push(user_message);
        }

        if self.messages.is_empty() {
            return Ok(None);
        }

        let tools = self.tool_collection.get_all_schemas();
        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: self.messages.clone(),
            tools: Some(tools),
            stream: Some(true), // This will be overridden in the non-streaming case
            response_format: None,
            provider: None,
            models: None,
            transforms: None,
        };

        if self.config.print_messages {
            println!();
            println!(
                "{}",
                console::style("Messages being sent to API:")
                    .yellow()
                    .bold()
            );
            for message in &self.messages {
                let message_json = serde_json::to_string_pretty(message)
                    .unwrap_or_else(|e| format!("Failed to serialize message: {e}"));
                println!("{message_json}");
            }
            println!();
        }

        Ok(Some(request))
    }
}
