use anyhow::Result;
use clap::Parser;
use console::{Key, Term, style};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use openrouter_api::{
    OpenRouterClient,
    types::chat::{ChatCompletionRequest, Message},
    utils,
};
use std::io::{self, Write};
use std::thread;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

mod cli;
mod config;
mod diff;
mod enricher;
mod file_creator;
mod file_editor;
mod file_editor_tests;
mod file_reader;
mod file_state;
mod file_state_manager;
mod file_state_tests;
mod list_files;
mod patch;
mod path_expander;
mod permissions;
mod prompt_builder;
mod shell;
mod streaming_executor;
mod tool_executor;
mod ui;

use crate::config::Config;
use crate::file_state_manager::FileStateManager;
use crate::prompt_builder::expand_file_mentions;
use crate::shell::shell_tool_schema;
use crate::ui::{pretty_print_json, pretty_print_message};

pub enum UserAction {
    Confirm,
    Cancel,
}

pub enum UserRetryAction {
    Retry,
    Cancel,
}

fn get_user_confirmation(config: &Config) -> Result<UserAction> {
    ui::ring_bell(config);
    let term = Term::stdout();
    let prompt = style("\nPress Enter to execute tool, or Esc to cancel...")
        .dim()
        .to_string();
    print!("{prompt}");
    io::stdout().flush()?;

    loop {
        match term.read_key()? {
            Key::Enter => {
                println!();
                return Ok(UserAction::Confirm);
            }
            Key::Escape => {
                println!();
                return Ok(UserAction::Cancel);
            }
            _ => {} // Ignore other keys
        }
    }
}

fn get_user_retry(config: &Config) -> Result<UserRetryAction> {
    ui::ring_bell(config);
    let term = Term::stdout();
    let prompt = style("Connection error. Press Enter to retry, or Esc to cancel...")
        .yellow()
        .dim()
        .to_string();
    print!("\n{prompt}");
    io::stdout().flush()?;

    loop {
        match term.read_key()? {
            Key::Enter => {
                println!();
                return Ok(UserRetryAction::Retry);
            }
            Key::Escape => {
                println!();
                return Ok(UserRetryAction::Cancel);
            }
            _ => {} // Ignore other keys
        }
    }
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

    println!("Model: {}", config.model);

    let system_message = Message {
        role: "system".to_string(),
        content: system_prompt_content,
        name: None,
        tool_calls: None,
        tool_call_id: None,
    };
    print!("{}", pretty_print_message(&system_message));
    let mut messages: Vec<Message> = vec![system_message.clone()];

    let tools = vec![
        shell_tool_schema(),
        file_creator::create_file_tool_schema(),
        file_editor::edit_file_tool_schema(),
        file_reader::read_file_tool_schema(),
        list_files::list_files_tool_schema(),
    ];

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

        let mut user_cancelled_request = false;
        'turn: for _i in 0..config.max_iterations {
            let mut response_message_opt = None;
            loop {
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

                let api_call_future =
                    streaming_executor::stream_and_collect_response(&or_client, request);

                let cancellation_token = CancellationToken::new();
                let escape_listener = {
                    let cancellation_token = cancellation_token.clone();
                    tokio::task::spawn_blocking(move || {
                        loop {
                            if cancellation_token.is_cancelled() {
                                break;
                            }

                            if crossterm::terminal::enable_raw_mode().is_ok() {
                                // Poll with a zero timeout to make the check non-blocking.
                                if event::poll(Duration::from_secs(0)).unwrap_or(false) {
                                    if let Ok(Event::Key(key_event)) = event::read() {
                                        if key_event.code == KeyCode::Esc {
                                            let _ = crossterm::terminal::disable_raw_mode();
                                            break;
                                        }
                                    }
                                }
                                // Always disable raw mode immediately after the check.
                                let _ = crossterm::terminal::disable_raw_mode();
                            }

                            // Sleep for a short duration to prevent a tight loop.
                            thread::sleep(Duration::from_millis(100));
                        }
                        // Ensure raw mode is disabled on exit.
                        let _ = crossterm::terminal::disable_raw_mode();
                    })
                };

                tokio::select! {
                    biased; // Prioritize user input

                    _ = escape_listener => {
                        println!("\n{}", style("Request cancelled by user.").yellow());
                        user_cancelled_request = true;
                        break;
                    },

                    result = api_call_future => {
                        cancellation_token.cancel();
                        match result {
                            Ok(response) => {
                                response_message_opt = Some(response);
                                break;
                            }
                            Err(e) => {
                                eprintln!("\n{}", style(format!("API Connection Error: {e}")).red());
                                match get_user_retry(&config) {
                                    Ok(UserRetryAction::Retry) => continue,
                                    Ok(UserRetryAction::Cancel) => {
                                        response_message_opt = None;
                                        break;
                                    }
                                    Err(e) => {
                                        println!("Error reading input: {e:?}");
                                        break 'main_loop;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if user_cancelled_request {
                break 'turn;
            }

            if let Some(response_message) = response_message_opt.flatten() {
                messages.push(response_message.clone());

                if let Some(tool_calls) = &response_message.tool_calls {
                    let mut user_cancelled = false;
                    for tool_call in tool_calls {
                        let function_name = &tool_call.function_call.name;
                        println!("\n[{}]", style(format!("tool: {function_name}")).magenta());
                        println!("{}", pretty_print_json(&tool_call.function_call.arguments));

                        match get_user_confirmation(&config) {
                            Ok(UserAction::Confirm) => {
                                let tool_message = tool_executor::handle_tool_call(
                                    tool_call,
                                    &config,
                                    &mut file_state_manager,
                                );
                                messages.push(tool_message);
                            }
                            Ok(UserAction::Cancel) => {
                                println!("{}", style("Tool execution cancelled by user.").yellow());
                                user_cancelled = true;
                                break;
                            }
                            Err(e) => {
                                println!("Error: {e:?}");
                                break 'main_loop;
                            }
                        }
                    }
                    if user_cancelled {
                        break 'turn;
                    }
                } else {
                    // If no tool call, the LLM is giving its final answer for this turn
                    break 'turn;
                }
            } else {
                // If the response is empty, just break and prompt for user input
                break 'turn;
            }
        }

        ui::ring_bell(&config);

        'input_loop: loop {
            let term = Term::stdout();
            let mut buffer = String::new();

            term.write_str("user> ")?;

            crossterm::terminal::enable_raw_mode()?;
            loop {
                let event_result = event::read();
                match event_result {
                    Ok(Event::Key(key_event)) => match key_event.code {
                        KeyCode::Enter => {
                            crossterm::terminal::disable_raw_mode()?;
                            term.write_line("")?;
                            if buffer.is_empty() {
                                continue 'input_loop;
                            } else {
                                next_prompt = buffer;
                                break 'input_loop;
                            }
                        }
                        KeyCode::Esc => {
                            crossterm::terminal::disable_raw_mode()?;
                            if buffer.is_empty() {
                                if let Some(pos) = messages.iter().rposition(|m| m.role == "user") {
                                    if pos > 0 && pos < messages.len() - 1 {
                                        messages.truncate(pos);
                                        term.write_str("\r")?;
                                        term.clear_line()?;
                                        term.write_line(
                                            &style("Last exchange removed.").yellow().to_string(),
                                        )?;
                                        if let Some(last_message) = messages.last() {
                                            term.write_str(&pretty_print_message(last_message))?;
                                        }
                                        continue 'input_loop;
                                    }
                                }
                            } else {
                                term.clear_chars(buffer.len())?;
                                buffer.clear();
                                term.write_str("user> ")?;
                            }
                        }
                        KeyCode::Char('d') if key_event.modifiers == KeyModifiers::CONTROL => {
                            if buffer.is_empty() {
                                crossterm::terminal::disable_raw_mode()?;
                                println!("\nCTRL-D");
                                break 'main_loop;
                            }
                        }
                        KeyCode::Char('c') if key_event.modifiers == KeyModifiers::CONTROL => {
                            crossterm::terminal::disable_raw_mode()?;
                            println!("\nCTRL-C");
                            break 'main_loop;
                        }
                        KeyCode::Char(c) => {
                            buffer.push(c);
                            term.write_str(&c.to_string())?;
                        }
                        KeyCode::Backspace => {
                            if !buffer.is_empty() {
                                buffer.pop();
                                term.clear_chars(1)?;
                            }
                        }
                        _ => {}
                    },
                    Ok(Event::Resize(_, _)) => {
                        // Ignore resize events
                    }
                    Err(_) => {
                        crossterm::terminal::disable_raw_mode()?;
                        break 'main_loop;
                    }
                    _ => {}
                }
            }
        }
    }

    println!("{messages:#?}");

    Ok(())
}
