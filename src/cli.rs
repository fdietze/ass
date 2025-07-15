use clap::Parser;

/// A simple command-line agent
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Parser, Debug)]
pub enum Commands {
    /// Run the agent with a prompt
    Agent {
        /// The prompt for the agent
        prompt: String,
    },
}
