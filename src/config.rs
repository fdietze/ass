use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;

const DEFAULT_SYSTEM_PROMPT: &str = "You are a helpful assistant that can execute shell commands.
When asked to perform a task, use the available `execute_shell_command` tool.
When you have the final answer, provide it directly without using a tool.";

#[derive(Deserialize, Serialize, Debug)]
#[serde(default)]
pub struct Config {
    pub model: String,
    pub system_prompt: String,
    pub timeout_seconds: u64,
    pub max_iterations: u8,
    pub allowed_command_prefixes: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "google/gemini-2.5-flash-preview".to_string(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            timeout_seconds: 120,
            max_iterations: 5,
            allowed_command_prefixes: vec![
                "ls".to_string(),
                "cat".to_string(),
                "echo".to_string(),
                "pwd".to_string(),
            ],
        }
    }
}

pub fn load_or_create() -> Result<Config> {
    let xdg_dirs = xdg::BaseDirectories::new();
    let config_path = xdg_dirs.place_config_file("ass/config.toml")?;

    if !config_path.exists() {
        let default_config = Config::default();
        let toml_string = toml::to_string_pretty(&default_config)?;

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&config_path, toml_string)?;

        println!("Created default config at: {}", config_path.display());
        return Ok(default_config);
    }

    let config_string = fs::read_to_string(&config_path)?;
    let config: Config = toml::from_str(&config_string)?;

    // Fill in missing fields with default values
    let default_config = Config::default();
    let final_config = Config {
        model: if config.model.is_empty() {
            default_config.model
        } else {
            config.model
        },
        system_prompt: if config.system_prompt.is_empty() {
            default_config.system_prompt
        } else {
            config.system_prompt
        },
        timeout_seconds: if config.timeout_seconds == 0 {
            default_config.timeout_seconds
        } else {
            config.timeout_seconds
        },
        max_iterations: if config.max_iterations == 0 {
            default_config.max_iterations
        } else {
            config.max_iterations
        },
        allowed_command_prefixes: if config.allowed_command_prefixes.is_empty() {
            default_config.allowed_command_prefixes
        } else {
            config.allowed_command_prefixes
        },
    };

    // If any values were missing, we can write the complete config back to the file
    // This makes it easy for users to see all available options.
    let final_toml_string = toml::to_string_pretty(&final_config)?;
    if final_toml_string != config_string {
        fs::write(&config_path, final_toml_string)?;
    }

    Ok(final_config)
}
