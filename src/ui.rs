use anyhow::Result;
use console::style;
use da::{
    agent::{Agent, AgentOutput},
    tool_collection::ToolCollection,
};
use openrouter_api::{models::tool::ToolCall, types::chat::Message};
use std::{
    io::{self, Write},
    process,
    sync::Arc,
};
use tokio::sync::mpsc;

pub struct App {
    agent: Agent,
    stdin_receiver: mpsc::Receiver<Option<String>>,
}

impl App {
    pub fn new(agent: Agent) -> Self {
        Self {
            agent,
            stdin_receiver: spawn_stdin_channel(),
        }
    }

    pub async fn run(&mut self, initial_prompt: &str) -> Result<()> {
        let mut current_prompt = initial_prompt.to_string();
        let mut ctrl_c_pressed = false;

        loop {
            // Prepare the prompt and display feedback before starting the agent's turn.
            if !current_prompt.is_empty() {
                let prompt_data = self.agent.prepare_prompt(&current_prompt)?;
                display_user_message(
                    &current_prompt,
                    &prompt_data.file_summaries,
                    &prompt_data.warnings,
                )
                .await?;
                current_prompt = prompt_data.final_prompt;
            }

            // Inner loop for the agent's turn. It continues as long as there's a prompt
            // to process or if the last message was a tool result.
            while !current_prompt.is_empty()
                || self.agent.messages.last().is_some_and(|m| m.role == "tool")
            {
                let agent_output = self.agent.step_streaming(current_prompt)?;
                current_prompt = String::new(); // Consume the prompt

                let final_agent_output = if let AgentOutput::PendingLLM(mut handle) = agent_output {
                    tokio::select! {
                        biased;
                        _ = tokio::signal::ctrl_c() => {
                            if ctrl_c_pressed {
                                println!("\nShutting down...");
                                process::exit(0);
                            } else {
                                println!("\nLLM generation cancelled. Press Ctrl+C again to exit.");
                                ctrl_c_pressed = true;
                                handle.abort(); // CRITICAL: Abort the detached task
                                None
                            }
                        }
                        result = &mut handle => {
                            match result {
                                Ok(Ok(Some(msg))) => {
                                    self.agent.messages.push(msg.clone());
                                    Some(Ok(if let Some(calls) = msg.tool_calls {
                                        AgentOutput::ToolCalls(calls)
                                    } else {
                                        AgentOutput::Message(msg)
                                    }))
                                },
                                Ok(Ok(None)) => Some(Ok(AgentOutput::Done)),
                                Ok(Err(e)) => Some(Err(e)),
                                Err(e) => Some(Err(e.into())),
                            }
                        }
                    }
                } else {
                    Some(Ok(agent_output))
                };

                if let Some(result) = final_agent_output {
                    match result {
                        Ok(output) => match output {
                            AgentOutput::Message(_msg) => {
                                // The content was already printed by the streaming executor.
                            }
                            AgentOutput::ToolCalls(calls) => {
                                let tool_collection = Arc::clone(&self.agent.tool_collection);
                                self.process_tool_calls_interactively(
                                    calls,
                                    tool_collection,
                                    &mut ctrl_c_pressed,
                                )
                                .await?;
                                // After processing, the agent will run again to process results.
                            }
                            AgentOutput::Done => {
                                // Agent's turn is over.
                            }
                            AgentOutput::PendingLLM(_) => unreachable!(), // Already handled
                        },
                        Err(e) => {
                            eprintln!("{}", style(format!("[Error] Agent failed: {e}")).red());
                            // Agent's turn is over, break to user prompt.
                            break;
                        }
                    }
                } else {
                    // This happens on Ctrl+C
                    break;
                }
            }

            // After the agent's turn is complete, always wait for new user input.
            print!("\x07{} ", style("user>").cyan().bold());
            io::stdout().flush()?;

            tokio::select! {
                        biased;
                        _ = tokio::signal::ctrl_c() => {
                            if ctrl_c_pressed {
                        println!("\nShutting down...");
                        process::exit(0);
                            } else {
                                println!("\nPress Ctrl+C again to exit.");
                                ctrl_c_pressed = true;
                            }
                        }
                        line_opt = self.stdin_receiver.recv() => {
                    match line_opt.flatten() {
                                Some(input) => {
                            current_prompt = input;
                                    ctrl_c_pressed = false;
                                }
                                None => {
                                    // Ctrl+D was pressed
                            println!("\nShutting down...");
                            process::exit(0);
                        }
                    }
                }
            }
        }
    }

    async fn process_tool_calls_interactively(
        &mut self,
        tool_calls: Vec<ToolCall>,
        tool_collection: Arc<ToolCollection>,
        ctrl_c_pressed: &mut bool,
    ) -> Result<bool> {
        let mut any_tool_run = false;

        for (index, tool_call) in tool_calls.iter().enumerate() {
            println!(
                "[{}]",
                style(format!("tool: {}", tool_call.function_call.name)).magenta()
            );

            // --- Preview ---
            match tool_collection
                .preview_tool_call(
                    tool_call,
                    &self.agent.config,
                    self.agent.file_state_manager.clone(),
                )
                .await
            {
                Ok(preview) => println!("{preview}"),
                Err(e) => {
                    let error_message = format!("Preview failed, skipping: {e}");
                    eprintln!("{}", style(&error_message).red());
                    // Inform the agent that this tool failed.
                    self.agent.messages.push(Message {
                        role: "tool".to_string(),
                        content: error_message,
                        name: Some(tool_call.function_call.name.clone()),
                        tool_call_id: Some(tool_call.id.clone()),
                        tool_calls: None,
                    });
                    any_tool_run = true; // We "ran" it in the sense that we got a result for it.
                    continue; // Skip to the next tool call
                }
            };

            // --- Confirmation ---
            let is_safe = tool_collection
                .is_safe_for_auto_execute(tool_call, &self.agent.config)
                .unwrap_or(false);

            let confirmed = if self.agent.config.auto_execute && is_safe {
                true
            } else {
                print!("\x07{} ", style("Execute this tool? [Y/n] ").dim());
                io::stdout().flush()?;
                tokio::select! {
                            _ = tokio::signal::ctrl_c() => {
                        if *ctrl_c_pressed {
                            println!("\nShutting down...");
                            process::exit(0);
                        } else {
                            println!("\nPress Ctrl+C again to exit.");
                            *ctrl_c_pressed = true;
                            // Abort and generate cancellation messages for remaining tools
                            for remaining_tool_call in &tool_calls[index..] {
                                self.agent.messages.push(Message {
                                    role: "tool".to_string(),
                                    content: "Tool execution cancelled by user.".to_string(),
                                    name: Some(remaining_tool_call.function_call.name.clone()),
                                    tool_call_id: Some(remaining_tool_call.id.clone()),
                                    tool_calls: None,
                                });
                            }
                            return Ok(true);
                        }
                            }
                            line_opt = self.stdin_receiver.recv() => {
                                let input = line_opt.flatten().unwrap_or_default();
                                if input.eq_ignore_ascii_case("n") {
                                    println!("{}", style("Operation cancelled. Returning to input.").yellow());
                            // User cancelled. Generate messages for this and all subsequent tools.
                            for remaining_tool_call in &tool_calls[index..] {
                                self.agent.messages.push(Message {
                                    role: "tool".to_string(),
                                    content: "Tool execution cancelled by user.".to_string(),
                                    name: Some(remaining_tool_call.function_call.name.clone()),
                                    tool_call_id: Some(remaining_tool_call.id.clone()),
                                    tool_calls: None,
                                });
                            }
                            return Ok(true); // We generated responses, so the agent needs to run.
                            } else {
                            *ctrl_c_pressed = false; // Reset on confirmation
                            true
                        }
                    }
                }
            };

            // --- Execution ---
            if confirmed {
                let result_msg = tool_collection
                    .execute_tool_call(
                        tool_call,
                        &self.agent.config,
                        self.agent.file_state_manager.clone(),
                    )
                    .await;
                self.agent.messages.push(result_msg);
                any_tool_run = true;
            }
        }
        Ok(any_tool_run)
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
