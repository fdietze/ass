use clap::Parser;
use da::config::ConfigLayer;

/// A command-line interface for the `da` agent.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The prompt for the agent
    pub prompt: Option<String>,

    #[command(flatten)]
    pub overrides: ConfigLayer,
}
