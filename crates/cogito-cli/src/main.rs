//! cogito-cli — Surface layer for the cogito runtime.

#![warn(clippy::pedantic)]
#![allow(clippy::missing_errors_doc)]

use clap::{Parser, Subcommand};
use cogito_cli::chat;

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
    // Build an EnvFilter from RUST_LOG (defaulting to "info"), then append
    // noise-suppression directives for HTTP plumbing crates that spam the
    // terminal with connection-pool lifecycle messages even at DEBUG level.
    // The specific directives are appended *after* the user's filter so they
    // override the global level for those targets (last wins for same
    // specificity in tracing-subscriber).
    let base = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    let filter_str = format!("{base},hyper=warn,hyper_util=warn,reqwest=warn,h2=warn,tower=warn");
    let filter = tracing_subscriber::EnvFilter::new(filter_str);

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Command::Chat(args) => chat::run(args).await,
    }
}
