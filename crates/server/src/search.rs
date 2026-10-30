//! Fixed same-origin administration of the independently authenticated search engine.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use nekochat_search::client::SearchClient;
use nekochat_search::{AuthMode, SearchRequest, SearchResponse, ServiceView, ServiceWrite};
use nekochat_store::{SearchUsageDashboard, SearchUsageRecord, SettingKey, Store, new_id, now_ms};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::api::{ApiError, JsonBody, blocking};
use crate::state::AppState;

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub max_calls_per_turn: u32,
    pub max_results: u32,
}

// Credentials are intentionally neither Debug nor part of the public connection view.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SearchConnection {
    enabled: bool,
    base_url: String,
    timeout_secs: u64,
    max_calls_per_turn: u32,
    max_results: u32,
    access_token: String,
    admin_token: String,
}

impl Default for SearchConnection {
    fn default() -> Self {
        Self { enabled: false, base_url: String::new(), timeout_secs: 60, max_calls_per_turn: 3, max_results: 5, access_token: String::new(), admin_token: String::new() }
    }
}

#[derive(Serialize)]
pub struct ConnectionView {
    enabled: bool,
    base_url: String,
    timeout_secs: u64,
    max_calls_per_turn: u32,
    max_results: u32,
    has_access_token: bool,
    has_admin_token: bool,
}

impl SearchConnection {
    fn view(&self) -> ConnectionView {
        ConnectionView { enabled: self.enabled, base_url: self.base_url.clone(), timeout_secs: self.timeout_secs, max_calls_per_turn: self.max_calls_per_turn, max_results: self.max_results, has_access_token: !self.access_token.is_empty(), has_admin_token: !self.admin_token.is_empty() }
    }

    fn clients(&self) -> Result<Option<(SearchClient, SearchClient)>, ApiError> {
        if !(1..=180).contains(&self.timeout_secs) || !(1..=8).contains(&self.max_calls_per_turn) || !(1..=10).contains(&self.max_results) {
            return Err(ApiError::bad_request("搜索超时或调用数量超出允许范围"));
        }
        if !self.base_url.is_empty() {
            let url = reqwest::Url::parse(&self.base_url).map_err(|_| ApiError::bad_request("搜索引擎地址无效"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() || !matches!(url.path(), "" | "/") {
                return Err(ApiError::bad_request("搜索引擎地址必须是无凭据的 HTTP(S) 服务地址"));
            }
        }
        if !self.enabled { return Ok(None); }
        if self.base_url.is_empty() || self.access_token.is_empty() || self.admin_token.is_empty() || self.access_token == self.admin_token {
            return Err(ApiError::bad_request("启用搜索需要地址及不同的运行和管理令牌"));
        }
        let timeout = Duration::from_secs(self.timeout_secs);
        let runtime = SearchClient::new(&self.base_url, &self.access_token, timeout).map_err(|_| ApiError::bad_request("搜索引擎连接配置无效"))?;
        let admin = SearchClient::new(&self.base_url, &self.admin_token, timeout).map_err(|_| ApiError::bad_request("搜索引擎连接配置无效"))?;
        Ok(Some((runtime, admin)))
    }
}

struct CachedConnection {
    connection: SearchConnection,
    clients: Option<(SearchClient, SearchClient)>,
    invalid: bool,
}

pub struct SearchBridge {
    store: Arc<Store>,
    account: Arc<crate::codex::CodexAccount>,
    http: reqwest::Client,
    credential_slot: Arc<tokio::sync::Semaphore>,
    cached: RwLock<CachedConnection>,
    // Serialize role binding writes with engine deletion so a validated reference cannot race deletion.
    pub(crate) mutations: tokio::sync::Mutex<()>,
}

impl SearchBridge {
    pub fn new(store: Arc<Store>, account: Arc<crate::codex::CodexAccount>, http: reqwest::Client) -> Self {
        let loaded = store.get_setting(SettingKey::SearchConnection)
            .and_then(|raw| raw.map(|raw| serde_json::from_str::<SearchConnection>(&raw)).transpose().map_err(Into::into));
        let (connection, invalid) = match loaded {
            Ok(connection) => (connection.unwrap_or_default(), false),
            Err(_) => (SearchConnection::default(), true),
        };
        let (clients, invalid) = match connection.clients() {
            Ok(clients) => (clients, invalid),
            Err(_) => (None, true),
        };
        Self { store, account, http, credential_slot: Arc::new(tokio::sync::Semaphore::new(1)), cached: RwLock::new(CachedConnection { connection, clients, invalid }), mutations: tokio::sync::Mutex::new(()) }
    }

    pub fn runtime(&self) -> Result<(SearchClient, SearchLimits), ApiError> {
        let cached = self.cached.read().unwrap_or_else(|poison| poison.into_inner());
        if cached.invalid { return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "搜索连接配置不可用")); }
        let client = cached.clients.as_ref().ok_or_else(unconfigured)?.0.clone();
        Ok((client, SearchLimits { max_calls_per_turn: cached.connection.max_calls_per_turn, max_results: cached.connection.max_results }))
    }

    fn admin(&self) -> Result<SearchClient, ApiError> {
        let cached = self.cached.read().unwrap_or_else(|poison| poison.into_inner());
        if cached.invalid { return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "搜索连接配置不可用")); }
        Ok(cached.clients.as_ref().ok_or_else(unconfigured)?.1.clone())
    }

    fn effective_service(&self, mut service: ServiceView) -> ServiceView {
        if service.auth_mode == AuthMode::SharedChatgpt {
            service.connected = self.account.connected();
        }
        service
    }

    pub(crate) async fn catalog(&self, client: &SearchClient) -> Result<Vec<ServiceView>, ApiError> {
        Ok(client.services().await.map_err(engine_error)?.into_iter().map(|service| self.effective_service(service)).collect())
    }

    pub(crate) async fn shared_access_token(&self) -> Result<String, ApiError> {
        let unavailable = || ApiError::new(StatusCode::CONFLICT, "站点的 ChatGPT 订阅暂时不可用，请在模型管理中检查连接");
        if !self.account.connected() { return Err(unavailable()); }
        let permit = self.credential_slot.clone().acquire_owned().await.map_err(|_| unavailable())?;
        let account = self.account.clone();
        let http = self.http.clone();
        // Dropping a search detaches this owned, single-slot task instead of aborting a
        // rotating refresh before the existing account manager commits its replacement.
        // The pooled HTTP client's connect/read limits bound the manager's requests.
        tokio::spawn(async move {
            let _permit = permit;
            account.access_token(&http).await
        }).await.map_err(|_| unavailable())?.map_err(|_| unavailable())
    }

    async fn search(&self, client: &SearchClient, auth_mode: AuthMode, request: &SearchRequest) -> Result<SearchResponse, ApiError> {
        if auth_mode == AuthMode::SharedChatgpt {
            let token = self.shared_access_token().await?;
            client.search_with_account(request, &token).await.map_err(engine_error)
        } else {
            client.search(request).await.map_err(engine_error)
        }
    }

    pub(crate) async fn validate_binding(&self, id: &str) -> Result<(), ApiError> {
        service_id(id)?;
        let (client, _) = self.runtime()?;
        let services = self.catalog(&client).await?;
        if !services.iter().any(|service| service.id == id && service.enabled && service.connected) {
            return Err(ApiError::bad_request("搜索服务不存在、未启用或未连接"));
        }
        Ok(())
    }
}

fn unconfigured() -> ApiError { ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "搜索引擎未配置或未启用") }
fn engine_error(error: nekochat_search::SearchError) -> ApiError {
    // Engine authentication failure is an upstream configuration error, not a browser logout.
    let status = match error.status() {
        400 => StatusCode::BAD_REQUEST,
        404 => StatusCode::NOT_FOUND,
        409 => StatusCode::CONFLICT,
        413 => StatusCode::PAYLOAD_TOO_LARGE,
        429 => StatusCode::TOO_MANY_REQUESTS,
        504 => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::BAD_GATEWAY,
    };
    ApiError::new(status, "搜索引擎请求失败，请检查连接、授权或服务状态")
}
fn service_id(id: &str) -> Result<(), ApiError> {
    uuid::Uuid::parse_str(id).map_err(|_| ApiError::bad_request("搜索服务标识无效"))?;
    Ok(())
}
fn service_path(id: &str) -> Result<String, ApiError> {
    service_id(id)?;
    Ok(format!("/v1/admin/services/{id}"))
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectionWrite {
    enabled: bool,
    base_url: String,
    timeout_secs: u64,
    max_calls_per_turn: u32,
    max_results: u32,
    access_token: Option<String>,
    admin_token: Option<String>,
    clear_access_token: bool,
    clear_admin_token: bool,
}
impl Default for ConnectionWrite {
    fn default() -> Self {
        Self { enabled: false, base_url: String::new(), timeout_secs: 60, max_calls_per_turn: 3, max_results: 5, access_token: None, admin_token: None, clear_access_token: false, clear_admin_token: false }
    }
}

pub async fn connection(State(state): State<AppState>) -> Result<Json<ConnectionView>, ApiError> {
    let cached = state.search.cached.read().unwrap_or_else(|poison| poison.into_inner());
    if cached.invalid { return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "搜索连接配置不可用")); }
    Ok(Json(cached.connection.view()))
}

pub async fn save_connection(State(state): State<AppState>, JsonBody(body): JsonBody<ConnectionWrite>) -> Result<Json<ConnectionView>, ApiError> {
    let _guard = state.search.mutations.lock().await;
    let old = state.search.cached.read().unwrap_or_else(|poison| poison.into_inner()).connection.clone();
    fn token(old: String, replacement: Option<String>, clear: bool) -> String {
        if clear { String::new() } else { replacement.filter(|token| !token.trim().is_empty()).unwrap_or(old) }
    }
    let connection = SearchConnection { enabled: body.enabled, base_url: body.base_url.trim().trim_end_matches('/').to_owned(), timeout_secs: body.timeout_secs, max_calls_per_turn: body.max_calls_per_turn, max_results: body.max_results, access_token: token(old.access_token, body.access_token, body.clear_access_token), admin_token: token(old.admin_token, body.admin_token, body.clear_admin_token) };
    let clients = connection.clients()?;
    let encoded = serde_json::to_string(&connection).map_err(|_| ApiError::internal())?;
    let store = state.search.store.clone();
    blocking(move || store.set_setting(SettingKey::SearchConnection, Some(&encoded))).await?;
    let view = connection.view();
    *state.search.cached.write().unwrap_or_else(|poison| poison.into_inner()) = CachedConnection { connection, clients, invalid: false };
    Ok(Json(view))
}

pub async fn test_connection(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let _guard = state.search.mutations.lock().await;
    let (runtime, _) = state.search.runtime()?;
    let admin = state.search.admin()?;
    let services = runtime.services().await.map_err(engine_error)?;
    let _: Vec<ServiceView> = admin.request_json(Method::GET, "/v1/admin/services", None).await.map_err(engine_error)?;
    Ok(Json(json!({"ok":true,"services":services.len()})))
}

pub async fn services(State(state): State<AppState>) -> Result<Json<Vec<ServiceView>>, ApiError> {
    let services: Vec<ServiceView> = state.search.admin()?.request_json(Method::GET, "/v1/admin/services", None).await.map_err(engine_error)?;
    Ok(Json(services.into_iter().map(|service| state.search.effective_service(service)).collect()))
}

pub async fn create_service(State(state): State<AppState>, JsonBody(body): JsonBody<ServiceWrite>) -> Result<(StatusCode, Json<ServiceView>), ApiError> {
    let body = serde_json::to_value(body).map_err(|_| ApiError::bad_request("搜索服务配置无效"))?;
    let service: ServiceView = state.search.admin()?.request_json(Method::POST, "/v1/admin/services", Some(&body)).await.map_err(engine_error)?;
    Ok((StatusCode::CREATED, Json(state.search.effective_service(service))))
}

pub async fn update_service(State(state): State<AppState>, Path(id): Path<String>, JsonBody(body): JsonBody<ServiceWrite>) -> Result<Json<ServiceView>, ApiError> {
    let path = service_path(&id)?;
    let body = serde_json::to_value(body).map_err(|_| ApiError::bad_request("搜索服务配置无效"))?;
    let service: ServiceView = state.search.admin()?.request_json(Method::PUT, &path, Some(&body)).await.map_err(engine_error)?;
    Ok(Json(state.search.effective_service(service)))
}

pub async fn delete_service(State(state): State<AppState>, Path(id): Path<String>) -> Result<StatusCode, ApiError> {
    let path = service_path(&id)?;
    let _guard = state.search.mutations.lock().await;
    let store = state.store.clone();
    if blocking(move || store.search_service_in_use(&id)).await? {
        return Err(ApiError::new(StatusCode::CONFLICT, "搜索服务仍被角色引用，请先解除绑定"));
    }
    state.search.admin()?.delete(&path).await.map_err(engine_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestSearch { query: String }

pub async fn test_service(State(state): State<AppState>, Path(id): Path<String>, JsonBody(body): JsonBody<TestSearch>) -> Result<Json<SearchResponse>, ApiError> {
    service_id(&id)?;
    if body.query.trim().is_empty() || body.query.chars().count() > 128 { return Err(ApiError::bad_request("搜索词必须为 1..128 个字符")); }
    let (client, limits) = state.search.runtime()?;
    let services = state.search.catalog(&client).await?;
    let service = services.iter().find(|service| service.id == id && service.enabled)
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "搜索服务不存在或未启用"))?;
    let request = SearchRequest { request_id: new_id(), service_id: id, query: body.query, limit: limits.max_results };
    let result = state.search.search(&client, service.auth_mode, &request).await;
    let usage = match &result {
        Ok(response) => Some(serde_json::to_value(&response.usage).map_err(|_| ApiError::internal())?),
        Err(_) => None,
    };
    let record = SearchUsageRecord { id: request.request_id, at_ms: now_ms(), service_id: request.service_id, status: if result.is_ok() { "complete" } else { "error" }.into(), usage };
    let store = state.store.clone();
    blocking(move || store.insert_search_usage(&record)).await?;
    Ok(Json(result?))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageRange { from_ms: Option<i64>, to_ms: Option<i64> }

pub async fn usage(State(state): State<AppState>, Query(range): Query<UsageRange>) -> Result<Json<SearchUsageDashboard>, ApiError> {
    const DAY: i64 = 86_400_000;
    let to = range.to_ms.unwrap_or_else(now_ms);
    let from = range.from_ms.unwrap_or_else(|| to.saturating_sub(7 * DAY).max(0));
    if from < 0 || to < from || to.checked_sub(from).is_none_or(|span| span > 31 * DAY) { return Err(ApiError::bad_request("搜索用量日期范围必须在 31 天以内")); }
    let span = to - from;
    let bucket_ms = if span <= DAY { 15 * 60_000 } else if span <= 7 * DAY { 60 * 60_000 } else { 6 * 60 * 60_000 };
    let store = state.store.clone();
    Ok(Json(blocking(move || store.search_usage_dashboard(from, to, bucket_ms)).await?))
}
