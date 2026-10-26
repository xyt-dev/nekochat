pub mod config;

use axum::{Router, routing::get};

use crate::config::{Config, startup_warnings};

pub fn router() -> Router {
    Router::new().route("/api/health", get(|| async { "ok" }))
}

pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    for warning in startup_warnings(&cfg) {
        tracing::warn!("{warning}");
    }
    let listener = tokio::net::TcpListener::bind(cfg.listen).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(listener, router()).await?;
    Ok(())
}
