//! Independent, authenticated provider search with caller-owned subscription access.
pub mod client;
pub mod config;
pub mod server;
mod cancel;
mod reader;
mod provider;
mod vault;
mod validation;

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind { Deepseek, Gpt }

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode { ApiKey, SharedChatgpt }

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ServiceView {
    pub id: String,
    pub name: String,
    pub kind: ServiceKind,
    pub enabled: bool,
    pub auth_mode: AuthMode,
    pub model: Option<String>,
    pub has_api_key: bool,
    pub connected: bool,
    pub supports_fetch: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceWrite {
    pub name: String,
    pub kind: ServiceKind,
    pub enabled: bool,
    pub auth_mode: AuthMode,
    #[serde(default)] pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub api_key: Option<String>,
    #[serde(default)] pub clear_api_key: bool,
}
impl fmt::Debug for ServiceWrite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceWrite").field("kind", &self.kind).field("enabled", &self.enabled).field("auth_mode", &self.auth_mode).field("api_key", &"[REDACTED]").finish_non_exhaustive()
    }
}

fn default_limit() -> u32 { 5 }
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub request_id: String,
    pub service_id: String,
    pub query: String,
    #[serde(default = "default_limit")] pub limit: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchSource {
    pub id: String,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub summary: Option<String>,
    pub content: Option<String>,
    pub published_at: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(try_from = "MoneyWire")]
pub struct Money { pub amount: f64, pub currency: String }
#[derive(Deserialize)]
struct MoneyWire { amount: f64, currency: String }
impl TryFrom<MoneyWire> for Money {
    type Error = &'static str;
    fn try_from(value: MoneyWire) -> Result<Self,Self::Error> {
        if !value.amount.is_finite() || value.amount < 0.0 || !matches!(value.currency.as_str(),"CNY"|"USD") { return Err("invalid monetary usage"); }
        Ok(Self { amount:value.amount,currency:value.currency })
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SearchUsage {
    pub requests: u32,
    pub cached_in: u64,
    pub uncached_in: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cost: Option<Money>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SearchResponse {
    pub query: String,
    pub sources: Vec<SearchSource>,
    pub summary: Option<String>,
    pub retrieved_at_ms: i64,
    pub usage: SearchUsage,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchRequest { pub request_id: String, pub service_id: String, pub url: String }
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FetchResponse { pub source: SearchSource, pub retrieved_at_ms: i64, pub usage: SearchUsage }

/// Safe, fixed diagnostics. Neither upstream free-form text nor request bodies are retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchError { status: u16, code: &'static str }
impl SearchError {
    pub(crate) const fn new(status: u16, code: &'static str) -> Self { Self { status, code } }
    pub fn status(&self) -> u16 { self.status }
    pub fn code(&self) -> &'static str { self.code }
    pub(crate) fn transport(error: reqwest::Error) -> Self {
        if error.is_timeout() { Self::new(504,"provider_timeout") } else { Self::new(502,"provider_transport_failed") }
    }
    pub(crate) fn http(status: u16) -> Self {
        match status {
            400 => Self::new(502,"provider_rejected_request"),
            401 => Self::new(502,"provider_authentication_failed"),
            402 => Self::new(502,"provider_balance_insufficient"),
            403 => Self::new(502,"provider_access_denied"),
            404 => Self::new(502,"provider_resource_unavailable"),
            408 | 504 => Self::new(504,"provider_timeout"),
            413 => Self::new(502,"provider_request_too_large"),
            429 => Self::new(429,"provider_rate_limited"),
            _ => Self::new(502,"provider_failed"),
        }
    }
}
impl fmt::Display for SearchError { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.code) } }
impl std::error::Error for SearchError {}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().min(i64::MAX as u128) as i64
}
