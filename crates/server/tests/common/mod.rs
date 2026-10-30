//! Shared integration-test harness: a scripted mock upstream and an in-process app.
#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use nekochat_gateway::Provider;
use nekochat_server::config::Config;
use nekochat_server::state::{AppState, CONNECT_TIMEOUT, RunningTurns};
use nekochat_server::web::WebAssets;
use nekochat_store::Store;
use serde_json::{Value, json};
use tempfile::TempDir;

pub const PROVIDER_KEY: &str = "mock";

#[derive(Clone, Default)]
pub struct MockScript {
    pub status: u16,
    /// (delay before sending, bytes)
    pub chunks: Vec<(Duration, Vec<u8>)>,
    pub models: Vec<String>,
}

impl MockScript {
    /// One `data: <json>\n\n` chunk per event, terminated by `data: [DONE]\n\n`.
    pub fn sse(events: &[Value]) -> MockScript {
        let mut chunks: Vec<(Duration, Vec<u8>)> = events
            .iter()
            .map(|e| (Duration::ZERO, format!("data: {e}\n\n").into_bytes()))
            .collect();
        chunks.push((Duration::ZERO, b"data: [DONE]\n\n".to_vec()));
        MockScript {
            status: 200,
            chunks,
            models: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct RecordedRequest {
    pub headers: HeaderMap,
    pub body: Value,
}

pub struct MockUpstream {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

#[derive(Clone)]
struct MockState {
    script: Arc<MockScript>,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

async fn mock_chat(State(s): State<MockState>, headers: HeaderMap, body: Bytes) -> Response {
    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    s.requests
        .lock()
        .unwrap()
        .push(RecordedRequest { headers, body });
    let chunks = s.script.chunks.clone();
    let stream = futures_util::stream::unfold(chunks.into_iter(), |mut it| async move {
        let (delay, bytes) = it.next()?;
        tokio::time::sleep(delay).await;
        Some((Ok::<_, std::convert::Infallible>(Bytes::from(bytes)), it))
    });
    let status = StatusCode::from_u16(s.script.status).unwrap_or(StatusCode::OK);
    Response::builder()
        .status(status)
        .header("content-type", "text/event-stream")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn mock_models(State(s): State<MockState>) -> impl IntoResponse {
    let data: Vec<Value> = s.script.models.iter().map(|m| json!({"id": m})).collect();
    Json(json!({ "data": data }))
}

pub async fn spawn_mock_upstream(script: MockScript) -> MockUpstream {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = MockState {
        script: Arc::new(script),
        requests: requests.clone(),
    };
    let router = Router::new()
        .route("/chat/completions", post(mock_chat))
        .route("/models", get(mock_models))
        .with_state(state);
    let addr = serve_router(router).await;
    MockUpstream {
        base_url: format!("http://{addr}"),
        requests,
    }
}

async fn serve_router(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

#[derive(Default)]
pub struct TestCfg {
    /// Leaves the account cookie unset for authentication boundary tests.
    pub anonymous: bool,
    pub access_password: Option<String>,
    pub admin_password: Option<String>,
    /// Base URL of a mock upstream registered as provider `mock`.
    pub upstream: Option<String>,
    pub session_header: Option<String>,
    pub allowed_hosts: Vec<String>,
    /// Overrides the upstream read timeout (connect timeout stays the default).
    pub read_timeout: Option<Duration>,
    /// Frontend files to serve instead of the embedded `web/dist`.
    pub web_assets: Option<Arc<dyn WebAssets>>,
    /// Loads an explicitly synthetic, unexpired SIWC grant into this test's private directory.
    pub shared_account: bool,
}

pub struct TestApp {
    pub base: String,
    pub client: reqwest::Client,
    pub store: Arc<Store>,
    pub running: RunningTurns,
    pub dir: TempDir,
    pub state: AppState,
    pub user_id: String,
}

impl TestApp {
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    pub async fn login_admin(&self, password: &str) {
        let r = self
            .client
            .post(self.url("/api/admin/session"))
            .json(&json!({ "password": password }))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 204);
    }
}

pub async fn spawn_app(cfg: TestCfg) -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    if cfg.shared_account {
        let path = dir.path().join("chatgpt-account.json");
        std::fs::write(&path, serde_json::to_vec(&json!({
            "host_id":"urn:uuid:0196f817-1eb0-7000-8000-000000000001","active":"test-client","welcomed":true,
            "registrations":[{
                "client_id":"test-client","subject":"test-subject","email":null,
                "credential":{
                    "access_token":"synthetic-shared-access",
                    "refresh_token":"synthetic-refresh-never-delegated",
                    "id_token":"synthetic-id-never-delegated",
                    "scopes":["chatgpt.tokens.use.direct"],
                    "expires_at":4102444800_u64
                }
            }]
        })).unwrap()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }
    let store = Store::open(&dir.path().join("nekochat.db")).unwrap();
    let providers: Vec<Provider> = cfg
        .upstream
        .iter()
        .map(|base| Provider {
            key: PROVIDER_KEY.into(),
            label: "Mock".into(),
            base_url: base.clone(),
            models_url: None,
            api_key: "test-key".into(),
            session_header: cfg.session_header.clone(),
        })
        .collect();
    let config = Config {
        listen: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().to_path_buf(),
        access_password: cfg.access_password,
        admin_password: cfg.admin_password,
        trusted_proxies: Vec::new(),
        allowed_hosts: cfg.allowed_hosts,
        providers: Vec::new(),
    };
    let state = match cfg.read_timeout {
        Some(read) => AppState::with_timeouts(config, providers, store, CONNECT_TIMEOUT, read),
        None => AppState::new(config, providers, store),
    };
    let store = state.store.clone();
    let running = state.running.clone();
    let service_state = state.clone();
    let router = match cfg.web_assets {
        Some(assets) => nekochat_server::app_with_assets(state, assets),
        None => nekochat_server::app(state),
    };
    let addr = serve_router(router).await;
    let base = format!("http://{addr}");
    let jar = Arc::new(reqwest::cookie::Jar::default());
    let user_id = if cfg.anonymous {
        String::new()
    } else {
        let user = store
            .create_user("fixture-user", "fixture-password-only")
            .unwrap();
        let token = uuid::Uuid::new_v4().to_string();
        store
            .create_user_session(
                &user.id,
                user.credential_revision,
                &nekochat_server::accounts::fingerprint(&token),
                i64::MAX,
            )
            .unwrap();
        jar.add_cookie_str(
            &format!("nk_user={token}; Path=/; HttpOnly"),
            &base.parse().unwrap(),
        );
        user.id.clone()
    };
    TestApp {
        base,
        client: reqwest::Client::builder()
            .cookie_provider(jar)
            .build()
            .unwrap(),
        store,
        running,
        dir,
        state: service_state,
        user_id,
    }
}
