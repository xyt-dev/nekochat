//! Server-owned official Sign in with ChatGPT public-client OAuth.
//! Sources: developers.openai.com/siwc/token-sharing-open-source/{sign-in,profiles-and-sessions}.
//! Credentials are never read from another application or serialized into API responses.
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::{
    api::{ApiError, JsonBody},
    model_defaults::ModelDefaults,
    state::AppState,
};
use axum::{
    Json, Router,
    extract::{OriginalUri, State},
    http::StatusCode,
    response::Html,
    routing::get,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use nekochat_gateway::Provider;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

pub const PROVIDER_KEY: &str = "chatgpt-subscription";
const ISSUER: &str = "https://auth.openai.com";
const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const RESOURCE: &str = "https://api.openai.com/v1";
const DIRECT_SCOPE: &str = "chatgpt.tokens.use.direct";
const ATTEMPT_SECONDS: u64 = 600;
type OAuthResult<T> = Result<T, &'static str>;

#[derive(Serialize, Deserialize, Default, Clone)]
struct Saved {
    host_id: String,
    active: Option<String>,
    welcomed: bool,
    registrations: Vec<Registration>,
    #[serde(default)]
    incomplete_clients: Vec<String>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Registration {
    client_id: String,
    subject: String,
    email: Option<String>,
    credential: Option<Credential>,
}
#[derive(Serialize, Deserialize, Clone)]
struct Credential {
    access_token: String,
    refresh_token: String,
    id_token: String,
    scopes: Vec<String>,
    expires_at: u64,
    #[serde(default)]
    earliest_refresh_at: Option<u64>,
}
struct Pending {
    state: String,
    nonce: String,
    verifier: String,
    redirect_uri: String,
    client_id: Option<String>,
    subject: Option<String>,
    expires_at: u64,
    cancel: CancellationToken,
}
struct Inner {
    saved: Saved,
    error: Option<&'static str>,
}
pub struct CodexAccount {
    path: PathBuf,
    connected: AtomicBool,
    inner: tokio::sync::Mutex<Inner>,
    pending: Mutex<Option<Pending>>,
    exchanging: Mutex<Option<CancellationToken>>,
}
#[derive(Serialize)]
pub struct AccountSummary {
    client_id: String,
    label: String,
    connected: bool,
}
#[derive(Serialize)]
pub struct AccountStatus {
    connected: bool,
    active_client_id: Option<String>,
    accounts: Vec<AccountSummary>,
    pending: bool,
    expires_at: Option<u64>,
    error: Option<&'static str>,
    welcome_required: bool,
    usage_url: &'static str,
}
#[derive(Serialize)]
pub struct Authorization {
    authorization_url: String,
    expires_at: u64,
    loopback_available: bool,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    client_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallbackRequest {
    redirect_url: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectRequest {
    client_id: String,
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: u64,
    scope: String,
    token_type: String,
    id_token: Option<String>,
    earliest_refresh_at: Option<u64>,
}
#[derive(Deserialize, Clone)]
struct Identity {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
}
#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
    revocation_endpoint: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random() -> OAuthResult<String> {
    let mut value = [0u8; 32];
    getrandom::fill(&mut value).map_err(|_| "系统随机源不可用")?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}
fn private_read(path: &Path) -> OAuthResult<Option<Saved>> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法安全读取 ChatGPT 凭证文件"),
    };
    let metadata = file.metadata().map_err(|_| "无法读取凭证权限")?;
    if !metadata.is_file()
        || metadata.permissions().mode() & 0o077 != 0
        || metadata.len() > 2 * 1024 * 1024
    {
        return Err("ChatGPT 凭证必须为私密普通文件（0600）");
    }
    let mut data = Vec::new();
    file.read_to_end(&mut data)
        .map_err(|_| "无法读取 ChatGPT 凭证")?;
    let saved: Saved =
        serde_json::from_slice(&data).map_err(|_| "ChatGPT 凭证格式损坏，请修复私密文件")?;
    if !saved.host_id.starts_with("urn:uuid:")
        || uuid::Uuid::parse_str(&saved.host_id[9..]).is_err()
    {
        return Err("ChatGPT 主机标识无效");
    }
    Ok(Some(saved))
}
struct PrivateSaveError {
    message: &'static str,
    replaced: bool,
}
impl From<&'static str> for PrivateSaveError {
    fn from(message: &'static str) -> Self { Self { message, replaced: false } }
}
fn private_save(path: &Path, saved: &Saved) -> OAuthResult<()> {
    private_save_with_commit(path, saved).map_err(|failure| failure.message)
}

fn private_save_with_commit(path: &Path, saved: &Saved) -> Result<(), PrivateSaveError> {
    let parent = path.parent().ok_or("凭证目录无效")?;
    std::fs::create_dir_all(parent).map_err(|_| "无法创建凭证目录")?;
    let temporary = parent.join(format!(".chatgpt-{}.tmp", uuid::Uuid::new_v4()));
    let result: OAuthResult<()> = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| "无法创建私密凭证文件")?;
        serde_json::to_writer(&mut file, saved).map_err(|_| "无法编码 ChatGPT 凭证")?;
        file.flush()
            .and_then(|_| file.sync_all())
            .map_err(|_| "无法保存 ChatGPT 凭证")?;
        std::fs::rename(&temporary, path).map_err(|_| "无法原子替换 ChatGPT 凭证")?;
        Ok(())
    })();
    if let Err(message) = result {
        let _ = std::fs::remove_file(temporary);
        return Err(message.into());
    }
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| PrivateSaveError { message: "无法同步凭证目录", replaced: true })
}
fn has_session(saved: &Saved) -> bool {
    saved.registrations.iter().any(|registration| {
        Some(&registration.client_id) == saved.active.as_ref()
            && registration.credential.as_ref().is_some_and(|credential| {
                credential.scopes.iter().any(|scope| scope == DIRECT_SCOPE)
            })
    })
}
fn callback_fields(url: &str, pending: &Pending, at: u64) -> OAuthResult<(String, String)> {
    if at >= pending.expires_at {
        return Err("授权已过期，请重新开始");
    }
    let url = reqwest::Url::parse(url.trim()).map_err(|_| "请粘贴完整回调 URL")?;
    let expected = reqwest::Url::parse(&pending.redirect_uri).map_err(|_| "回调地址无效")?;
    if url.origin() != expected.origin()
        || url.path() != expected.path()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("回调必须与本次授权的 127.0.0.1 地址完全匹配");
    }
    let mut fields = HashMap::new();
    for (key, value) in url.query_pairs() {
        if fields
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err("回调参数不能重复");
        }
    }
    let state = fields.get("state").ok_or("回调缺少 state")?;
    if !bool::from(subtle::ConstantTimeEq::ct_eq(
        state.as_bytes(),
        pending.state.as_bytes(),
    )) {
        return Err("授权 state 不匹配");
    }
    if fields.contains_key("error") {
        return Err("ChatGPT 授权被拒绝或取消");
    }
    let code = fields
        .remove("code")
        .filter(|code| !code.trim().is_empty())
        .ok_or("回调缺少授权码")?;
    let client = match (&pending.client_id, fields.remove("client_id")) {
        (Some(expected), Some(actual)) if expected != &actual => {
            return Err("回调 client_id 与账号注册不匹配");
        }
        (Some(expected), _) => expected.clone(),
        (None, Some(actual)) if !actual.trim().is_empty() && actual != "dynamic_agent_client" => {
            actual
        }
        _ => return Err("注册回调缺少已签发 client_id"),
    };
    Ok((code, client))
}
fn validate_identity(
    identity: &Identity,
    nonce: Option<&str>,
    subject: Option<&str>,
) -> OAuthResult<()> {
    if identity.sub.is_empty() || subject.is_some_and(|expected| expected != identity.sub) {
        return Err("ChatGPT 账号身份不匹配");
    }
    if nonce.is_some_and(|expected| identity.nonce.as_deref() != Some(expected)) {
        return Err("ChatGPT ID token nonce 不匹配");
    }
    Ok(())
}

impl CodexAccount {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("chatgpt-account.json");
        let (saved, error) = match private_read(&path) {
            Ok(Some(saved)) => (saved, None),
            Ok(None) => (Saved::default(), None),
            Err(message) => (Saved::default(), Some(message)),
        };
        Self {
            path,
            connected: AtomicBool::new(has_session(&saved)),
            inner: tokio::sync::Mutex::new(Inner { saved, error }),
            pending: Mutex::new(None),
            exchanging: Mutex::new(None),
        }
    }
    pub fn connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    pub async fn status(&self) -> AccountStatus {
        let inner = self.inner.lock().await;
        let pending = self.pending.lock().unwrap();
        let attempt = pending
            .as_ref()
            .filter(|attempt| now() < attempt.expires_at && !attempt.cancel.is_cancelled());
        AccountStatus {
            connected: self.connected(),
            active_client_id: inner.saved.active.clone(),
            accounts: inner
                .saved
                .registrations
                .iter()
                .map(|registration| AccountSummary {
                    client_id: registration.client_id.clone(),
                    label: format!(
                        "{} · {}",
                        registration.email.as_deref().unwrap_or("ChatGPT 账号"),
                        registration.client_id
                    ),
                    connected: registration.credential.is_some(),
                })
                .chain(
                    inner
                        .saved
                        .incomplete_clients
                        .iter()
                        .map(|client| AccountSummary {
                            client_id: client.clone(),
                            label: format!("未完成注册 · {client}"),
                            connected: false,
                        }),
                )
                .collect(),
            pending: attempt.is_some(),
            expires_at: attempt.map(|attempt| attempt.expires_at),
            error: inner.error,
            welcome_required: self.connected() && !inner.saved.welcomed,
            usage_url: "https://chatgpt.com/settings/usage",
        }
    }
    pub async fn start(
        self: &Arc<Self>,
        http: &reqwest::Client,
        client_id: Option<String>,
    ) -> OAuthResult<Authorization> {
        self.cancel();
        let mut inner = self.inner.lock().await;
        if inner.error.is_some() && inner.saved.host_id.is_empty() && self.path.exists() {
            return Err("请先修复 ChatGPT 私密凭证文件");
        }
        let selected = client_id.as_ref().and_then(|id| {
            inner
                .saved
                .registrations
                .iter()
                .find(|registration| &registration.client_id == id)
        });
        if client_id
            .as_ref()
            .is_some_and(|id| selected.is_none() && !inner.saved.incomplete_clients.contains(id))
        {
            return Err("找不到此账号注册");
        }
        let subject = selected.map(|registration| registration.subject.clone());
        let email = selected.and_then(|registration| registration.email.clone());
        let request_consent = selected.is_some_and(|registration| {
            registration.credential.as_ref().is_some_and(|credential| {
                !credential.scopes.iter().any(|scope| scope == DIRECT_SCOPE)
            })
        });
        if inner.saved.host_id.is_empty() {
            inner.saved.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
        }
        private_save(&self.path, &inner.saved)?;
        // The official redirect requires 127.0.0.1, not localhost. An ephemeral port avoids
        // sending codes to another application's already-running callback listener.
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "无法启动安全本地回调监听")?;
        let port = listener
            .local_addr()
            .map_err(|_| "无法获取回调端口")?
            .port();
        let redirect_uri = format!("http://127.0.0.1:{port}/auth/callback");
        let state = random()?;
        let nonce = random()?;
        let verifier = random()?;
        let cancel = CancellationToken::new();
        let expires_at = now() + ATTEMPT_SECONDS;
        let mut url = reqwest::Url::parse(AUTHORIZE).map_err(|_| "授权地址无效")?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair(
                "client_id",
                client_id.as_deref().unwrap_or("dynamic_agent_client"),
            );
            if client_id.is_none() {
                query.append_pair("agent_name_hint", "nekochat");
            }
            if let Some(email) = email {
                query.append_pair("login_hint", &email);
            }
            if request_consent {
                query.append_pair("prompt", "consent");
            }
            // Do not send retained ID tokens through the browser-facing API, even as login hints.
            query
                .append_pair("ext_agent_host_id", &inner.saved.host_id)
                .append_pair("response_type", "code")
                .append_pair("redirect_uri", &redirect_uri)
                .append_pair("resource", RESOURCE)
                .append_pair(
                    "scope",
                    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
                )
                .append_pair("state", &state)
                .append_pair("nonce", &nonce)
                .append_pair("code_challenge_method", "S256")
                .append_pair(
                    "code_challenge",
                    &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                );
        }
        *self.pending.lock().unwrap() = Some(Pending {
            state,
            nonce,
            verifier,
            redirect_uri,
            client_id,
            subject,
            expires_at,
            cancel: cancel.clone(),
        });
        inner.error = None;
        drop(inner);
        let account = self.clone();
        let http = http.clone();
        let shutdown = cancel.clone();
        let router = Router::new().route("/auth/callback", get(move |OriginalUri(uri): OriginalUri| {
            let account = account.clone(); let http = http.clone();
            async move {
                let url = format!("http://127.0.0.1:{port}{uri}");
                match account.complete(&http, &url).await {
                    Ok(()) => (StatusCode::OK, [("cache-control", "no-store"), ("referrer-policy", "no-referrer")], Html("官方身份授权已完成。请关闭此窗口并返回 nekochat 管理端查看订阅权限状态。")),
                    Err(message) => {
                        account.inner.lock().await.error = Some(message);
                        (StatusCode::BAD_REQUEST, [("cache-control", "no-store"), ("referrer-policy", "no-referrer")], Html("授权未完成。请返回管理端查看状态或重新开始；不要分享此 URL。"))
                    }
                }
            }
        }));
        let account = self.clone();
        tokio::spawn(async move {
            let server = axum::serve(listener, router).with_graceful_shutdown(async move {
                tokio::select! { _ = shutdown.cancelled() => {}, _ = tokio::time::sleep(Duration::from_secs(ATTEMPT_SECONDS)) => { account.cancel(); } }
            });
            let _ = server.await;
        });
        Ok(Authorization {
            authorization_url: url.into(),
            expires_at,
            loopback_available: true,
        })
    }
    pub fn cancel(&self) {
        if let Some(attempt) = self.pending.lock().unwrap().take() {
            attempt.cancel.cancel();
        }
        if let Some(cancel) = self.exchanging.lock().unwrap().take() {
            cancel.cancel();
        }
    }
    async fn discovery(http: &reqwest::Client) -> OAuthResult<Discovery> {
        let discovery: Discovery = http
            .get(format!("{ISSUER}/.well-known/openid-configuration"))
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| "无法获取 OpenAI 身份配置")?
            .error_for_status()
            .map_err(|_| "OpenAI 身份配置不可用")?
            .json()
            .await
            .map_err(|_| "OpenAI 身份配置格式无效")?;
        for endpoint in [&discovery.jwks_uri, &discovery.revocation_endpoint] {
            let url = reqwest::Url::parse(endpoint).map_err(|_| "OpenAI 身份配置地址无效")?;
            if url.scheme() != "https" || url.host_str() != Some("auth.openai.com") {
                return Err("OpenAI 身份配置地址不受信任");
            }
        }
        if discovery.issuer != ISSUER {
            return Err("OpenAI 身份签发者不匹配");
        }
        Ok(discovery)
    }
    async fn identity(
        http: &reqwest::Client,
        token: &str,
        client: &str,
        nonce: Option<&str>,
        subject: Option<&str>,
    ) -> OAuthResult<Identity> {
        let discovery = Self::discovery(http).await?;
        let keys: JwkSet = http
            .get(discovery.jwks_uri)
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| "无法获取 OpenAI 签名密钥")?
            .error_for_status()
            .map_err(|_| "OpenAI 签名密钥不可用")?
            .json()
            .await
            .map_err(|_| "OpenAI 签名密钥格式无效")?;
        Self::verify_identity(token, &keys, client, nonce, subject)
    }
    fn verify_identity(
        token: &str,
        keys: &JwkSet,
        client: &str,
        nonce: Option<&str>,
        subject: Option<&str>,
    ) -> OAuthResult<Identity> {
        let header = decode_header(token).map_err(|_| "ChatGPT ID token 格式无效")?;
        if header.alg != Algorithm::RS256 {
            return Err("ChatGPT ID token 签名算法无效");
        }
        let key = keys
            .find(
                header
                    .kid
                    .as_deref()
                    .ok_or("ChatGPT ID token 缺少签名标识")?,
            )
            .ok_or("ChatGPT ID token 签名密钥未找到")?;
        let key = DecodingKey::from_jwk(key).map_err(|_| "ChatGPT 签名密钥无效")?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[ISSUER]);
        validation.set_audience(&[client]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        validation.leeway = 0;
        let identity = decode::<Identity>(token, &key, &validation)
            .map_err(|_| "ChatGPT ID token 签名、身份或有效期验证失败")?
            .claims;
        validate_identity(&identity, nonce, subject)?;
        Ok(identity)
    }
    async fn request_token(
        http: &reqwest::Client,
        endpoint: &str,
        fields: &[(&str, &str)],
    ) -> Result<TokenResponse, (&'static str, bool)> {
        let response = http
            .post(endpoint)
            .timeout(Duration::from_secs(30))
            .form(fields)
            .send()
            .await
            .map_err(|_| ("ChatGPT token 请求网络失败，请稍后重试", false))?;
        if !response.status().is_success() {
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let code = body
                .get("error")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    body.pointer("/error/code")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or("");
            let terminal = matches!(
                code,
                "invalid_grant"
                    | "invalid_refresh_token"
                    | "token_expired"
                    | "refresh_token_expired"
                    | "refresh_token_invalidated"
                    | "refresh_token_reused"
            );
            return Err((
                if terminal {
                    "ChatGPT 会话已失效，请重新授权"
                } else if code == "invalid_client" {
                    "ChatGPT client 配置无效，请检查账号注册"
                } else {
                    "ChatGPT token 服务暂不可用，请稍后重试"
                },
                terminal,
            ));
        }
        let token: TokenResponse = response
            .json()
            .await
            .map_err(|_| ("ChatGPT token 响应格式无效", false))?;
        if token.access_token.is_empty()
            || token.refresh_token.is_empty()
            || token.expires_in == 0
            || !token.token_type.eq_ignore_ascii_case("bearer")
        {
            return Err(("ChatGPT token 响应缺少有效凭证", false));
        }
        Ok(token)
    }
    pub async fn complete(&self, http: &reqwest::Client, url: &str) -> OAuthResult<()> {
        let (code, client, attempt) = {
            let mut pending = self.pending.lock().unwrap();
            let attempt = pending
                .as_ref()
                .ok_or("没有待完成授权（可能已使用或取消）")?;
            let parsed = callback_fields(url, attempt, now());
            // A matching state consumes an OAuth denial too; malformed/wrong-state traffic
            // cannot cancel the legitimate pending login.
            let (code, client) = match parsed {
                Ok(fields) => fields,
                Err(message) => {
                    if message == "ChatGPT 授权被拒绝或取消" || message == "授权已过期，请重新开始"
                    {
                        if let Some(attempt) = pending.take() {
                            attempt.cancel.cancel();
                        }
                    }
                    return Err(message);
                }
            };
            let attempt = pending.take().ok_or("授权已使用")?;
            *self.exchanging.lock().unwrap() = Some(attempt.cancel.clone());
            (code, client, attempt)
        };
        let mut inner = self.inner.lock().await;
        let operation = async {
            if attempt.cancel.is_cancelled() { return Err("授权已取消"); }
            if !inner
                .saved
                .registrations
                .iter()
                .any(|registration| registration.client_id == client)
                && !inner.saved.incomplete_clients.contains(&client)
            {
                inner.saved.incomplete_clients.push(client.clone());
                private_save(&self.path, &inner.saved)?;
            }
            let token = Self::request_token(
                http,
                TOKEN,
                &[
                    ("grant_type", "authorization_code"),
                    ("client_id", &client),
                    ("code", &code),
                    ("code_verifier", &attempt.verifier),
                    ("redirect_uri", &attempt.redirect_uri),
                    ("resource", RESOURCE),
                ],
            )
            .await
            .map_err(|(message, _)| message)?;
            let id_token = token
                .id_token
                .as_deref()
                .ok_or("ChatGPT 响应缺少 ID token")?;
            let identity = Self::identity(
                http,
                id_token,
                &client,
                Some(&attempt.nonce),
                attempt.subject.as_deref(),
            )
            .await?;
            let credential = Credential {
                access_token: token.access_token,
                refresh_token: token.refresh_token,
                id_token: id_token.to_owned(),
                scopes: token.scope.split_whitespace().map(str::to_owned).collect(),
                expires_at: now().saturating_add(token.expires_in),
                earliest_refresh_at: token.earliest_refresh_at,
            };
            let mut saved = inner.saved.clone();
            if let Some(registration) = saved
                .registrations
                .iter_mut()
                .find(|registration| registration.client_id == client)
            {
                if registration.subject != identity.sub {
                    return Err("ChatGPT 账号身份不匹配");
                }
                registration.credential = Some(credential);
                registration.email = identity.email;
            } else {
                saved.registrations.push(Registration {
                    client_id: client.clone(),
                    subject: identity.sub,
                    email: identity.email,
                    credential: Some(credential),
                });
            }
            saved
                .incomplete_clients
                .retain(|incomplete| incomplete != &client);
            saved.active = Some(client);
            private_save(&self.path, &saved)?;
            inner.saved = saved;
            Ok(())
        };
        let result = tokio::select! { result = operation => result, _ = attempt.cancel.cancelled() => Err("授权已取消"), _ = tokio::time::sleep(Duration::from_secs(attempt.expires_at.saturating_sub(now()))) => Err("授权已过期，请重新开始") };
        attempt.cancel.cancel();
        self.exchanging.lock().unwrap().take();
        inner.error = result.as_ref().err().copied();
        if result.is_ok() && !has_session(&inner.saved) {
            inner.error = Some("已登录，但尚未允许 ChatGPT 订阅用量；请选此账号重新授权以启用");
        }
        self.connected
            .store(has_session(&inner.saved), Ordering::Release);
        result
    }
    pub async fn access_token(&self, http: &reqwest::Client) -> OAuthResult<String> {
        self.access_token_at(http, TOKEN).await
    }
    async fn access_token_at(&self, http: &reqwest::Client, endpoint: &str) -> OAuthResult<String> {
        // This mutex is the single-flight refresh guard; rotating credentials and expiry
        // are committed together before another request can observe the replacement.
        let mut inner = self.inner.lock().await;
        let active = inner.saved.active.clone().ok_or("尚未连接 ChatGPT 账号")?;
        let index = inner
            .saved
            .registrations
            .iter()
            .position(|registration| registration.client_id == active)
            .ok_or("ChatGPT 注册不存在")?;
        let credential = inner.saved.registrations[index]
            .credential
            .as_ref()
            .ok_or("ChatGPT 已退出，请重新连接")?;
        if !credential.scopes.iter().any(|scope| scope == DIRECT_SCOPE) {
            return Err("此账号未允许 ChatGPT 订阅用量，请重新授权启用");
        }
        let at = now();
        if credential.expires_at > at + 180
            || credential.expires_at > at
                && credential
                    .earliest_refresh_at
                    .is_some_and(|earliest| at < earliest)
        {
            return Ok(credential.access_token.clone());
        }
        if credential
            .earliest_refresh_at
            .is_some_and(|earliest| at < earliest)
        {
            return Err("ChatGPT token 尚未到允许刷新时间，请稍后重试");
        }
        let result = Self::request_token(
            http,
            endpoint,
            &[
                ("grant_type", "refresh_token"),
                ("client_id", &active),
                ("refresh_token", &credential.refresh_token),
                ("resource", RESOURCE),
            ],
        )
        .await;
        let token = match result {
            Ok(token) => token,
            Err((message, terminal)) => {
                inner.error = Some(message);
                if terminal {
                    inner.saved.registrations[index].credential = None;
                    self.connected.store(false, Ordering::Release);
                    private_save(&self.path, &inner.saved)?;
                }
                return Err(message);
            }
        };
        let old = inner.saved.registrations[index]
            .credential
            .as_ref()
            .ok_or("ChatGPT 会话已结束")?;
        let id_token = if let Some(id_token) = token.id_token {
            Self::identity(
                http,
                &id_token,
                &active,
                None,
                Some(&inner.saved.registrations[index].subject),
            )
            .await?;
            id_token
        } else {
            old.id_token.clone()
        };
        let mut saved = inner.saved.clone();
        saved.registrations[index].credential = Some(Credential {
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            id_token,
            scopes: token.scope.split_whitespace().map(str::to_owned).collect(),
            expires_at: now().saturating_add(token.expires_in),
            earliest_refresh_at: token.earliest_refresh_at,
        });
        if let Err(message) = private_save(&self.path, &saved) {
            inner.error = Some(message);
            self.connected.store(false, Ordering::Release);
            return Err(message);
        }
        inner.saved = saved;
        inner.error = None;
        self.connected
            .store(has_session(&inner.saved), Ordering::Release);
        if !has_session(&inner.saved) {
            return Err("账号不再允许 ChatGPT 订阅用量，请重新授权启用");
        }
        Ok(inner.saved.registrations[index]
            .credential
            .as_ref()
            .ok_or("ChatGPT 会话已结束")?
            .access_token
            .clone())
    }
    pub async fn select(&self, client: &str) -> OAuthResult<()> {
        let mut inner = self.inner.lock().await;
        if !inner.saved.registrations.iter().any(|registration| {
            registration.client_id == client && registration.credential.is_some()
        }) {
            return Err("该账号未连接，请先重新授权");
        }
        inner.saved.active = Some(client.into());
        private_save(&self.path, &inner.saved)?;
        self.connected
            .store(has_session(&inner.saved), Ordering::Release);
        inner.error = None;
        Ok(())
    }
    pub async fn acknowledge(&self) -> OAuthResult<()> {
        let mut inner = self.inner.lock().await;
        inner.saved.welcomed = true;
        private_save(&self.path, &inner.saved)
    }
    pub async fn remove_disconnected(&self, client: &str) -> OAuthResult<bool> {
        let mut inner = self.inner.lock().await;
        let registration = inner.saved.registrations.iter().position(|entry| entry.client_id == client);
        let incomplete = inner.saved.incomplete_clients.iter().position(|entry| entry == client);
        if registration.is_none() && incomplete.is_none() { return Ok(false); }
        if registration.is_some_and(|index| inner.saved.registrations[index].credential.is_some()) {
            return Err("请先退出此 ChatGPT 账号，再移除本地记录");
        }
        let pending = self.pending.lock().unwrap();
        let exchanging = self.exchanging.lock().unwrap();
        if pending.as_ref().is_some_and(|attempt| now() < attempt.expires_at && !attempt.cancel.is_cancelled())
            || exchanging.as_ref().is_some_and(|attempt| !attempt.is_cancelled()) {
            return Err("授权正在处理中，请完成或取消后再移除记录");
        }
        let removed_registration = registration.map(|index| (index, inner.saved.registrations.remove(index)));
        let removed_incomplete = incomplete.map(|index| (index, inner.saved.incomplete_clients.remove(index)));
        let active = if inner.saved.active.as_deref() == Some(client) { inner.saved.active.take() } else { None };
        if let Err(failure) = private_save_with_commit(&self.path, &inner.saved) {
            if !failure.replaced {
                if let Some((index, entry)) = removed_registration { inner.saved.registrations.insert(index, entry); }
                if let Some((index, entry)) = removed_incomplete { inner.saved.incomplete_clients.insert(index, entry); }
                if active.is_some() { inner.saved.active = active; }
                return Err(failure.message);
            }
            inner.error = Some("本地记录已移除，但凭证目录同步失败；持久化尚未确认，请检查存储");
        }
        if active.is_some() { self.connected.store(false, Ordering::Release); }
        Ok(true)
    }

    pub async fn logout(&self, http: &reqwest::Client) -> OAuthResult<bool> {
        self.cancel();
        self.connected.store(false, Ordering::Release);
        let mut inner = self.inner.lock().await;
        self.connected.store(false, Ordering::Release);
        let mut revoked = true;
        let active = inner.saved.active.clone();
        if let Some(registration) = inner
            .saved
            .registrations
            .iter_mut()
            .find(|registration| Some(&registration.client_id) == active.as_ref())
        {
            if let Some(credential) = registration.credential.take() {
                revoked = false;
                if let Ok(discovery) = Self::discovery(http).await {
                    // Two bounded attempts while the refresh token is still held locally.
                    for attempt in 0..2 {
                        let response = http
                            .post(&discovery.revocation_endpoint)
                            .timeout(Duration::from_secs(15))
                            .form(&[
                                ("token", credential.refresh_token.as_str()),
                                ("token_type_hint", "refresh_token"),
                                ("client_id", registration.client_id.as_str()),
                            ])
                            .send()
                            .await;
                        if response
                            .as_ref()
                            .is_ok_and(|response| response.status() == StatusCode::OK)
                        {
                            revoked = true;
                            break;
                        }
                        if response
                            .as_ref()
                            .is_ok_and(|response| !response.status().is_server_error())
                        {
                            break;
                        }
                        if attempt == 0 {
                            tokio::time::sleep(Duration::from_millis(400)).await;
                        }
                    }
                }
            }
        }
        private_save(&self.path, &inner.saved)?;
        inner.error = if revoked {
            None
        } else {
            Some("本地已退出，远端撤销尚未确认；请在 ChatGPT 设置中断开此应用")
        };
        Ok(revoked)
    }
}

pub fn provider() -> Provider {
    Provider {
        key: PROVIDER_KEY.into(),
        label: "ChatGPT 订阅（官方账号）".into(),
        base_url: RESOURCE.into(),
        models_url: Some(format!("{RESOURCE}/models")),
        api_key: String::new(),
        session_header: None,
    }
}
fn api_error(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, message)
}
pub async fn status(State(state): State<AppState>) -> Json<AccountStatus> {
    Json(state.codex.status().await)
}
pub async fn start(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<StartRequest>,
) -> Result<Json<Authorization>, ApiError> {
    state
        .codex
        .start(&state.http, request.client_id)
        .await
        .map(Json)
        .map_err(api_error)
}
pub async fn callback(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<CallbackRequest>,
) -> Result<Json<AccountStatus>, ApiError> {
    state
        .codex
        .complete(&state.http, &request.redirect_url)
        .await
        .map_err(api_error)?;
    Ok(Json(state.codex.status().await))
}
pub async fn cancel(State(state): State<AppState>) -> StatusCode {
    state.codex.cancel();
    StatusCode::NO_CONTENT
}
pub async fn logout(State(state): State<AppState>) -> Result<Json<AccountStatus>, ApiError> {
    state.codex.logout(&state.http).await.map_err(api_error)?;
    Ok(Json(state.codex.status().await))
}
pub async fn remove_account(
    State(state): State<AppState>,
    axum::extract::Path(client): axum::extract::Path<String>,
) -> Result<Json<AccountStatus>, ApiError> {
    if !state.codex.remove_disconnected(&client).await.map_err(api_error)? {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "找不到此 ChatGPT 账号记录"));
    }
    Ok(Json(state.codex.status().await))
}
pub async fn select(
    State(state): State<AppState>,
    JsonBody(request): JsonBody<SelectRequest>,
) -> Result<Json<AccountStatus>, ApiError> {
    state
        .codex
        .select(&request.client_id)
        .await
        .map_err(api_error)?;
    Ok(Json(state.codex.status().await))
}
pub async fn acknowledge(State(state): State<AppState>) -> Result<StatusCode, ApiError> {
    state.codex.acknowledge().await.map_err(api_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct CatalogModel {
    pub slug: String,
    pub display_name: String,
    pub visibility: String,
    #[serde(default)]
    pub supported_reasoning_levels: Vec<Effort>,
    pub default_reasoning_level: Option<String>,
    #[serde(default)]
    pub input_modalities: Vec<String>,
    pub context_window: Option<i64>,
}
#[derive(Deserialize)]
pub struct Effort {
    pub effort: String,
}
#[derive(Deserialize)]
struct Catalog {
    models: Vec<CatalogModel>,
}
pub async fn catalog(state: &AppState) -> Result<Vec<CatalogModel>, ApiError> {
    let provider = state.authenticated_provider(PROVIDER_KEY).await?;
    let response = state
        .http
        .get(format!("{RESOURCE}/models"))
        .bearer_auth(provider.api_key)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "获取 ChatGPT 账号模型目录失败"))?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "ChatGPT 模型目录不可用，请检查账号连接与工作区权限",
        ));
    }
    let catalog: Catalog = response
        .json()
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "ChatGPT 返回的模型目录格式无效"))?;
    Ok(catalog
        .models
        .into_iter()
        .filter(|model| model.visibility == "list" && !model.slug.is_empty())
        .collect())
}
impl CatalogModel {
    pub fn defaults(&self) -> ModelDefaults {
        use nekochat_core::{Price, Protocol, ReasoningSpec, ReasoningStyle};
        let levels: Vec<_> = self
            .supported_reasoning_levels
            .iter()
            .map(|effort| effort.effort.clone())
            .filter(|effort| !effort.is_empty())
            .collect();
        let default = self
            .default_reasoning_level
            .clone()
            .filter(|effort| levels.contains(effort));
        ModelDefaults {
            display_name: self.display_name.clone(),
            protocol: Protocol::ChatgptResponses,
            reasoning: ReasoningSpec {
                style: if levels.is_empty() {
                    ReasoningStyle::None
                } else {
                    ReasoningStyle::Effort
                },
                levels,
                default,
            },
            vision: self
                .input_modalities
                .iter()
                .any(|modality| modality == "image"),
            context_window: self.context_window.filter(|value| *value > 0),
            price: Price {
                cached_in: None,
                uncached_in: None,
                output: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending() -> Pending {
        Pending {
            state: "expected-state".into(),
            nonce: "nonce".into(),
            verifier: "verifier".into(),
            redirect_uri: "http://127.0.0.1:12345/auth/callback".into(),
            client_id: None,
            subject: None,
            expires_at: 100,
            cancel: CancellationToken::new(),
        }
    }
    #[test]
    fn callback_requires_exact_origin_state_unique_fields_and_issued_client() {
        let attempt = pending();
        let valid = "http://127.0.0.1:12345/auth/callback?code=code&state=expected-state&client_id=oaiapp_test";
        assert_eq!(
            callback_fields(valid, &attempt, 99).unwrap(),
            ("code".into(), "oaiapp_test".into())
        );
        assert!(callback_fields(valid, &attempt, 100).is_err());
        for url in [
            valid.replace("expected-state", "wrong"),
            valid.replace("127.0.0.1", "localhost"),
            valid.replace(":12345/", ":12346/"),
            valid.replace("oaiapp_test", "dynamic_agent_client"),
            format!("{valid}&state=expected-state"),
            valid.replace("&client_id=oaiapp_test", ""),
            valid.replace("&state=expected-state", ""),
        ] {
            assert!(callback_fields(&url, &attempt, 99).is_err());
        }
        let mut returning = pending();
        returning.client_id = Some("registered".into());
        assert!(callback_fields(valid, &returning, 99).is_err());
    }
    #[test]
    fn identity_rejects_wrong_nonce_or_replaced_account() {
        let identity = Identity {
            sub: "account-one".into(),
            nonce: Some("nonce".into()),
            email: None,
        };
        assert!(validate_identity(&identity, Some("nonce"), Some("account-one")).is_ok());
        assert!(validate_identity(&identity, Some("other"), None).is_err());
        assert!(validate_identity(&identity, Some("nonce"), Some("account-two")).is_err());
    }
    #[tokio::test]
    async fn malformed_callback_does_not_consume_state_but_denial_and_cancel_do() {
        let directory = tempfile::tempdir().unwrap();
        let account = CodexAccount::new(directory.path());
        let mut attempt = pending();
        attempt.expires_at = now() + 60;
        *account.pending.lock().unwrap() = Some(attempt);
        let http = reqwest::Client::new();
        assert!(
            account
                .complete(
                    &http,
                    "http://127.0.0.1:12345/auth/callback?state=wrong&error=access_denied"
                )
                .await
                .is_err()
        );
        assert!(account.pending.lock().unwrap().is_some());
        assert!(
            account
                .complete(
                    &http,
                    "http://127.0.0.1:12345/auth/callback?state=expected-state&error=access_denied"
                )
                .await
                .is_err()
        );
        assert!(account.pending.lock().unwrap().is_none());
        assert!(account.complete(&http, "http://127.0.0.1:12345/auth/callback?state=expected-state&code=replay&client_id=client").await.is_err());
        *account.pending.lock().unwrap() = Some(pending());
        account.cancel();
        assert!(account.pending.lock().unwrap().is_none());
    }
    #[test]
    fn private_credentials_are_atomic_owner_only_and_symlinks_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("account.json");
        let mut saved = Saved {
            host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            ..Default::default()
        };
        private_save(&path, &saved).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        saved.welcomed = true;
        private_save(&path, &saved).unwrap();
        assert!(private_read(&path).unwrap().unwrap().welcomed);
        let link = directory.path().join("symlink.json");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(private_read(&link).is_err());
    }
    #[tokio::test]
    async fn status_never_serializes_credentials_and_logout_preserves_registration_host() {
        let directory = tempfile::tempdir().unwrap();
        let account = CodexAccount::new(directory.path());
        {
            let mut inner = account.inner.lock().await;
            inner.saved.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            inner.saved.active = Some("issued-client".into());
            inner.saved.registrations.push(Registration {
                client_id: "issued-client".into(),
                subject: "subject".into(),
                email: None,
                credential: Some(Credential {
                    access_token: "secret-access".into(),
                    refresh_token: "secret-refresh".into(),
                    id_token: "secret-id".into(),
                    scopes: vec![DIRECT_SCOPE.into()],
                    expires_at: now() + 3600,
                    earliest_refresh_at: None,
                }),
            });
        }
        let status = serde_json::to_string(&account.status().await).unwrap();
        for secret in ["secret-access", "secret-refresh", "secret-id"] {
            assert!(!status.contains(secret));
        }
        // A registration without tokens survives restart and does not become usable.
        account.inner.lock().await.saved.registrations[0].credential = None;
        account.logout(&reqwest::Client::new()).await.unwrap();
        let restored = CodexAccount::new(directory.path());
        assert!(!restored.connected());
        assert_eq!(restored.status().await.accounts.len(), 1);
    }
    #[tokio::test]
    async fn concurrent_expiring_requests_refresh_once_and_persist_the_rotating_pair() {
        use std::sync::atomic::AtomicUsize;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let router = Router::new().route("/token", axum::routing::post(move |axum::Form(fields): axum::Form<HashMap<String, String>>| {
            let counter = counter.clone();
            async move {
                assert_eq!(fields.get("grant_type").unwrap(), "refresh_token");
                assert_eq!(fields.get("client_id").unwrap(), "issued");
                assert_eq!(fields.get("refresh_token").unwrap(), "old-refresh");
                assert_eq!(fields.get("resource").unwrap(), "https://api.openai.com/v1");
                assert!(!fields.contains_key("scope"));
                counter.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({"access_token":"new-access","refresh_token":"new-refresh","expires_in":3600,"token_type":"Bearer","scope":"openid offline_access resource.invoke chatgpt.tokens.use.direct"}))
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let account = CodexAccount::new(directory.path());
        {
            let mut inner = account.inner.lock().await;
            inner.saved.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            inner.saved.active = Some("issued".into());
            inner.saved.registrations.push(Registration {
                client_id: "issued".into(),
                subject: "subject".into(),
                email: None,
                credential: Some(Credential {
                    access_token: "old-access".into(),
                    refresh_token: "old-refresh".into(),
                    id_token: "retained-id".into(),
                    scopes: vec![DIRECT_SCOPE.into()],
                    expires_at: now(),
                    earliest_refresh_at: None,
                }),
            });
        }
        let http = reqwest::Client::new();
        let results = futures_util::future::join_all(
            (0..8).map(|_| account.access_token_at(&http, &endpoint)),
        )
        .await;
        assert!(
            results
                .into_iter()
                .all(|result| result.unwrap() == "new-access")
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let saved = private_read(&account.path).unwrap().unwrap();
        let credential = saved.registrations[0].credential.as_ref().unwrap();
        assert_eq!(credential.access_token, "new-access");
        assert_eq!(credential.refresh_token, "new-refresh");
        assert!(credential.expires_at > now() + 180);
        assert_eq!(credential.id_token, "retained-id");
        server.abort();
    }

    #[tokio::test]
    async fn begin_persists_stable_host_and_pkce_without_exposing_tokens() {
        let directory = tempfile::tempdir().unwrap();
        let account = Arc::new(CodexAccount::new(directory.path()));
        let http = reqwest::Client::new();
        let first = account.start(&http, None).await.unwrap();
        let first_url = reqwest::Url::parse(&first.authorization_url).unwrap();
        let fields: HashMap<_, _> = first_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(fields["client_id"], "dynamic_agent_client");
        assert_eq!(fields["agent_name_hint"], "nekochat");
        assert_eq!(fields["resource"], "https://api.openai.com/v1");
        assert!(!fields.contains_key("id_token_hint"));
        let host = private_read(&account.path).unwrap().unwrap().host_id;
        assert_eq!(fields["ext_agent_host_id"], host);
        {
            let pending = account.pending.lock().unwrap();
            let attempt = pending.as_ref().unwrap();
            assert_eq!(
                fields["code_challenge"],
                URL_SAFE_NO_PAD.encode(Sha256::digest(attempt.verifier.as_bytes()))
            );
            assert_eq!(fields["nonce"], attempt.nonce);
        }
        account.cancel();
        let second = account.start(&http, None).await.unwrap();
        let second_url = reqwest::Url::parse(&second.authorization_url).unwrap();
        let second_fields: HashMap<_, _> = second_url
            .query_pairs()
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(second_fields["ext_agent_host_id"], host);
        assert_ne!(fields["state"], second_fields["state"]);
        assert_ne!(fields["nonce"], second_fields["nonce"]);
        account.cancel();
    }
    #[tokio::test]
    async fn identity_only_grant_never_enables_subscription_access() {
        let router = Router::new().route("/token", axum::routing::post(|| async {
            Json(serde_json::json!({"access_token":"access","refresh_token":"refresh","expires_in":3600,"token_type":"Bearer","scope":"openid email profile"}))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let token = CodexAccount::request_token(
            &reqwest::Client::new(),
            &endpoint,
            &[("grant_type", "authorization_code")],
        )
        .await
        .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let account = CodexAccount::new(directory.path());
        {
            let mut inner = account.inner.lock().await;
            inner.saved.active = Some("issued".into());
            inner.saved.registrations.push(Registration {
                client_id: "issued".into(),
                subject: "verified-subject".into(),
                email: None,
                credential: Some(Credential {
                    access_token: token.access_token,
                    refresh_token: token.refresh_token,
                    id_token: "verified-id".into(),
                    scopes: token.scope.split_whitespace().map(str::to_owned).collect(),
                    expires_at: now() + token.expires_in,
                    earliest_refresh_at: None,
                }),
            });
        }
        assert!(!account.connected());
        assert!(
            account
                .access_token_at(&reqwest::Client::new(), &endpoint)
                .await
                .is_err()
        );
        server.abort();
    }

    #[tokio::test]
    async fn refresh_honors_earliest_time_and_only_terminal_errors_clear_the_session() {
        use std::sync::atomic::AtomicUsize;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let router = Router::new().route(
            "/token",
            axum::routing::post(move || {
                let counter = counter.clone();
                async move {
                    if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        (
                            StatusCode::SERVICE_UNAVAILABLE,
                            Json(serde_json::json!({"error":"temporarily_unavailable"})),
                        )
                    } else {
                        (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({"error":"invalid_grant"})),
                        )
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let directory = tempfile::tempdir().unwrap();
        let account = CodexAccount::new(directory.path());
        {
            let mut inner = account.inner.lock().await;
            inner.saved.host_id = format!("urn:uuid:{}", uuid::Uuid::new_v4());
            inner.saved.active = Some("issued".into());
            inner.saved.registrations.push(Registration {
                client_id: "issued".into(),
                subject: "subject".into(),
                email: None,
                credential: Some(Credential {
                    access_token: "still-valid".into(),
                    refresh_token: "refresh".into(),
                    id_token: "retained".into(),
                    scopes: vec![DIRECT_SCOPE.into()],
                    expires_at: now() + 60,
                    earliest_refresh_at: Some(now() + 300),
                }),
            });
            private_save(&account.path, &inner.saved).unwrap();
        }
        account.connected.store(true, Ordering::Release);
        let http = reqwest::Client::new();
        assert_eq!(
            account.access_token_at(&http, &endpoint).await.unwrap(),
            "still-valid"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        {
            let mut inner = account.inner.lock().await;
            let credential = inner.saved.registrations[0].credential.as_mut().unwrap();
            credential.expires_at = now();
            credential.earliest_refresh_at = None;
            private_save(&account.path, &inner.saved).unwrap();
        }
        assert!(account.access_token_at(&http, &endpoint).await.is_err());
        assert!(account.connected());
        assert!(
            private_read(&account.path).unwrap().unwrap().registrations[0]
                .credential
                .is_some()
        );
        assert!(account.access_token_at(&http, &endpoint).await.is_err());
        assert!(!account.connected());
        assert!(
            private_read(&account.path).unwrap().unwrap().registrations[0]
                .credential
                .is_none()
        );
        assert_eq!(account.status().await.accounts.len(), 1);
        server.abort();
    }

    #[test]
    fn id_token_verifies_signature_issuer_audience_expiry_nonce_and_selected_identity() {
        use rsa::{pkcs1::EncodeRsaPrivateKey, traits::PublicKeyParts};
        let private = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let key =
            jsonwebtoken::EncodingKey::from_rsa_der(private.to_pkcs1_der().unwrap().as_bytes());
        let jwks: JwkSet = serde_json::from_value(serde_json::json!({"keys":[{"kty":"RSA","kid":"test-key","alg":"RS256","use":"sig","n":URL_SAFE_NO_PAD.encode(private.n().to_bytes_be()),"e":URL_SAFE_NO_PAD.encode(private.e().to_bytes_be())}]})).unwrap();
        let header = jsonwebtoken::Header {
            kid: Some("test-key".into()),
            ..jsonwebtoken::Header::new(Algorithm::RS256)
        };
        let claims = serde_json::json!({"iss":"https://auth.openai.com","aud":"issued-client","exp":now()+3600,"sub":"subject","nonce":"nonce"});
        let token = jsonwebtoken::encode(&header, &claims, &key).unwrap();
        assert!(
            CodexAccount::verify_identity(
                &token,
                &jwks,
                "issued-client",
                Some("nonce"),
                Some("subject")
            )
            .is_ok()
        );
        assert!(
            CodexAccount::verify_identity(
                &token,
                &jwks,
                "other-client",
                Some("nonce"),
                Some("subject")
            )
            .is_err()
        );
        assert!(
            CodexAccount::verify_identity(
                &token,
                &jwks,
                "issued-client",
                Some("wrong"),
                Some("subject")
            )
            .is_err()
        );
        assert!(
            CodexAccount::verify_identity(
                &token,
                &jwks,
                "issued-client",
                Some("nonce"),
                Some("other")
            )
            .is_err()
        );
        for (field, value) in [
            ("iss", serde_json::json!("https://attacker.invalid")),
            ("exp", serde_json::json!(now() - 1)),
        ] {
            let mut invalid = claims.clone();
            invalid[field] = value;
            let token = jsonwebtoken::encode(&header, &invalid, &key).unwrap();
            assert!(
                CodexAccount::verify_identity(
                    &token,
                    &jwks,
                    "issued-client",
                    Some("nonce"),
                    Some("subject")
                )
                .is_err()
            );
        }
        let mut parts: Vec<_> = token.split('.').map(str::to_owned).collect();
        let replacement = if parts[2].starts_with('A') { "B" } else { "A" };
        parts[2].replace_range(..1, replacement);
        assert!(
            CodexAccount::verify_identity(
                &parts.join("."),
                &jwks,
                "issued-client",
                Some("nonce"),
                Some("subject")
            )
            .is_err()
        );
    }
    #[test]
    fn catalog_uses_advertised_capabilities_without_guessed_vision_or_prices() {
        let model: CatalogModel = serde_json::from_str(r#"{"slug":"actual","display_name":"Actual","visibility":"list","supported_reasoning_levels":[{"effort":"high"}],"default_reasoning_level":"high","input_modalities":["text","image"],"context_window":200000}"#).unwrap();
        let defaults = model.defaults();
        assert_eq!(defaults.protocol, nekochat_core::Protocol::ChatgptResponses);
        assert!(defaults.vision);
        assert_eq!(defaults.reasoning.levels, vec!["high"]);
        assert!(defaults.price.output.is_none());
        let unknown: CatalogModel = serde_json::from_str(
            r#"{"slug":"unknown","display_name":"Unknown","visibility":"list"}"#,
        )
        .unwrap();
        assert!(!unknown.defaults().vision);
        assert!(unknown.defaults().reasoning.levels.is_empty());
    }

    fn removal_fixture(directory: &Path) -> CodexAccount {
        let saved = Saved {
            host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
            active: Some("active".into()),
            welcomed: true,
            registrations: vec![
                Registration {
                    client_id: "active".into(), subject: "subject".into(), email: Some("same@example.invalid".into()),
                    credential: Some(Credential {
                        access_token: "retained-access".into(), refresh_token: "retained-refresh".into(), id_token: "retained-id".into(),
                        scopes: vec![DIRECT_SCOPE.into()], expires_at: now() + 3600, earliest_refresh_at: None,
                    }),
                },
                Registration { client_id: "inactive".into(), subject: "subject".into(), email: Some("same@example.invalid".into()), credential: None },
            ],
            incomplete_clients: vec!["incomplete".into()],
        };
        private_save(&directory.join("chatgpt-account.json"), &saved).unwrap();
        CodexAccount::new(directory)
    }

    #[tokio::test]
    async fn removing_inactive_and_incomplete_records_preserves_other_account_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let account = removal_fixture(directory.path());
        let mut expected = private_read(&account.path).unwrap().unwrap();
        expected.registrations.retain(|entry| entry.client_id != "inactive");
        expected.incomplete_clients.clear();
        assert!(account.remove_disconnected("inactive").await.unwrap());
        assert!(account.remove_disconnected("incomplete").await.unwrap());
        let actual = private_read(&account.path).unwrap().unwrap();
        assert_eq!(serde_json::to_value(actual).unwrap(), serde_json::to_value(expected).unwrap());
        assert!(account.connected());
        assert_eq!(account.status().await.active_client_id.as_deref(), Some("active"));
        let restored = CodexAccount::new(directory.path());
        assert!(restored.connected());
        assert_eq!(restored.status().await.accounts.iter().map(|entry| entry.client_id.as_str()).collect::<Vec<_>>(), vec!["active"]);
    }

    #[tokio::test]
    async fn removal_during_pending_or_exchanging_authorization_does_not_cancel_or_change_it() {
        let directory = tempfile::tempdir().unwrap();
        let account = removal_fixture(directory.path());
        let before = std::fs::read(&account.path).unwrap();
        let cancel = CancellationToken::new();
        *account.pending.lock().unwrap() = Some(Pending {
            state: "state".into(), nonce: "nonce".into(), verifier: "verifier".into(),
            redirect_uri: "http://127.0.0.1:1/auth/callback".into(), client_id: Some("inactive".into()), subject: Some("subject".into()),
            expires_at: now() + 600, cancel: cancel.clone(),
        });
        assert!(account.remove_disconnected("inactive").await.is_err());
        assert!(!cancel.is_cancelled());
        assert_eq!(std::fs::read(&account.path).unwrap(), before);
        account.pending.lock().unwrap().take();
        *account.exchanging.lock().unwrap() = Some(cancel.clone());
        assert!(account.remove_disconnected("inactive").await.is_err());
        assert!(!cancel.is_cancelled());
        assert_eq!(std::fs::read(&account.path).unwrap(), before);
    }

    #[tokio::test]
    async fn failed_removal_save_restores_registration_order_and_stale_active_selection() {
        let directory = tempfile::tempdir().unwrap();
        let account = removal_fixture(directory.path());
        {
            let mut inner = account.inner.lock().await;
            inner.saved.active = Some("inactive".into());
            private_save(&account.path, &inner.saved).unwrap();
        }
        account.connected.store(false, Ordering::Release);
        let before = serde_json::to_value(account.status().await).unwrap();
        let original = std::fs::read(&account.path).unwrap();
        let backup = directory.path().join("original.json");
        std::fs::rename(&account.path, &backup).unwrap();
        std::fs::create_dir(&account.path).unwrap();
        assert!(account.remove_disconnected("inactive").await.is_err());
        assert_eq!(serde_json::to_value(account.status().await).unwrap(), before);
        assert_eq!(std::fs::read(backup).unwrap(), original);
    }

    #[tokio::test]
    async fn cleanup_does_not_reconnect_another_account_after_refresh_persistence_failure() {
        let directory = tempfile::tempdir().unwrap();
        let account = removal_fixture(directory.path());
        account.connected.store(false, Ordering::Release);
        account.inner.lock().await.error = Some("无法保存轮换凭证");
        assert!(account.remove_disconnected("inactive").await.unwrap());
        assert!(!account.connected());
        let status = account.status().await;
        assert_eq!(status.active_client_id.as_deref(), Some("active"));
        assert_eq!(status.error, Some("无法保存轮换凭证"));
    }

    #[tokio::test]
    async fn cleanup_does_not_resurrect_a_registration_from_a_canceled_queued_callback() {
        let directory = tempfile::tempdir().unwrap();
        let account = removal_fixture(directory.path());
        let cancel = CancellationToken::new();
        *account.pending.lock().unwrap() = Some(Pending {
            state: "state".into(), nonce: "nonce".into(), verifier: "verifier".into(),
            redirect_uri: "http://127.0.0.1:1/auth/callback".into(), client_id: Some("inactive".into()), subject: Some("subject".into()),
            expires_at: now() + 600, cancel: cancel.clone(),
        });
        let http = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap()).build().unwrap();
        let inner = account.inner.lock().await;
        let mut removal = Box::pin(account.remove_disconnected("inactive"));
        assert!(futures_util::poll!(removal.as_mut()).is_pending());
        let mut callback = Box::pin(account.complete(&http, "http://127.0.0.1:1/auth/callback?code=cancelled-code&state=state&client_id=inactive"));
        assert!(futures_util::poll!(callback.as_mut()).is_pending());
        account.cancel();
        assert!(cancel.is_cancelled());
        drop(inner);
        assert!(removal.await.unwrap());
        assert!(callback.await.is_err());
        let saved = private_read(&account.path).unwrap().unwrap();
        assert!(!saved.registrations.iter().any(|entry| entry.client_id == "inactive"));
        assert!(!saved.incomplete_clients.iter().any(|entry| entry == "inactive"));
        assert!(account.connected());
    }
}
