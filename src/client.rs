use crate::config::Config;
use anyhow::{Result, bail};
use openrouter_api::{OpenRouterClient, Ready};
use std::time::Duration;

pub fn initialize_client(config: &Config) -> Result<OpenRouterClient<Ready>> {
    let api_key = if let Some(env_var) = config.backend.config().api_key_env_var {
        match std::env::var(env_var) {
            Ok(val) => val,
            Err(_) => bail!("environment variable {} not set", env_var),
        }
    } else {
        // TODO: only call .with_api_key if let Some(config.backend.api_key_env_var())
        "sk-or-v1-0000000000000000000000000000000000000000000000000000000000000000".to_string()
    };
    let client = OpenRouterClient::new()
        .with_base_url(&config.base_url)?
        .with_timeout(Duration::from_secs(config.timeout_seconds))
        .with_api_key(api_key)?;
    Ok(client)
}
