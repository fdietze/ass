use clap::Parser;

/// A simple command-line agent
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// The prompt for the agent
    pub prompt: String,
}

