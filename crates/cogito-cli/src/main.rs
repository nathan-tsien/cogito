//! cogito-cli — Surface layer for the cogito runtime.

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

mod chat;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cogito", version, about = "cogito Agent Runtime CLI")]
struct Cli {
    #[command(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive chat session against an Anthropic or OpenAI-compatible endpoint.
    Chat(chat::ChatArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Command::Chat(args) => chat::run(args).await,
    }
}
