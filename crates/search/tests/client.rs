use axum::{routing::get,Router};
use nekochat_search::{client::SearchClient,server::Engine};
use serde_json::json;
use std::{sync::{Arc,atomic::{AtomicUsize,Ordering}},time::Duration};

#[tokio::test]
async fn consumer_client_drives_real_engine_crud_and_runtime_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = engine.router();
    let server = tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    let runtime = SearchClient::new(&format!("http://{address}"),"runtime-secret",Duration::from_secs(10)).unwrap();
    let admin = SearchClient::new(&format!("http://{address}"),"admin-secret",Duration::from_secs(10)).unwrap();
    let write = json!({"name":"Subscription native search","kind":"gpt","enabled":true,"auth_mode":"shared_chatgpt"});
    let view: nekochat_search::ServiceView = admin.request_json(reqwest::Method::POST,"/v1/admin/services",Some(&write)).await.unwrap();
    assert!(!view.connected);
    assert!(view.supports_fetch);
    let services = runtime.services().await.unwrap();
    assert_eq!(services.len(),1);
    assert_eq!(services[0].id,view.id);
    assert_eq!(admin.services().await.unwrap_err().status(),403);
    runtime.cancel("d673ba88-b8dd-40de-ab5d-c531c23e064b").await.unwrap();
    admin.delete(&format!("/v1/admin/services/{}",view.id)).await.unwrap();
    assert!(runtime.services().await.unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn client_never_follows_redirects_or_returns_upstream_freeform_errors() {
    let hits = Arc::new(AtomicUsize::new(0));
    let sink_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sink_address = sink_listener.local_addr().unwrap();
    let sink_hits = hits.clone();
    let sink = Router::new().fallback(move || { let hits = sink_hits.clone(); async move { hits.fetch_add(1,Ordering::SeqCst); "private" } });
    let sink_server = tokio::spawn(async move { axum::serve(sink_listener,sink).await.unwrap(); });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/v1/services",get(move || async move { (axum::http::StatusCode::FOUND,[("location",format!("http://{sink_address}/"))],"secret-provider-body") }));
    let server = tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    let client = SearchClient::new(&format!("http://{address}"),"private-bearer",Duration::from_secs(10)).unwrap();
    let error = client.services().await.unwrap_err();
    assert_eq!(hits.load(Ordering::SeqCst),0);
    assert!(!error.to_string().contains("secret-provider-body"));
    assert!(!format!("{error:?}").contains("private-bearer"));
    server.abort();
    sink_server.abort();
}

#[tokio::test]
async fn delegated_search_rejects_remote_plaintext_without_exposing_credentials() {
    let request = nekochat_search::SearchRequest {
        request_id:uuid::Uuid::new_v4().to_string(),service_id:uuid::Uuid::new_v4().to_string(),
        query:"test".to_owned(),limit:1,
    };
    for endpoint in ["http://example.com","http://localhost.example.com","http://192.168.1.1","http://127.0.0.1.example.com","http://[2001:db8::1]"] {
        let client = SearchClient::new(endpoint,"engine-token",Duration::from_secs(10)).unwrap();
        let error = client.search_with_account(&request,"never-send-this-access").await.unwrap_err();
        assert_eq!(error.status(),400);
        assert!(!format!("{error:?}").contains("never-send-this-access"));
    }
}

#[tokio::test]
async fn delegated_search_credentials_do_not_become_default_headers() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = engine.router();
    let server = tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    let runtime = SearchClient::new(&format!("http://{address}"),"runtime-secret",Duration::from_secs(10)).unwrap();
    let admin = SearchClient::new(&format!("http://{address}"),"admin-secret",Duration::from_secs(10)).unwrap();
    let shared: nekochat_search::ServiceView = admin.request_json(reqwest::Method::POST,"/v1/admin/services",Some(&json!({"name":"Shared","kind":"gpt","enabled":true,"auth_mode":"shared_chatgpt"}))).await.unwrap();
    let keyed: nekochat_search::ServiceView = admin.request_json(reqwest::Method::POST,"/v1/admin/services",Some(&json!({"name":"Key","kind":"deepseek","enabled":true,"auth_mode":"api_key","api_key":"fake-key"}))).await.unwrap();
    let mut request = nekochat_search::SearchRequest { request_id:uuid::Uuid::new_v4().to_string(),service_id:shared.id,query:"test".to_owned(),limit:1 };
    runtime.cancel(&request.request_id).await.unwrap();
    assert_eq!(runtime.search_with_account(&request,"temporary-account-access").await.unwrap_err().status(),409);
    request.request_id = uuid::Uuid::new_v4().to_string();
    request.service_id = keyed.id;
    runtime.cancel(&request.request_id).await.unwrap();
    assert_eq!(runtime.search_with_account(&request,"temporary-account-access").await.unwrap_err().status(),400);
    assert_eq!(runtime.search(&request).await.unwrap_err().status(),409);
    assert!(!std::fs::read_to_string(directory.path().join("services.json")).unwrap().contains("temporary-account-access"));
    server.abort();
}

#[tokio::test]
async fn ambient_proxy_cannot_intercept_loopback_engine_credentials() {
    // Isolate proxy environment in a child process: process-global env mutation
    // would race unrelated tests and reqwest's proxy initialization.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let proxy_hits = hits.clone();
    let proxy = Router::new().fallback(move || {
        let hits = proxy_hits.clone();
        async move {
            hits.fetch_add(1,Ordering::SeqCst);
            (axum::http::StatusCode::FORBIDDEN,"ambient proxy must not see engine traffic")
        }
    });
    let server = tokio::spawn(async move { axum::serve(listener,proxy).await.unwrap(); });
    let proxy_url = format!("http://{address}");
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact","delegated_search_credentials_do_not_become_default_headers","--nocapture"])
        .env("http_proxy",&proxy_url).env("HTTP_PROXY",&proxy_url)
        .env("https_proxy",&proxy_url).env("HTTPS_PROXY",&proxy_url)
        .env("all_proxy",&proxy_url).env("ALL_PROXY",&proxy_url)
        .env("no_proxy","").env("NO_PROXY","")
        .output().await.unwrap();
    server.abort();
    assert_eq!(hits.load(Ordering::SeqCst),0,"engine credentials must bypass ambient proxies");
    assert!(output.status.success(),"isolated engine credential regression failed: {}",String::from_utf8_lossy(&output.stdout));
}
