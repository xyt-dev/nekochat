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
use nekochat_server::state::{AppState, RunningTurns};
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
    pub access_password: Option<String>,
    pub admin_password: Option<String>,
    /// Base URL of a mock upstream registered as provider `mock`.
    pub upstream: Option<String>,
    pub session_header: Option<String>,
}

pub struct TestApp {
    pub base: String,
    pub client: reqwest::Client,
    pub store: Arc<Store>,
    pub running: RunningTurns,
    pub dir: TempDir,
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
        providers: Vec::new(),
    };
    let state = AppState::new(config, providers, store);
    let store = state.store.clone();
    let running = state.running.clone();
    let addr = serve_router(nekochat_server::app(state)).await;
    TestApp {
        base: format!("http://{addr}"),
        client: reqwest::Client::builder()
            .cookie_store(true)
            .build()
            .unwrap(),
        store,
        running,
        dir,
    }
}
