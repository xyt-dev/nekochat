use crate::{FetchRequest, FetchResponse, SearchError, SearchRequest, SearchResponse, ServiceView};
use reqwest::{header::{HeaderMap, HeaderValue, AUTHORIZATION}, Method, Url};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::{fmt, time::Duration};

pub(crate) const ACCOUNT_ACCESS_HEADER: &str = "x-nekochat-openai-access-token";

pub(crate) fn valid_account_access(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32768 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Clones share a connection pool. Only a trusted administrator selects the endpoint.
#[derive(Clone)]
pub struct SearchClient { http: reqwest::Client, base: Url }
impl fmt::Debug for SearchClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.debug_struct("SearchClient").finish_non_exhaustive() }
}
impl SearchClient {
    pub fn new(base_url: &str, token: &str, timeout: Duration) -> Result<Self, SearchError> {
        let base = Url::parse(base_url).map_err(|_| SearchError::new(400,"invalid_engine_url"))?;
        if !matches!(base.scheme(),"http"|"https") || base.host_str().is_none() || !base.username().is_empty() || base.password().is_some() || base.query().is_some() || base.fragment().is_some() || base.path() != "/" {
            return Err(SearchError::new(400,"invalid_engine_url"));
        }
        if token.trim().is_empty() || timeout.is_zero() || timeout > Duration::from_secs(180) { return Err(SearchError::new(400,"invalid_engine_credentials_or_timeout")); }
        let mut bearer = HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| SearchError::new(400,"invalid_engine_credentials"))?;
        bearer.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION,bearer);
        // Engine credentials, including delegated access on loopback HTTP, must
        // never be detoured through process-wide proxy environment settings.
        let http = reqwest::Client::builder().no_proxy().default_headers(headers).retry(reqwest::retry::never()).redirect(reqwest::redirect::Policy::none()).user_agent("nekochat-search-client/0.1.0").timeout(timeout).connect_timeout(Duration::from_secs(10)).build().map_err(|_| SearchError::new(400,"engine_client_configuration_failed"))?;
        Ok(Self { http, base })
    }
    pub async fn services(&self) -> Result<Vec<ServiceView>, SearchError> { self.request_json(Method::GET,"/v1/services",None).await }
    pub async fn search(&self, request: &SearchRequest) -> Result<SearchResponse, SearchError> {
        let body = serde_json::to_value(request).map_err(|_| SearchError::new(400,"invalid_search_request"))?;
        self.request_json(Method::POST,"/v1/search",Some(&body)).await
    }
    /// Supplies caller-owned access only on this request, never on pooled-client defaults.
    pub async fn search_with_account(&self, request: &SearchRequest, access_token: &str) -> Result<SearchResponse, SearchError> {
        let loopback = match self.base.host() {
            Some(url::Host::Domain(host)) => host == "localhost",
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            None => false,
        };
        if self.base.scheme() != "https" && !loopback { return Err(SearchError::new(400,"account_access_requires_secure_transport")); }
        if !valid_account_access(access_token) { return Err(SearchError::new(400,"invalid_account_credential")); }
        let mut credential = HeaderValue::from_str(access_token).map_err(|_| SearchError::new(400,"invalid_account_credential"))?;
        credential.set_sensitive(true);
        let body = serde_json::to_value(request).map_err(|_| SearchError::new(400,"invalid_search_request"))?;
        let response = self.send_with_account(Method::POST,"/v1/search",Some(&body),Some(credential)).await?;
        let bytes = bounded_body(response,2 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes).map_err(|_| SearchError::new(502,"invalid_engine_response"))
    }
    pub async fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, SearchError> {
        let body = serde_json::to_value(request).map_err(|_| SearchError::new(400,"invalid_fetch_request"))?;
        self.request_json(Method::POST,"/v1/fetch",Some(&body)).await
    }
    pub async fn cancel(&self, request_id: &str) -> Result<(), SearchError> {
        self.send(Method::POST,"/v1/cancel",Some(&json!({"request_id":request_id}))).await?;
        Ok(())
    }
    pub async fn request_json<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<&Value>) -> Result<T, SearchError> {
        let response = self.send(method,path,body).await?;
        let bytes = bounded_body(response,2 * 1024 * 1024).await?;
        serde_json::from_slice(&bytes).map_err(|_| SearchError::new(502,"invalid_engine_response"))
    }
    pub async fn delete(&self, path: &str) -> Result<(), SearchError> { self.send(Method::DELETE,path,None).await?; Ok(()) }
    async fn send(&self, method: Method, path: &str, body: Option<&Value>) -> Result<reqwest::Response, SearchError> {
        self.send_with_account(method,path,body,None).await
    }
    async fn send_with_account(&self, method: Method, path: &str, body: Option<&Value>, account_access: Option<HeaderValue>) -> Result<reqwest::Response, SearchError> {
        if !path.starts_with("/v1/") || path.contains("..") || !path.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'/' || c == b'-' || c == b'_') {
            return Err(SearchError::new(400,"invalid_engine_path"));
        }
        let url = self.base.join(path).map_err(|_| SearchError::new(400,"invalid_engine_path"))?;
        let mut request = self.http.request(method,url);
        if let Some(body) = body { request = request.json(body); }
        if let Some(credential) = account_access { request = request.header(ACCOUNT_ACCESS_HEADER,credential); }
        let response = request.send().await.map_err(|error| if error.is_timeout() { SearchError::new(504,"engine_timeout") } else { SearchError::new(502,"engine_unavailable") })?;
        if !response.status().is_success() {
            let status = response.status().as_u16();
            if status >= 500 {
                let bytes = bounded_body(response, 4096).await?;
                if let Ok(body) = serde_json::from_slice::<Value>(&bytes) {
                    let code = match body["error"].as_str() {
                        Some("provider_rejected_request") => "provider_rejected_request",
                        Some("provider_authentication_failed") => "provider_authentication_failed",
                        Some("provider_balance_insufficient") => "provider_balance_insufficient",
                        Some("provider_access_denied") => "provider_access_denied",
                        Some("provider_resource_unavailable") => "provider_resource_unavailable",
                        Some("provider_request_too_large") => "provider_request_too_large",
                        Some("provider_rate_limited") => "provider_rate_limited",
                        Some("provider_timeout") => "provider_timeout",
                        Some("provider_transport_failed") => "provider_transport_failed",
                        Some("provider_failed") => "provider_failed",
                        Some("invalid_provider_response") => "invalid_provider_response",
                        Some("private_storage_unavailable") => "private_storage_unavailable",
                        _ => "invalid_engine_response",
                    };
                    return Err(SearchError::new(status, code));
                }
            }
            return Err(match status {
                400 => SearchError::new(400,"engine_rejected_request"),
                401 => SearchError::new(401,"engine_authentication_required"),
                403 => SearchError::new(403,"engine_scope_denied"),
                404 => SearchError::new(404,"engine_service_not_found"),
                409 => SearchError::new(409,"engine_request_conflict_or_cancelled"),
                413 => SearchError::new(413,"engine_request_too_large"),
                429 => SearchError::new(429,"engine_rate_limited"),
                504 => SearchError::new(504,"engine_timeout"),
                _ => SearchError::new(502,"engine_request_failed"),
            });
        }
        Ok(response)
    }
}

pub(crate) async fn bounded_body(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>,SearchError> {
    if response.content_length().is_some_and(|size| size > max as u64) { return Err(SearchError::new(502,"provider_response_too_large")); }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(SearchError::transport)? {
        if bytes.len().saturating_add(chunk.len()) > max { return Err(SearchError::new(502,"provider_response_too_large")); }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
