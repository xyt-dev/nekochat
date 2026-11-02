mod account_events;
pub mod accounts;
pub mod api;
pub mod assets;
pub mod attachments;
pub mod auth;
pub mod chat;
mod chat_search;
pub mod codex;
pub mod config;
pub mod generation;
mod model_defaults;
mod observation;
pub mod search;
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
        .route(
            "/api/account/session",
            get(accounts::session).delete(accounts::logout),
        )
        .route("/api/account/register", post(accounts::register))
        .route("/api/account/login", post(accounts::login))
        .route("/api/account/events", get(account_events::events))
        .route(
            "/api/account",
            axum::routing::patch(accounts::update_profile),
        )
        .route("/api/account/preferences", put(accounts::preferences))
        .route(
            "/api/account/avatar",
            post(accounts::upload_avatar)
                .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route("/api/avatars/{id}", get(accounts::avatar))
        .route("/api/characters", get(user::list_characters))
        .route("/api/resources/{pack}/{id}", get(crate::assets::resource))
        .route("/api/backgrounds/{file}", get(crate::assets::background))
        .route("/api/admin/resource-packs", get(crate::assets::packs))
        .route(
            "/api/admin/backgrounds",
            post(crate::assets::upload).layer(axum::extract::DefaultBodyLimit::max(
                crate::assets::MAX_BACKGROUND,
            )),
        )
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
        .route(
            "/api/conversations/{id}/turns",
            post(chat::post_turn).get(chat::observe_turn),
        )
        .route(
            "/api/conversations/{id}/turns/{turn_id}/stop",
            post(chat::stop_turn),
        )
        .route(
            "/api/conversations/{id}/attachments",
            post(attachments::upload).layer(axum::extract::DefaultBodyLimit::max(
                attachments::MAX_DOCUMENT_BYTES,
            )),
        )
        .route(
            "/api/conversations/{id}/attachments/{attachment_id}",
            get(attachments::read),
        )
        .route(
            "/api/conversations/{id}/attachments/{attachment_id}/metadata",
            get(attachments::metadata),
        )
        .route("/api/admin/session", post(auth::admin_login))
        .route(
            "/api/admin/users",
            get(accounts::admin_users).post(accounts::admin_create_user),
        )
        .route(
            "/api/admin/users/{id}",
            axum::routing::patch(accounts::admin_update_user).delete(accounts::admin_delete_user),
        )
        .route(
            "/api/admin/users/{id}/password",
            post(accounts::admin_password),
        )
        .route(
            "/api/admin/users/{id}/legacy-conversations",
            post(accounts::admin_assign_legacy),
        )
        .route(
            "/api/admin/user-settings",
            get(accounts::admin_user_settings).put(accounts::admin_put_user_settings),
        )
        .route(
            "/api/admin/avatars",
            post(accounts::admin_upload_avatar)
                .layer(axum::extract::DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route("/api/admin/chatgpt", get(codex::status))
        .route("/api/admin/chatgpt/start", post(codex::start))
        .route("/api/admin/chatgpt/callback", post(codex::callback))
        .route("/api/admin/chatgpt/cancel", post(codex::cancel))
        .route("/api/admin/chatgpt/logout", post(codex::logout))
        .route("/api/admin/chatgpt/accounts/{client_id}", axum::routing::delete(codex::remove_account))
        .route("/api/admin/chatgpt/select", post(codex::select))
        .route("/api/admin/chatgpt/acknowledge", post(codex::acknowledge))
        .route("/api/admin/search/connection", get(search::connection).put(search::save_connection))
        .route("/api/admin/search/connection/test", post(search::test_connection))
        .route("/api/admin/search/services", get(search::services).post(search::create_service))
        .route("/api/admin/search/services/{id}", put(search::update_service).delete(search::delete_service))
        .route("/api/admin/search/services/{id}/test", post(search::test_service))
        .route("/api/admin/search/usage", get(search::usage))
        .route("/api/admin/providers", get(admin::list_providers))
        .route("/api/admin/stats", get(api::monitoring::stats))
        .route("/api/admin/quotas", get(api::monitoring::quotas))
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
