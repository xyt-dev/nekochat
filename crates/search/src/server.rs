use crate::{cancel::Registry, provider, reader::Reader, validation, vault::{Service,Vault}, AuthMode, FetchRequest, FetchResponse, SearchError, SearchRequest, SearchResponse, ServiceKind, ServiceView, ServiceWrite};
use axum::{extract::{DefaultBodyLimit,Path,State,rejection::JsonRejection}, http::{HeaderMap,Request,StatusCode}, middleware::{self,Next}, response::{IntoResponse,Response}, routing::{get,post}, Json, Router};
use serde::Deserialize;
use serde_json::json;
use std::{path::Path as FilePath,sync::{Arc,Mutex},time::Duration};
use subtle::ConstantTimeEq;

/// Owns the exclusive private-data lock for the lifetime of its routers and requests.
#[derive(Clone)]
pub struct Engine { inner: Arc<Inner> }
struct Inner {
    vault: Mutex<Vault>,
    runtime_token: String,
    admin_token: String,
    http: reqwest::Client,
    reader: Reader,
    requests: Registry,
    slots: Arc<tokio::sync::Semaphore>,
}
impl Engine {
    pub fn open(data_dir: &FilePath,runtime_token: &str,admin_token: &str) -> Result<Self,SearchError> {
        if runtime_token.trim().is_empty() || admin_token.trim().is_empty() || runtime_token == admin_token || [runtime_token,admin_token].iter().any(|token| token.len() > 16384 || token.chars().any(char::is_control)) { return Err(SearchError::new(400,"distinct_runtime_and_admin_tokens_required")); }
        let vault = Mutex::new(Vault::open(data_dir)?);
        let http = provider::http_client()?;
        Ok(Self { inner:Arc::new(Inner { vault,runtime_token:runtime_token.to_owned(),admin_token:admin_token.to_owned(),http,reader:Reader::new()?,requests:Registry::default(),slots:Arc::new(tokio::sync::Semaphore::new(32)) }) })
    }
    pub fn router(&self) -> Router {
        let runtime = Router::new()
            .route("/v1/services",get(runtime_services))
            .route("/v1/search",post(search))
            .route("/v1/fetch",post(fetch))
            .route("/v1/cancel",post(cancel))
            .route_layer(middleware::from_fn_with_state(self.inner.clone(),runtime_auth));
        let admin = Router::new()
            .route("/v1/admin/services",get(admin_services).post(create_service))
            .route("/v1/admin/services/{id}",axum::routing::put(update_service).delete(delete_service))
            .route_layer(middleware::from_fn_with_state(self.inner.clone(),admin_auth));
        Router::new().route("/healthz",get(|| async { Json(json!({"status":"ok"})) }))
            .merge(runtime).merge(admin)
            .fallback(|| async { SearchError::new(404,"route_not_found") })
            .method_not_allowed_fallback(|| async { SearchError::new(405,"method_not_allowed") })
            .layer(DefaultBodyLimit::max(16 * 1024)).with_state(self.inner.clone())
    }
}
impl IntoResponse for SearchError {
    fn into_response(self) -> Response {
        (StatusCode::from_u16(self.status()).unwrap_or(StatusCode::BAD_GATEWAY),Json(json!({"error":self.code()}))).into_response()
    }
}
fn authorize(headers: &axum::http::HeaderMap,expected: &str,other: &str) -> Result<(),SearchError> {
    let value = headers.get(axum::http::header::AUTHORIZATION).and_then(|value| value.to_str().ok()).and_then(|value| value.strip_prefix("Bearer ")).ok_or(SearchError::new(401,"bearer_authentication_required"))?;
    if bool::from(value.as_bytes().ct_eq(expected.as_bytes())) { Ok(()) }
    else if bool::from(value.as_bytes().ct_eq(other.as_bytes())) { Err(SearchError::new(403,"bearer_scope_denied")) }
    else { Err(SearchError::new(401,"bearer_authentication_required")) }
}
async fn runtime_auth(State(state): State<Arc<Inner>>,request: Request<axum::body::Body>,next: Next) -> Response {
    match authorize(request.headers(),&state.runtime_token,&state.admin_token) { Ok(()) => next.run(request).await,Err(error) => error.into_response() }
}
async fn admin_auth(State(state): State<Arc<Inner>>,request: Request<axum::body::Body>,next: Next) -> Response {
    match authorize(request.headers(),&state.admin_token,&state.runtime_token) { Ok(()) => next.run(request).await,Err(error) => error.into_response() }
}
fn body<T>(value: Result<Json<T>,JsonRejection>) -> Result<T,SearchError> {
    value.map(|Json(value)| value).map_err(|error| if error.status() == StatusCode::PAYLOAD_TOO_LARGE { SearchError::new(413,"request_too_large") } else { SearchError::new(400,"invalid_json_request") })
}
fn vault_error() -> SearchError { SearchError::new(502,"private_storage_unavailable") }
async fn runtime_services(State(state): State<Arc<Inner>>) -> Result<Json<Vec<ServiceView>>,SearchError> {
    let vault = state.vault.lock().map_err(|_| vault_error())?;
    Ok(Json(vault.services.values().filter(|service| service.enabled).map(Service::view).collect()))
}
async fn admin_services(State(state): State<Arc<Inner>>) -> Result<Json<Vec<ServiceView>>,SearchError> {
    let vault = state.vault.lock().map_err(|_| vault_error())?;
    Ok(Json(vault.services.values().map(Service::view).collect()))
}
async fn create_service(State(state): State<Arc<Inner>>,value: Result<Json<ServiceWrite>,JsonRejection>) -> Result<(StatusCode,Json<ServiceView>),SearchError> {
    let value = body(value)?;
    let mut service = Service { id:uuid::Uuid::new_v4().to_string(),name:String::new(),kind:value.kind,enabled:false,auth_mode:value.auth_mode,model:None,api_key:None };
    service.apply(value)?;
    let view = service.view();
    let mut vault = state.vault.lock().map_err(|_| vault_error())?;
    if vault.services.len() >= 128 { return Err(SearchError::new(409,"service_limit_reached")); }
    vault.replace(service)?;
    Ok((StatusCode::CREATED,Json(view)))
}
async fn update_service(State(state): State<Arc<Inner>>,Path(id): Path<String>,value: Result<Json<ServiceWrite>,JsonRejection>) -> Result<Json<ServiceView>,SearchError> {
    validation::uuid(&id)?;
    let value = body(value)?;
    let mut vault = state.vault.lock().map_err(|_| vault_error())?;
    let mut service = vault.services.get(&id).cloned().ok_or(SearchError::new(404,"service_not_found"))?;
    service.apply(value)?;
    let view = service.view();
    vault.replace(service)?;
    Ok(Json(view))
}
async fn delete_service(State(state): State<Arc<Inner>>,Path(id): Path<String>) -> Result<StatusCode,SearchError> {
    validation::uuid(&id)?;
    state.vault.lock().map_err(|_| vault_error())?.remove(&id)?;
    Ok(StatusCode::NO_CONTENT)
}
fn selected(state: &Inner,id: &str) -> Result<Service,SearchError> {
    let vault = state.vault.lock().map_err(|_| vault_error())?;
    let service = vault.services.get(id).cloned().ok_or(SearchError::new(404,"service_not_found"))?;
    if !service.enabled { return Err(SearchError::new(409,"service_disabled")); }
    Ok(service)
}
async fn search(State(state): State<Arc<Inner>>,headers: HeaderMap,value: Result<Json<SearchRequest>,JsonRejection>) -> Result<Json<SearchResponse>,SearchError> {
    let request = body(value)?;
    validation::uuid(&request.request_id)?;
    validation::uuid(&request.service_id)?;
    if request.query.trim().is_empty() || request.query.chars().count() > 128 || !(1..=10).contains(&request.limit) { return Err(SearchError::new(400,"invalid_query_or_limit")); }
    let service = selected(&state,&request.service_id)?;
    let credential_headers = headers.get_all(crate::client::ACCOUNT_ACCESS_HEADER);
    let mut credentials = credential_headers.iter();
    let credential = credentials.next();
    if service.auth_mode != AuthMode::SharedChatgpt && credential.is_some() { return Err(SearchError::new(400,"unexpected_account_credential")); }
    let access = if service.auth_mode == AuthMode::SharedChatgpt {
        let value = credential.ok_or(SearchError::new(409,"shared_account_credential_required"))?;
        let value = value.to_str().map_err(|_| SearchError::new(400,"invalid_account_credential"))?;
        if credentials.next().is_some() || !crate::client::valid_account_access(value) { return Err(SearchError::new(400,"invalid_account_credential")); }
        Some(value)
    } else { None };
    let guard = state.requests.register(&request.request_id)?;
    let _slot = state.slots.clone().try_acquire_owned().map_err(|_| SearchError::new(429,"too_many_search_requests"))?;
    let work = provider::search(&state.http,&service,&request.query,request.limit,access);
    tokio::select! {
        biased;
        _ = guard.token.cancelled() => Err(SearchError::new(409,"request_cancelled")),
        result = tokio::time::timeout(Duration::from_secs(120),work) => result.map_err(|_| SearchError::new(504,"provider_timeout"))?.map(Json),
    }
}
async fn fetch(State(state): State<Arc<Inner>>,value: Result<Json<FetchRequest>,JsonRejection>) -> Result<Json<FetchResponse>,SearchError> {
    let request = body(value)?;
    validation::uuid(&request.request_id)?;
    validation::uuid(&request.service_id)?;
    let url = validation::public_url(&request.url)?;
    let guard = state.requests.register(&request.request_id)?;
    let _slot = state.slots.clone().try_acquire_owned().map_err(|_| SearchError::new(429,"too_many_search_requests"))?;
    let work = async {
        let service = selected(&state,&request.service_id)?;
        if service.kind == ServiceKind::Gpt {
            state.reader.fetch(url.as_str()).await
        } else {
            // DeepSeek retrieves the source remotely; only its fixed API endpoint is
            // contacted here. Source DNS belongs to that network, not this host's.
            provider::fetch(&state.http,&service,url.as_str()).await
        }
    };
    tokio::select! {
        biased;
        _ = guard.token.cancelled() => Err(SearchError::new(409,"request_cancelled")),
        result = tokio::time::timeout(Duration::from_secs(120),work) => result.map_err(|_| SearchError::new(504,"provider_timeout"))?.map(Json),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelRequest { request_id: String }
async fn cancel(State(state): State<Arc<Inner>>,value: Result<Json<CancelRequest>,JsonRejection>) -> Result<StatusCode,SearchError> {
    let request = body(value)?;
    validation::uuid(&request.request_id)?;
    state.requests.cancel(&request.request_id)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
