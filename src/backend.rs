use serde::{Deserialize, Serialize};

#[derive(clap::ValueEnum, Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    #[default]
    OpenRouter,
    Ollama,
    OpenAI,
}

impl Backend {
    pub fn default_base_url(&self) -> &str {
        match self {
            Backend::OpenRouter => "https://openrouter.ai/api/v1/",
            Backend::Ollama => "http://localhost:11434/v1/",
            Backend::OpenAI => "https://api.openai.com/v1/",
        }
    }
    pub fn api_key_env_var(&self) -> Option<&str> {
        match self {
            Backend::OpenRouter => Some("OPENROUTER_API_KEY"),
            Backend::OpenAI => Some("OPENAI_API_KEY"),
            Backend::Ollama => None,
        }
    }
}
