use clap::Parser;
use nekochat_search::{config::Config, server::Engine};
use std::{io::Write, path::PathBuf, process::ExitCode};

#[derive(Parser)]
#[command(name="nekochat-search",version,about="Independent authenticated DeepSeek and ChatGPT native search service")]
struct Arguments {
    #[arg(long)] config: PathBuf,
    /// Notify an owning launcher on stdout after initialization and binding.
    #[arg(long)] notify_ready: bool,
}
async fn run(arguments: Arguments) -> Result<(),String> {
    let config = Config::load(&arguments.config)?;
    let (runtime, admin) = config.tokens()?;
    let engine = Engine::open(&config.data_dir,&runtime,&admin).map_err(|error| error.to_string())?;
    let listener = tokio::net::TcpListener::bind(config.listen).await.map_err(|_| "cannot bind search listen address")?;
    let app = engine.router();
    if arguments.notify_ready {
        let address = listener.local_addr().map_err(|_| "cannot read search listen address")?;
        let mut output = std::io::stdout().lock();
        writeln!(output, "nekochat-search-ready {address}").map_err(|_| "cannot notify search readiness")?;
        output.flush().map_err(|_| "cannot notify search readiness")?;
    }
    axum::serve(listener,app).with_graceful_shutdown(shutdown()).await.map_err(|_| "search HTTP server failed")?;
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
