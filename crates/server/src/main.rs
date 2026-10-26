use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nekochat", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server.
    Serve {
        #[arg(long)]
        config: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match Cli::parse().command {
        Command::Serve { config } => {
            let cfg = nekochat_server::config::load(&config)?;
            nekochat_server::serve(cfg).await
        }
    }
}
