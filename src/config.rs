use anyhow::Result;
use clap::Args;
use serde::{Deserialize, Serialize};
use std::fs;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant that can execute shell commands.
When asked to perform a task, use the available `execute_shell_command` tool.
When you have the final answer, provide it directly without using a tool.";

/// Represents a layer of configuration, either from a file or from the command line.
/// All fields are optional.
#[derive(Args, Deserialize, Debug, Default)]
#[serde(default)]
pub struct ConfigLayer {
    /// The model to use for the agent.
    #[arg(long)]
    pub model: Option<String>,

    /// The system prompt to use.
    #[arg(long)]
    pub system_prompt: Option<String>,

    /// The timeout for API requests in seconds.
    #[arg(long)]
    pub timeout_seconds: Option<u64>,

    /// The maximum number of tool-use iterations.
    #[arg(long)]
    pub max_iterations: Option<u8>,

    /// The maximum number of lines to read from a file.
    #[arg(long)]
    pub max_read_lines: Option<u64>,

    /// Command prefixes that the agent is allowed to execute.
    #[arg(long, value_delimiter = ',')]
    pub allowed_command_prefixes: Vec<String>,

    /// Paths to ignore when listing or reading files.
    #[arg(long, value_delimiter = ',')]
    pub ignored_paths: Vec<String>,

    /// Paths that the agent is allowed to access.
    #[arg(long, value_delimiter = ',')]
    pub accessible_paths: Vec<String>,

    /// Enable or disable the terminal bell.
    #[arg(long)]
    pub terminal_bell: Option<bool>,

    /// Show the system prompt before starting the conversation.
    #[arg(long)]
    pub show_system_prompt: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(default)]
pub struct Config {
    pub model: String,
    pub system_prompt: String,
    pub timeout_seconds: u64,
    pub max_iterations: u8,
    pub max_read_lines: u64,
    pub allowed_command_prefixes: Vec<String>,
    pub ignored_paths: Vec<String>,
    pub accessible_paths: Vec<String>,
    pub terminal_bell: bool,
    pub show_system_prompt: bool,
}

impl Config {
    /// Merges a configuration layer into the current configuration.
    /// Values in the layer take precedence.
    pub fn merge(&mut self, layer: &ConfigLayer) {
        if let Some(model) = &layer.model {
            self.model = model.clone();
        }
        if let Some(system_prompt) = &layer.system_prompt {
            self.system_prompt = system_prompt.clone();
        }
        if let Some(timeout_seconds) = layer.timeout_seconds {
            self.timeout_seconds = timeout_seconds;
        }
        if let Some(max_iterations) = layer.max_iterations {
            self.max_iterations = max_iterations;
        }
        if let Some(max_read_lines) = layer.max_read_lines {
            self.max_read_lines = max_read_lines;
        }
        if !layer.allowed_command_prefixes.is_empty() {
            self.allowed_command_prefixes = layer.allowed_command_prefixes.clone();
        }
        if !layer.ignored_paths.is_empty() {
            self.ignored_paths = layer.ignored_paths.clone();
        }
        if !layer.accessible_paths.is_empty() {
            self.accessible_paths = layer.accessible_paths.clone();
        }
        if let Some(terminal_bell) = layer.terminal_bell {
            self.terminal_bell = terminal_bell;
        }
        if let Some(show_system_prompt) = layer.show_system_prompt {
            self.show_system_prompt = show_system_prompt;
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "google/gemini-2.5-flash-preview".to_string(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            timeout_seconds: 120,
            max_iterations: 5,
            max_read_lines: 1000,
            allowed_command_prefixes: vec![
                "ls".to_string(),
                "cat".to_string(),
                "echo".to_string(),
                "pwd".to_string(),
            ],
            ignored_paths: vec![".git".to_string()],
            accessible_paths: vec![".".to_string()],
            terminal_bell: true,
            show_system_prompt: false,
        }
    }
}

/// Loads configuration from defaults, a configuration file, and CLI arguments.
/// The layers are applied in order, with later layers taking precedence.
///
/// 1. `Config::default()` is used as the base.
/// 2. The `config.toml` file is loaded and merged.
/// 3. The `cli_layer` from command-line arguments is merged.
///
/// The function will also create or update the `config.toml` file to include any
/// newly available default settings, making them discoverable to the user.
pub fn load(cli_layer: &ConfigLayer) -> Result<Config> {
    let xdg_dirs = xdg::BaseDirectories::new();
    let config_path = xdg_dirs.place_config_file("ass/config.toml")?;

    // Load file layer, or use a default if it doesn't exist or fails to parse.
    let file_layer: ConfigLayer = if config_path.exists() {
        let config_string = fs::read_to_string(&config_path)?;
        toml::from_str(&config_string).unwrap_or_default()
    } else {
        ConfigLayer::default()
    };

    // Determine the state of the config as it should be on disk.
    let mut config_for_disk = Config::default();
    config_for_disk.merge(&file_layer);

    // If the on-disk representation is out of date or doesn't exist, write it.
    let new_disk_toml = toml::to_string_pretty(&config_for_disk)?;
    let old_disk_toml = fs::read_to_string(&config_path).unwrap_or_default();

    if new_disk_toml != old_disk_toml {
        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&config_path, new_disk_toml)?;
        if old_disk_toml.is_empty() {
            println!("Created default config at: {}", config_path.display());
        }
    }

    // Start with the on-disk config state and merge the final CLI layer.
    let mut final_config = config_for_disk;
    final_config.merge(cli_layer);

    Ok(final_config)
}
