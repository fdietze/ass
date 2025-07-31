use alors::config::ConfigLayer;
use clap::Parser;

/// A command-line interface for the `alors` agent.
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The prompt for the agent
    pub prompt: Option<String>,

    #[command(flatten)]
    pub overrides: ConfigLayer,
}
