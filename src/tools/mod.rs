//! # Tool Trait
//!
//! This module defines the core `Tool` trait that all tools in the application must implement.
//! It provides a standardized interface for discovering, previewing, and executing tools.

use crate::config::Config;
use crate::file_state_manager::FileStateManager;
use anyhow::Result;
use async_trait::async_trait;
use openrouter_api::models::tool::FunctionDescription;
use serde_json::Value;
use std::sync::{Arc, Mutex};

pub mod create_files;
pub mod edit_files;
pub mod execute_shell_command;
pub mod list_files;
pub mod read_files;
pub use self::create_files::FileCreatorTool;
pub use self::edit_files::FileEditorTool;
pub use self::execute_shell_command::ShellTool;
pub use self::list_files::ListFilesTool;
pub use self::read_files::FileReaderTool;

/// A trait representing a self-contained, executable tool.
///
/// This trait is designed to be object-safe, allowing for dynamic dispatch
/// via `Box<dyn Tool>`.
#[async_trait]
pub trait Tool: Send + Sync {
    /// Returns the unique, static name of the tool.
    fn name(&self) -> &'static str;

    /// Returns the JSON schema for the tool's arguments, used by the LLM.
    fn schema(&self) -> FunctionDescription;

    /// Provides a human-readable, colored preview of the action to be performed.
    /// This method acts as a full dry run, validating arguments and showing the
    /// intended changes without actually modifying any state on disk. The output
    /// is for the user to review.
    fn preview(
        &self,
        args: &Value,
        config: &Config,
        fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String>;

    /// Executes the tool's primary function.
    ///
    /// On success, this method should return a concise, machine-readable string
    /// containing only the essential information for the LLM (e.g., new file hashes,
    /// LIDs, command output). This output should be stripped of any ANSI color codes
    /// or human-centric formatting.
    ///
    /// Any output intended for the user during execution (e.g., live command output,
    /// progress indicators) should be printed directly to stdout/stderr within this method.
    async fn execute(
        &self,
        args: &Value,
        config: &Config,
        fsm: Arc<Mutex<FileStateManager>>,
    ) -> Result<String>;

    /// Checks if the tool call is safe to execute without user confirmation.
    /// The default implementation returns `true`.
    fn is_safe_for_auto_execute(&self, _args: &Value, _config: &Config) -> Result<bool> {
        Ok(true)
    }
}
