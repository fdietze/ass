use serde::{Deserialize, Serialize};

pub struct BackendConfig {
    pub base_url: String,
    pub api_key_env_var: Option<&'static str>,
}

#[derive(clap::ValueEnum, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Openrouter,
    Ollama,
    Openai,
}

impl Backend {
    pub fn config(&self) -> BackendConfig {
        match self {
            Backend::Openrouter => BackendConfig {
                base_url: "https://openrouter.ai/api/v1/".to_string(),
                api_key_env_var: Some("OPENROUTER_API_KEY"),
            },
            Backend::Ollama => BackendConfig {
                base_url: "http://localhost:11434/v1/".to_string(),
                api_key_env_var: None,
            },
            Backend::Openai => BackendConfig {
                base_url: "https://api.openai.com/v1/".to_string(),
                api_key_env_var: Some("OPENAI_API_KEY"),
            },
        }
    }
}
