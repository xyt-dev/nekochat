use clap::Parser;
use nekochat_search::server::Engine;
use serde::Deserialize;
use std::{net::SocketAddr,path::PathBuf,process::ExitCode};

#[derive(Parser)]
#[command(name="nekochat-search",version,about="Independent authenticated DeepSeek and ChatGPT native search service")]
struct Arguments { #[arg(long)] config: PathBuf }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default="default_listen")] listen: SocketAddr,
    #[serde(default="default_data")] data_dir: PathBuf,
    access_token_env: String,
    admin_token_env: String,
}
fn default_listen() -> SocketAddr { SocketAddr::from(([127,0,0,1],10011)) }
fn default_data() -> PathBuf { PathBuf::from("./search-data") }
fn secret(name: &str) -> Result<String,&'static str> {
    if name.is_empty() || name.len() > 128 || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') { return Err("invalid inbound token environment variable name"); }
    std::env::var(name).ok().filter(|token| !token.trim().is_empty()).ok_or("required inbound token environment variable is missing or empty")
}
async fn run(arguments: Arguments) -> Result<(),String> {
    let metadata = std::fs::metadata(&arguments.config).map_err(|_| "cannot read search configuration")?;
    if !metadata.is_file() || metadata.len() > 65536 { return Err("invalid search configuration file".into()); }
    let text = std::fs::read_to_string(&arguments.config).map_err(|_| "cannot read search configuration")?;
    let config: Config = toml::from_str(&text).map_err(|_| "invalid search TOML configuration")?;
    let runtime = secret(&config.access_token_env)?;
    let admin = secret(&config.admin_token_env)?;
    let engine = Engine::open(&config.data_dir,&runtime,&admin).map_err(|error| error.to_string())?;
    let listener = tokio::net::TcpListener::bind(config.listen).await.map_err(|_| "cannot bind search listen address")?;
    axum::serve(listener,engine.router()).with_graceful_shutdown(shutdown()).await.map_err(|_| "search HTTP server failed")?;
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)] {
        let mut terminate = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) { Ok(signal) => signal,Err(_) => { let _ = tokio::signal::ctrl_c().await; return; } };
        tokio::select! { _ = tokio::signal::ctrl_c() => {},_ = terminate.recv() => {} }
    }
    #[cfg(not(unix))] { let _ = tokio::signal::ctrl_c().await; }
}
#[tokio::main]
async fn main() -> ExitCode {
    match run(Arguments::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => { eprintln!("nekochat-search: {error}"); ExitCode::FAILURE },
    }
}
