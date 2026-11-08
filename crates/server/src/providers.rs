//! Live provider overrides and tombstones; credentials never enter public metadata.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, RwLock};

use axum::Json;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode};
use nekochat_gateway::Provider;
use nekochat_store::{SettingKey, Store};
use serde::{Deserialize, Serialize};

use crate::api::{ApiError, blocking};
use crate::state::AppState;
use crate::config::Config;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderProtocol {
    ChatCompletions,
    Responses,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderWrite {
    #[serde(default)]
    pub key: Option<String>,
    pub label: String,
    pub base_url: String,
    #[serde(default)]
    pub models_url: Option<String>,
    #[serde(default)]
    pub session_header: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub protocol: Option<ProviderProtocol>,
}

/// JSON failures deliberately omit serde's input excerpts, which can contain a key.
pub struct ProviderBody(pub ProviderWrite);

impl<S: Send + Sync> FromRequest<S> for ProviderBody {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let Json(body) = Json::<ProviderWrite>::from_request(req, state).await
            .map_err(|_| ApiError::bad_request("提供商请求格式无效"))?;
        Ok(Self(body))
    }
}

#[derive(Serialize)]
pub struct ProviderView {
    key: String,
    label: String,
    connected: bool,
    base_url: String,
    models_url: Option<String>,
    session_header: Option<String>,
    managed: bool,
    has_api_key: bool,
    protocol: Option<ProviderProtocol>,
}

impl ProviderView {
    fn new(provider: &Provider, managed: bool, connected: bool, protocol: Option<ProviderProtocol>) -> Self {
        Self {
            key: provider.key.clone(),
            label: provider.label.clone(),
            connected,
            base_url: public_url(&provider.base_url),
            models_url: provider.models_url.as_deref().map(public_url),
            session_header: provider.session_header.clone(),
            managed,
            has_api_key: !provider.api_key.is_empty(),
            protocol,
        }
    }
}

// Borrow credentials when serializing a delta instead of cloning them for a write.
#[derive(Serialize)]
struct StoredProvider<'a> {
    label: &'a str,
    base_url: &'a str,
    models_url: Option<&'a str>,
    session_header: Option<&'a str>,
    api_key: &'a str,
    protocol: Option<ProviderProtocol>,
}

#[derive(Clone)]
struct ManagedProvider {
    provider: Arc<Provider>,
    protocol: Option<ProviderProtocol>,
}

struct Cached {
    active: BTreeMap<String, Arc<Provider>>,
    delta: BTreeMap<String, Option<ManagedProvider>>,
}

fn load_delta(store: &Store) -> Result<BTreeMap<String, Option<ManagedProvider>>, &'static str> {
    let raw = store.get_setting(SettingKey::Providers)
        .map_err(|_| "无法读取已保存的提供商配置")?;
    let stored: BTreeMap<String, Option<ProviderWrite>> = raw
        .map(|raw| serde_json::from_str(&raw).map_err(|_| "已保存的提供商配置格式无效"))
        .transpose()?.unwrap_or_default();
    stored.into_iter().map(|(key, body)| {
        validate_key(&key).map_err(|_| "已保存的提供商配置标识无效")?;
        let provider = body.map(|body| {
            let protocol = body.protocol;
            resolve(body, &key, None, true)
                .map(|provider| ManagedProvider { provider: Arc::new(provider), protocol })
                .map_err(|_| "已保存的提供商配置内容无效")
        }).transpose()?;
        Ok((key, provider))
    }).collect()
}

/// Resolve only TOML providers that have no persisted override or deletion.
/// This keeps replaced credentials independent of obsolete environment variables.
pub fn resolve_configured(config: &Config, store: &Store, env: impl Fn(&str) -> Option<String>) -> anyhow::Result<Vec<Provider>> {
    let mut keys = BTreeSet::new();
    for entry in &config.providers {
        if validate_key(&entry.key).is_err() {
            anyhow::bail!("配置中的提供商标识无效或属于官方账号保留标识");
        }
        if !keys.insert(entry.key.as_str()) {
            anyhow::bail!("配置中存在重复提供商标识");
        }
    }
    let delta = load_delta(store).map_err(anyhow::Error::msg)?;
    config.providers.iter().filter(|entry| !delta.contains_key(&entry.key)).map(|entry| {
        let api_key = env(&entry.api_key_env).filter(|key| !key.is_empty())
            .ok_or_else(|| anyhow::anyhow!("提供商 {} 缺少 API 密钥：环境变量 {} 未设置", entry.key, entry.api_key_env))?;
        Ok(Provider {
            key: entry.key.clone(),
            label: entry.label.clone(),
            base_url: entry.base_url.clone(),
            models_url: entry.models_url.clone(),
            session_header: entry.session_header.clone(),
            api_key,
        })
    }).collect()
}

pub(crate) struct ProviderRegistry {
    cached: RwLock<Cached>,
    // This lock is acquired inside the blocking task. Cancellation of the HTTP
    // handler cannot separate a durable write from publishing its active value.
    mutations: Mutex<()>,
}

impl ProviderRegistry {
    pub(crate) fn new(providers: Vec<Provider>, store: &Store) -> Self {
        let mut active: BTreeMap<_, _> = providers.into_iter()
            .map(|provider| (provider.key.clone(), Arc::new(provider)))
            .collect();
        let delta = load_delta(store).unwrap_or_else(|message| panic!("{message}"));
        for (key, provider) in &delta {
            match provider {
                Some(entry) => { active.insert(key.clone(), entry.provider.clone()); }
                None => { active.remove(key); }
            }
        }
        Self { cached: RwLock::new(Cached { active, delta }), mutations: Mutex::new(()) }
    }

    pub(crate) fn get(&self, key: &str) -> Option<Arc<Provider>> {
        self.cached.read().unwrap_or_else(|poison| poison.into_inner()).active.get(key).cloned()
    }

    pub(crate) fn list(&self) -> Vec<Arc<Provider>> {
        self.cached.read().unwrap_or_else(|poison| poison.into_inner()).active.values().cloned().collect()
    }

    fn views(&self, subscription_connected: bool) -> Vec<ProviderView> {
        let cached = self.cached.read().unwrap_or_else(|poison| poison.into_inner());
        cached.active.values().map(|provider| ProviderView::new(
            provider,
            cached.delta.get(&provider.key).is_some_and(Option::is_some),
            provider.key != crate::codex::PROVIDER_KEY || subscription_connected,
            cached.delta.get(&provider.key).and_then(Option::as_ref).and_then(|entry| entry.protocol),
        )).collect()
    }

    fn persist(store: &Store, delta: &BTreeMap<String, Option<ManagedProvider>>) -> Result<(), ApiError> {
        let values: BTreeMap<_, _> = delta.iter().map(|(key, entry)| {
            (key, entry.as_ref().map(|entry| StoredProvider {
                label: &entry.provider.label,
                base_url: &entry.provider.base_url,
                models_url: entry.provider.models_url.as_deref(),
                session_header: entry.provider.session_header.as_deref(),
                api_key: &entry.provider.api_key,
                protocol: entry.protocol,
            }))
        }).collect();
        let raw = serde_json::to_string(&values).map_err(|_| ApiError::internal())?;
        store.set_setting(SettingKey::Providers, Some(&raw)).map_err(ApiError::from)
    }

    fn save(&self, store: &Store, key: Option<String>, body: ProviderWrite) -> Result<ProviderView, ApiError> {
        let _mutation = self.mutations.lock().unwrap_or_else(|poison| poison.into_inner());
        let creating = key.is_none();
        let key = key.or_else(|| body.key.clone()).ok_or_else(|| ApiError::bad_request("提供商标识不能为空"))?;
        validate_key(&key)?;
        let existing = self.get(&key);
        if creating && existing.is_some() {
            return Err(ApiError::new(StatusCode::CONFLICT, "提供商标识已存在"));
        }
        if !creating && existing.is_none() {
            return Err(ApiError::not_found());
        }
        let mut delta = self.cached.read().unwrap_or_else(|poison| poison.into_inner()).delta.clone();
        let protocol = body.protocol.or_else(|| delta.get(&key).and_then(Option::as_ref).and_then(|entry| entry.protocol));
        let provider = Arc::new(resolve(body, &key, existing.as_deref(), false)?);
        delta.insert(key.clone(), Some(ManagedProvider { provider: provider.clone(), protocol }));
        Self::persist(store, &delta)?;
        let view = ProviderView::new(&provider, true, true, protocol);
        let mut cached = self.cached.write().unwrap_or_else(|poison| poison.into_inner());
        cached.delta = delta;
        cached.active.insert(key, provider);
        Ok(view)
    }

    fn delete(&self, store: &Store, key: String) -> Result<(), ApiError> {
        validate_key(&key)?;
        let _mutation = self.mutations.lock().unwrap_or_else(|poison| poison.into_inner());
        if self.get(&key).is_none() {
            return Err(ApiError::not_found());
        }
        let mut delta = self.cached.read().unwrap_or_else(|poison| poison.into_inner()).delta.clone();
        delta.insert(key.clone(), None);
        Self::persist(store, &delta)?;
        let mut cached = self.cached.write().unwrap_or_else(|poison| poison.into_inner());
        cached.delta = delta;
        cached.active.remove(&key);
        Ok(())
    }
}

fn validate_key(key: &str) -> Result<(), ApiError> {
    if key.is_empty() || matches!(key, "." | "..") || key.trim() != key || key.chars().any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '\\' | '?' | '#')) {
        return Err(ApiError::bad_request("提供商标识无效"));
    }
    if key == crate::codex::PROVIDER_KEY {
        return Err(ApiError::bad_request("ChatGPT 订阅是保留提供商，请使用官方账号连接管理"));
    }
    if key == "discover" {
        return Err(ApiError::bad_request("discover 是模型发现接口保留标识，请使用其他站点标识"));
    }
    Ok(())
}

fn public_url(value: &str) -> String {
    let Ok(mut url) = reqwest::Url::parse(value) else { return String::new(); };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.to_string()
}

fn url(value: String, existing: Option<&str>, stored: bool) -> Result<String, ApiError> {
    let value = value.trim();
    let parsed = reqwest::Url::parse(value).map_err(|_| ApiError::bad_request("提供商地址无效"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none()
        || !stored && (!parsed.username().is_empty() || parsed.password().is_some()
            || parsed.query().is_some() || parsed.fragment().is_some())
    {
        return Err(ApiError::bad_request("提供商地址必须是不含凭据、查询参数或片段的 HTTP(S) 地址，认证请填写 API Key"));
    }
    // Editing redacted metadata must retain an existing private endpoint's authentication.
    if let Some(existing) = existing.filter(|existing| public_url(existing) == parsed.as_str()) {
        return Ok(existing.to_owned());
    }
    Ok(value.to_owned())
}

fn resolve(body: ProviderWrite, key: &str, existing: Option<&Provider>, stored: bool) -> Result<Provider, ApiError> {
    validate_key(key)?;
    if body.key.as_deref().is_some_and(|body_key| body_key != key) {
        return Err(ApiError::bad_request("请求中的提供商标识与地址不一致"));
    }
    let label = body.label.trim().to_owned();
    if label.is_empty() {
        return Err(ApiError::bad_request("提供商名称不能为空"));
    }
    let api_key = body.api_key.as_deref().map(str::trim).filter(|key| !key.is_empty())
        .or_else(|| existing.map(|provider| provider.api_key.as_str()))
        .ok_or_else(|| ApiError::bad_request("新建提供商需要 API 密钥"))?;
    if api_key.is_empty() || HeaderValue::from_str(api_key).is_err() {
        return Err(ApiError::bad_request("API 密钥不能用于 HTTP 认证请求头"));
    }
    let session_header = body.session_header.as_deref().map(str::trim).filter(|header| !header.is_empty())
        .map(|header| {
            HeaderName::from_bytes(header.as_bytes())
                .map_err(|_| ApiError::bad_request("会话请求头名称无效"))?;
            Ok::<_, ApiError>(header.to_owned())
        }).transpose()?;
    Ok(Provider {
        key: key.to_owned(),
        label,
        base_url: url(body.base_url, existing.map(|provider| provider.base_url.as_str()), stored)?,
        models_url: body.models_url.filter(|value| !value.trim().is_empty()).map(|value| url(value, existing.and_then(|provider| provider.models_url.as_deref()), stored)).transpose()?,
        session_header,
        api_key: api_key.to_owned(),
    })
}

/// Validate an unsaved discovery draft, retaining the current credential when omitted.
pub fn resolve_draft(state: &AppState, body: ProviderWrite) -> Result<Provider, ApiError> {
    let key = body.key.clone().ok_or_else(|| ApiError::bad_request("提供商标识不能为空"))?;
    let existing = state.providers.get(&key);
    resolve(body, &key, existing.as_deref(), false)
}

pub async fn list(State(state): State<AppState>) -> Json<Vec<ProviderView>> {
    Json(state.providers.views(state.codex.connected()))
}

pub async fn create(State(state): State<AppState>, ProviderBody(body): ProviderBody) -> Result<(StatusCode, Json<ProviderView>), ApiError> {
    let view = blocking(move || Ok(state.providers.save(&state.store, None, body))).await??;
    Ok((StatusCode::CREATED, Json(view)))
}

pub async fn update(State(state): State<AppState>, Path(key): Path<String>, ProviderBody(body): ProviderBody) -> Result<Json<ProviderView>, ApiError> {
    let view = blocking(move || Ok(state.providers.save(&state.store, Some(key), body))).await??;
    Ok(Json(view))
}

pub async fn delete(State(state): State<AppState>, Path(key): Path<String>) -> Result<StatusCode, ApiError> {
    blocking(move || Ok(state.providers.delete(&state.store, key))).await??;
    Ok(StatusCode::NO_CONTENT)
}
