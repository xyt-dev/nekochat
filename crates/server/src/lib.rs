pub mod api;
pub mod auth;
pub mod config;
pub mod state;

use anyhow::Context;
use axum::routing::{get, post, put};
use axum::{Router, middleware};
use nekochat_gateway::Provider;
use nekochat_store::Store;

use crate::api::{admin, user};
use crate::config::{Config, resolve_api_keys, startup_warnings};
use crate::state::AppState;

pub fn app(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/session", post(auth::access_login))
        .route("/api/models", get(user::list_models))
        .route(
            "/api/conversations",
            get(user::list_conversations).post(user::create_conversation),
        )
        .route(
            "/api/conversations/{id}",
            get(user::get_conversation).delete(user::delete_conversation),
        )
        .route(
            "/api/conversations/{id}/clear",
            post(user::clear_conversation),
        )
        .route("/api/conversations/{id}/leaf", put(user::set_leaf))
        .route("/api/admin/session", post(auth::admin_login))
        .route("/api/admin/providers", get(admin::list_providers))
        .route(
            "/api/admin/providers/{key}/models",
            get(admin::provider_models),
        )
        .route(
            "/api/admin/models",
            get(admin::list_models).post(admin::create_model),
        )
        .route(
            "/api/admin/models/{id}",
            put(admin::update_model).delete(admin::delete_model),
        )
        .route(
            "/api/admin/characters",
            get(admin::list_characters).post(admin::create_character),
        )
        .route(
            "/api/admin/characters/{id}",
            put(admin::update_character).delete(admin::delete_character),
        )
        .route(
            "/api/admin/settings",
            get(admin::get_settings).put(admin::put_settings),
        );
    api.layer(middleware::from_fn_with_state(state.clone(), auth::gate))
        .with_state(state)
}

pub async fn serve(cfg: Config) -> anyhow::Result<()> {
    let providers = resolve_api_keys(&cfg, |name| std::env::var(name).ok())?
        .into_iter()
        .map(|(p, api_key)| Provider {
            key: p.key,
            label: p.label,
            base_url: p.base_url,
            models_url: p.models_url,
            api_key,
            session_header: p.session_header,
        })
        .collect();
    let db = cfg.data_dir.join("nekochat.db");
    let store = Store::open(&db).with_context(|| format!("打开数据库 {} 失败", db.display()))?;
    for warning in startup_warnings(&cfg) {
        tracing::warn!("{warning}");
    }
    let listener = tokio::net::TcpListener::bind(cfg.listen).await?;
    tracing::info!(addr = %listener.local_addr()?, "listening");
    axum::serve(listener, app(AppState::new(cfg, providers, store))).await?;
    Ok(())
}
