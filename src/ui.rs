use crate::config::Config;
use crate::file_state_manager::FileStateManager;
use crate::prompt_builder;

use crate::streaming_executor;
use crate::tool_manager::ToolManager;
use anyhow::Result;
use console::style;
use openrouter_api::models::tool::ToolCall;
use openrouter_api::types::chat::{ChatCompletionRequest, Message};
use openrouter_api::{OpenRouterClient, Ready};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

// The LIF_HEADER_REGEX is no longer needed for display and will be removed.

#[derive(Debug)]
pub enum AppState {
    Initializing,
    WaitingForUserInput,
    ProcessingPrompt(String),
    WaitingForLLM(JoinHandle<anyhow::Result<Option<Message>>>),
    AwaitingToolConfirmation {
        llm_response: Message,
        tool_calls_queue: VecDeque<ToolCall>,
        completed_messages: Vec<Message>,
    },
    Shutdown,
}

pub struct App {
    pub config: Config,
    pub client: Arc<OpenRouterClient<Ready>>,
    pub messages: Vec<Message>,
    pub file_state_manager: Arc<Mutex<FileStateManager>>,
    pub tool_manager: Arc<ToolManager>,
    pub state: AppState,
    stdin_receiver: mpsc::Receiver<Option<String>>,
}

impl App {
    pub fn new(
        config: Config,
        client: OpenRouterClient<Ready>,
        tool_manager: Arc<ToolManager>,
    ) -> Self {
        let _file_state_manager = FileStateManager::new();
        Self {
            config,
            client: Arc::new(client),
            messages: Vec::new(),
            file_state_manager: Arc::new(Mutex::new(FileStateManager::new())),
            tool_manager,
            state: AppState::Initializing,
            stdin_receiver: spawn_stdin_channel(),
        }
    }

    pub async fn run(&mut self, initial_prompt: &str) -> Result<()> {
        if !initial_prompt.is_empty() {
            self.state = AppState::ProcessingPrompt(initial_prompt.to_string());
        } else {
            self.state = AppState::WaitingForUserInput;
        }

        let mut ctrl_c_pressed = false;

        loop {
            match &mut self.state {
                AppState::WaitingForUserInput => {
                    print!("\x07{} ", style("user>").cyan().bold());
                    io::stdout().flush()?;

                    tokio::select! {
                        biased;
                        _ = tokio::signal::ctrl_c() => {
                            if ctrl_c_pressed {
                                self.state = AppState::Shutdown;
                            } else {
                                println!("\nPress Ctrl+C again to exit.");
                                ctrl_c_pressed = true;
                            }
                        }
                        line_opt = self.stdin_receiver.recv() => {
                            // recv() returns None if channel is closed.
                            let line_opt = line_opt.unwrap_or(None);
                            match line_opt {
                                Some(input) => {
                                    if input.is_empty() {
                                        continue;
                                    }
                                    self.state = AppState::ProcessingPrompt(input);
                                    ctrl_c_pressed = false;
                                }
                                None => {
                                    // Ctrl+D was pressed
                                    println!();
                                    self.state = AppState::Shutdown;
                                }
                            }
                        }
                    }
                }
                AppState::AwaitingToolConfirmation { .. } => {
                    if let AppState::AwaitingToolConfirmation {
                        llm_response,
                        tool_calls_queue,
                        completed_messages,
                    } = &mut self.state
                    {
                        // If the queue is empty, we're done.
                        if tool_calls_queue.is_empty() {
                            // Add original LLM message if we actually did something.
                            if !completed_messages.is_empty() {
                                self.messages.push(llm_response.clone());
                            }
                            self.messages.append(completed_messages);
                            self.state = self.spawn_llm_call();
                            continue; // Go to next loop iteration for new state.
                        }

                        // Peek at the next tool. Don't remove it yet.
                        let tool_call = tool_calls_queue.front().unwrap().clone();

                        // Display preview.
                        println!(
                            "[{}]",
                            style(format!("tool: {}", tool_call.function_call.name)).magenta()
                        );
                        match self
                            .tool_manager
                            .preview_tool_call(
                                &tool_call,
                                &self.config,
                                self.file_state_manager.clone(),
                            )
                            .await
                        {
                            Ok(preview) => {
                                println!("{preview}");
                                // Good. Proceed to confirmation.
                            }
                            Err(e) => {
                                // Preview failed. Auto-skip this tool.
                                let error_message = format!("Preview failed, skipping: {e}");
                                eprintln!("{}", style(&error_message).red());
                                completed_messages.push(Message {
                                    role: "tool".to_string(),
                                    content: error_message,
                                    name: Some(tool_call.function_call.name.clone()),
                                    tool_call_id: Some(tool_call.id.clone()),
                                    tool_calls: None,
                                });
                                tool_calls_queue.pop_front();
                                // Loop to process the next tool in the queue.
                                continue;
                            }
                        };

                        let mut should_auto_execute = self.config.auto_execute;
                        match self
                            .tool_manager
                            .is_safe_for_auto_execute(&tool_call, &self.config)
                        {
                            Ok(is_safe) => {
                                if !is_safe {
                                    should_auto_execute = false;
                                }
                            }
                            Err(e) => {
                                let error_message = format!(
                                    "Security check failed for tool call, please confirm manually: {e}"
                                );
                                eprintln!("{}", style(&error_message).red());
                                should_auto_execute = false;
                            }
                        }

                        if should_auto_execute {
                            // Automatically execute the tool without confirmation.
                            let tool_to_execute = tool_calls_queue.pop_front().unwrap();
                            let result_msg = self
                                .tool_manager
                                .execute_tool_call(
                                    &tool_to_execute,
                                    &self.config,
                                    self.file_state_manager.clone(),
                                )
                                .await;
                            completed_messages.push(result_msg);

                            // Continue the loop to process the next tool call or move to the next state.
                            continue;
                        }

                        // Prompt for confirmation.
                        print!("\x07{} ", style("Execute this tool? [Y/n] ").dim());
                        io::stdout().flush()?;

                        // Asynchronously wait for input.
                        tokio::select! {
                            _ = tokio::signal::ctrl_c() => {
                                println!("\n{}", style("Operation cancelled. Returning to input.").yellow());
                                self.state = AppState::WaitingForUserInput;
                            }
                            line_opt = self.stdin_receiver.recv() => {
                                let input = line_opt.flatten().unwrap_or_default();
                                if input.eq_ignore_ascii_case("n") {
                                    // Abort All
                                    println!("{}", style("Operation cancelled. Returning to input.").yellow());
                                    self.state = AppState::WaitingForUserInput;
                                } else {
                                    // Confirmed. Execute the tool.
                                    let tool_to_execute = tool_calls_queue.pop_front().unwrap();
                                    let result_msg = self.tool_manager.execute_tool_call(&tool_to_execute, &self.config, self.file_state_manager.clone()).await;
                                    completed_messages.push(result_msg);
                                    // The state has been mutated (queue popped, results added),
                                    // so we just continue the main loop to process the next item.
                                }
                            }
                        };
                    }
                }
                AppState::WaitingForLLM(_) => {
                    let (new_state, should_break) = tokio::select! {
                        _ = tokio::signal::ctrl_c() => {
                            if let AppState::WaitingForLLM(handle) = &mut self.state {
                                println!("\n{}", style("LLM generation cancelled.").yellow());
                                handle.abort();
                            }
                            (AppState::WaitingForUserInput, false)
                        }
                        result = async {
                            if let AppState::WaitingForLLM(handle) = &mut self.state {
                                handle.await
                            } else {
                                unreachable!()
                            }
                        } => {
                             match result {
                                Ok(Ok(Some(response_message))) => {
                                     if let Some(tool_calls) = response_message.tool_calls.clone() {
                                        (
                                            AppState::AwaitingToolConfirmation {
                                                llm_response: response_message,
                                                tool_calls_queue: tool_calls.into(),
                                                completed_messages: Vec::new(),
                                            },
                                            false,
                                        )
                                     } else {
                                         self.messages.push(response_message);
                                         (AppState::WaitingForUserInput, false)
                                     }
                                }
                                Ok(Ok(None)) => {
                                    (AppState::WaitingForUserInput, false)
                                }
                                Ok(Err(e)) => {
                                    eprintln!("{}", style(format!("LLM request failed: {e}")).red());
                                    (AppState::WaitingForUserInput, false)
                                }
                                Err(e) => {
                                     eprintln!("{}", style(format!("LLM task failed: {e}")).red());
                                    (AppState::WaitingForUserInput, false)
                                }
                            }
                        }
                    };
                    self.state = new_state;
                    if should_break {
                        break;
                    }
                }
                AppState::Shutdown => {
                    println!("\nShutting down...");
                    std::process::exit(0);
                }
                AppState::Initializing => {
                    // This state is now handled before the loop starts
                    unreachable!();
                }
                AppState::ProcessingPrompt(prompt) => {
                    let prompt_data = {
                        let mut fsm = self.file_state_manager.lock().unwrap();
                        prompt_builder::process_prompt(prompt, &self.config, &mut fsm)?
                    };

                    display_user_message(
                        prompt,
                        &prompt_data.file_summaries,
                        &prompt_data.warnings,
                    )
                    .await?;

                    // Print enabled tools and model after user message
                    let tool_names: Vec<String> = self
                        .tool_manager
                        .get_all_schemas()
                        .iter()
                        .map(|api_tool| match api_tool {
                            openrouter_api::models::tool::Tool::Function { function } => {
                                function.name.clone()
                            }
                        })
                        .collect();
                    println!("tools: {}", tool_names.join(", "));
                    println!("model: {}", self.config.model);

                    let user_message = Message {
                        role: "user".to_string(),
                        content: prompt_data.final_prompt,
                        name: None,
                        tool_calls: None,
                        tool_call_id: None,
                    };
                    self.messages.push(user_message);
                    let new_state = self.spawn_llm_call();
                    self.state = new_state;
                }
            }
        }
        Ok(())
    }

    fn spawn_llm_call(&self) -> AppState {
        let tools = self.tool_manager.get_all_schemas();

        let request = ChatCompletionRequest {
            model: self.config.model.clone(),
            messages: self.messages.clone(),
            tools: Some(tools),
            stream: Some(true),
            response_format: None,
            provider: None,
            models: None,
            transforms: None,
        };

        // Print messages before sending to API if configured
        if self.config.print_messages {
            println!();
            println!("{}", style("Messages being sent to API:").yellow().bold());
            for message in &self.messages {
                let message_json = serde_json::to_string_pretty(message)
                    .unwrap_or_else(|e| format!("Failed to serialize message: {e}"));
                println!("{message_json}");
            }
            println!();
        }

        let client = Arc::clone(&self.client);
        let handle = tokio::spawn(async move {
            streaming_executor::stream_and_collect_response(&client, request).await
        });

        AppState::WaitingForLLM(handle)
    }
}

async fn display_user_message(
    prompt: &str,
    summaries: &[String],
    warnings: &[String],
) -> Result<()> {
    println!("[{}]", style("user").blue());
    println!("{}", style(prompt).cyan()); // Print the original prompt as is.

    if !summaries.is_empty() {
        println!("{}", style("Attached files:").dim());
        for summary in summaries {
            println!("{}", style(summary).dim());
        }
    }

    for warning in warnings {
        eprintln!("{}", style(warning).yellow());
    }

    Ok(())
}

fn spawn_stdin_channel() -> mpsc::Receiver<Option<String>> {
    let (tx, rx) = mpsc::channel(1);
    tokio::spawn(async move {
        loop {
            let result = tokio::task::spawn_blocking(|| {
                let mut buffer = String::new();
                match io::stdin().read_line(&mut buffer) {
                    Ok(0) => Ok(None), // EOF (Ctrl+D)
                    Ok(_) => Ok(Some(buffer.trim().to_string())),
                    Err(e) => Err(e),
                }
            })
            .await;

            match result {
                Ok(Ok(line_opt)) => {
                    if tx.send(line_opt).await.is_err() {
                        // Receiver was dropped, so we can exit.
                        break;
                    }
                }
                _ => {
                    // An error occurred, signal EOF and exit the task.
                    tx.send(None).await.ok();
                    break;
                }
            }
        }
    });
    rx
}
