use std::path::PathBuf;

use clap::{Parser, Subcommand};

mod search_process;

#[derive(Parser)]
#[command(name = "nekochat", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start local search, wait for readiness, then run the HTTP server.
    Serve {
        #[arg(long)]
        config: PathBuf,
        /// Configuration for the project-local search child.
        #[arg(long, default_value = "crates/search/config.toml", conflicts_with = "no_local_search")]
        search_config: PathBuf,
        /// Search executable; defaults to nekochat-search beside this executable.
        #[arg(long, conflicts_with = "no_local_search")]
        search_program: Option<PathBuf>,
        /// Use an independently managed or remote search service instead.
        #[arg(long)]
        no_local_search: bool,
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
        Command::Serve { config, search_config, search_program, no_local_search } => {
            let cfg = nekochat_server::config::load(&config)?;
            if no_local_search {
                nekochat_server::serve(cfg).await
            } else {
                search_process::serve(cfg, &search_config, search_program).await
            }
        }
    }
}
