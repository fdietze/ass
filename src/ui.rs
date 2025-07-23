use crate::config::Config;
use crate::file_state_manager::FileStateManager;
use crate::prompt_builder;
use crate::shell;
use crate::streaming_executor;
use crate::tool_executor;
use anyhow::Result;
use console::style;
use openrouter_api::models::tool::ToolCall;
use openrouter_api::types::chat::{ChatCompletionRequest, Message};
use openrouter_api::{OpenRouterClient, Ready};
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
    WaitingForToolConfirmation(Vec<ToolCall>),
    ExecutingTool(JoinHandle<anyhow::Result<Message>>),
    Shutdown,
}

pub struct App {
    pub config: Config,
    pub client: Arc<OpenRouterClient<Ready>>,
    pub messages: Vec<Message>,
    pub file_state_manager: Arc<Mutex<FileStateManager>>,
    pub state: AppState,
    stdin_receiver: mpsc::Receiver<Option<String>>,
}

impl App {
    pub fn new(config: Config, client: OpenRouterClient<Ready>) -> Self {
        let _file_state_manager = FileStateManager::new();
        Self {
            config,
            client: Arc::new(client),
            messages: Vec::new(),
            file_state_manager: Arc::new(Mutex::new(FileStateManager::new())),
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
                    print!("{} ", style("user>").cyan().bold());
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
                AppState::ExecutingTool(_) => {
                    let (new_state, should_break) = tokio::select! {
                            _ = tokio::signal::ctrl_c() => {
                                if let AppState::ExecutingTool(handle) = &mut self.state {
                                    println!("\n{}", style("Tool execution cancelled.").yellow());
                                    handle.abort();
                                }
                                (AppState::WaitingForUserInput, false)
                            }
                            result = async {
                                if let AppState::ExecutingTool(handle) = &mut self.state {
                                    handle.await
                    } else {
                                    unreachable!()
                                }
                            } => {
                                match result {
                                    Ok(Ok(tool_message)) => {
                                        self.messages.push(tool_message);
                                        (self.spawn_llm_call(), false)
                                    }
                                    Ok(Err(e)) => {
                                        eprintln!("{}", style(format!("Tool execution failed: {e}")).red());
                                        (AppState::WaitingForUserInput, false)
                                    }
                                    Err(e) => {
                                         eprintln!("{}", style(format!("Tool task failed: {e}")).red());
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
                                    let has_tool_calls = response_message.tool_calls.is_some();
                                    self.messages.push(response_message);
                                    if has_tool_calls {
                                        if let Some(tool_calls) = self.messages.last().unwrap().tool_calls.clone() {
                                            (AppState::WaitingForToolConfirmation(tool_calls), false)
                                        } else {
                                            (AppState::WaitingForUserInput, false)
                                        }
                    } else {
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
                AppState::WaitingForToolConfirmation(tool_calls) => {
                    println!("\n{}", style("Proposed tool calls:").magenta());
                    for tool_call in tool_calls {
                        let function_name = &tool_call.function_call.name;
                        println!("[{}]", style(format!("tool: {function_name}")).magenta());
                        let pretty_args = pretty_print_json(&tool_call.function_call.arguments);
                        println!("{pretty_args}");
                    }

                    print!(
                        "{} ",
                        style("Press Enter to execute, or Ctrl+C to cancel...").dim()
                    );
                    io::stdout().flush()?;

                    tokio::select! {
                            _ = tokio::signal::ctrl_c() => {
                                println!("\n{}", style("Tool confirmation cancelled.").yellow());
                                self.state = AppState::WaitingForUserInput;
                            }
                            line_opt = self.stdin_receiver.recv() => {
                                if let Some(_input) = line_opt.flatten() {
                                    // User pressed Enter
                                    if let AppState::WaitingForToolConfirmation(tool_calls) = &self.state {
                                        if let Some(first_tool_call) = tool_calls.first().cloned() {
                                            let config = self.config.clone();
                                            let fsm = Arc::clone(&self.file_state_manager);
                                            let handle = tokio::spawn(async move {
                                                tool_executor::handle_tool_call(&first_tool_call, &config, fsm).await
                                            });
                                            self.state = AppState::ExecutingTool(handle);
                                        } else {
                                            self.state = AppState::WaitingForUserInput;
                                        }
                                    }
                    } else {
                                    // Channel closed or Ctrl+D, treat as cancellation
                                    println!();
                                    self.state = AppState::Shutdown;
                                }
                            }
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
                    display_user_message(prompt, &self.config, &self.file_state_manager).await?;

                    let final_prompt = {
                        let mut fsm = self.file_state_manager.lock().unwrap();
                        prompt_builder::expand_file_mentions(prompt, &self.config, &mut fsm)?
                    };

                    let user_message = Message {
                        role: "user".to_string(),
                        content: final_prompt,
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
        let tools = vec![
            shell::shell_tool_schema(),
            crate::file_creator::create_file_tool_schema(),
            crate::file_editor::edit_file_tool_schema(),
            crate::file_reader::read_file_tool_schema(),
            crate::list_files::list_files_tool_schema(),
        ];

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

        let client = Arc::clone(&self.client);
        let handle = tokio::spawn(async move {
            streaming_executor::stream_and_collect_response(&client, request).await
        });

        AppState::WaitingForLLM(handle)
    }
}

async fn display_user_message(
    prompt: &str,
    config: &Config,
    file_state_manager: &Arc<Mutex<FileStateManager>>,
) -> Result<()> {
    println!("[{}]", style("user").blue());
    println!("{}", style(prompt).cyan()); // Print the original prompt as is.

    let enrichments = crate::enricher::extract_enrichments(prompt);
    if enrichments.mentioned_files.is_empty() {
        return Ok(());
    }

    let expansion_result = crate::path_expander::expand_and_validate(
        &enrichments.mentioned_files,
        &config.ignored_paths,
    );

    let summaries: Vec<String> = expansion_result
        .files
        .iter()
        .filter_map(|file_path| {
            let mut fsm = file_state_manager.lock().unwrap();
            match fsm.open_file(file_path) {
                Ok(file_state) => {
                    let total_lines = file_state.lines.len();
                    let filename = std::path::Path::new(file_path)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    Some(format!("[{filename} ({total_lines} lines)]"))
                }
                Err(_) => None, // Don't include files that failed to open in the summary
            }
        })
        .collect();

    if !summaries.is_empty() {
        println!("{}", style("Attached files:").dim());
        for summary in summaries {
            println!("{}", style(summary).dim());
        }
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

fn pretty_print_json(json_string: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(json_string) {
        Ok(value) => {
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| json_string.to_string())
        }
        Err(_) => json_string.to_string(),
    }
}

// The test for the old collapsing logic is no longer needed and will be removed.
