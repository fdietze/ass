use serde::{Deserialize, Serialize};

#[derive(clap::ValueEnum, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    Openrouter,
    Ollama,
    Openai,
}

impl Backend {
    pub fn default_base_url(&self) -> &str {
        match self {
            Backend::Openrouter => "https://openrouter.ai/api/v1/",
            Backend::Ollama => "http://localhost:11434/v1/",
            Backend::Openai => "https://api.openai.com/v1/",
        }
    }
    pub fn api_key_env_var(&self) -> Option<&str> {
        match self {
            Backend::Openrouter => Some("OPENROUTER_API_KEY"),
            Backend::Openai => Some("OPENAI_API_KEY"),
            Backend::Ollama => None,
        }
    }
}
