pub mod api;
pub mod auth;
pub mod chat;
pub mod config;
pub mod state;
pub mod web;

use std::sync::Arc;

use anyhow::Context;
use axum::routing::{get, post, put};
use axum::{Router, middleware};
use nekochat_gateway::Provider;
use nekochat_store::Store;

use crate::api::{admin, user};
use crate::config::{Config, resolve_api_keys, startup_warnings};
use crate::state::AppState;
use crate::web::{EmbeddedDist, WebAssets};

/// The full application with the frontend embedded at compile time.
pub fn app(state: AppState) -> Router {
    app_with_assets(state, Arc::new(EmbeddedDist))
}

/// The full application serving the frontend from `assets`.
pub fn app_with_assets(state: AppState, assets: Arc<dyn WebAssets>) -> Router {
    let api = Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/session", post(auth::access_login))
        .route("/api/models", get(user::list_models))
        .route("/api/themes", get(web::list_themes))
        .route("/themes/{file}", get(web::theme_file))
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
        .route("/api/conversations/{id}/turns", post(chat::post_turn))
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
        )
        .fallback(move |req| web::fallback(assets, req))
        .method_not_allowed_fallback(api::wrong_method);
    // The last layer runs first: the host check precedes the password gate.
    api.layer(middleware::from_fn_with_state(state.clone(), auth::gate))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::check_host,
        ))
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
