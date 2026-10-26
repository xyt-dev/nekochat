mod common;

use common::{TestCfg, spawn_app};
use serde_json::json;

fn pw(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[tokio::test]
async fn open_access_when_no_password() {
    let app = spawn_app(TestCfg::default()).await;
    let r = app
        .client
        .get(app.url("/api/conversations"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body, json!([]));
}

#[tokio::test]
async fn access_password_gates_user_api() {
    let app = spawn_app(TestCfg {
        access_password: pw("pw"),
        ..Default::default()
    })
    .await;
    let r = app
        .client
        .get(app.url("/api/conversations"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = app
        .client
        .post(app.url("/api/session"))
        .json(&json!({"password": "bad"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = app
        .client
        .post(app.url("/api/session"))
        .json(&json!({"password": "pw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let cookie = r.headers()["set-cookie"].to_str().unwrap().to_owned();
    assert!(cookie.starts_with("nk_access="), "{cookie}");
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Path=/"));
    let r = app
        .client
        .get(app.url("/api/conversations"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn health_is_always_open() {
    let app = spawn_app(TestCfg {
        access_password: pw("pw"),
        ..Default::default()
    })
    .await;
    let r = app.client.get(app.url("/api/health")).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn access_gate_covers_only_api_paths() {
    let app = spawn_app(TestCfg {
        access_password: pw("pw"),
        ..Default::default()
    })
    .await;
    let r = app.client.get(app.url("/index.html")).send().await.unwrap();
    assert_ne!(r.status(), 401);
    let r = app.client.get(app.url("/api/nope")).send().await.unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn unknown_route_and_wrong_method_are_json() {
    let app = spawn_app(TestCfg::default()).await;
    let r = app.client.get(app.url("/api/nope")).send().await.unwrap();
    assert_eq!(r.status(), 404);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v, json!({"error": "接口不存在"}));
    let r = app
        .client
        .patch(app.url("/api/conversations"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v, json!({"error": "不支持的请求方法"}));
}

async fn status_with_host(app: &common::TestApp, host: &str) -> reqwest::StatusCode {
    app.client
        .get(app.url("/api/conversations"))
        .header("host", host)
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn foreign_host_is_misdirected() {
    let app = spawn_app(TestCfg {
        allowed_hosts: vec!["Chat.Example.com".into()],
        ..Default::default()
    })
    .await;
    let r = app
        .client
        .get(app.url("/api/health"))
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 421);
    let v: serde_json::Value = r.json().await.unwrap();
    assert_eq!(v, json!({"error": "不允许的访问地址"}));
    for host in ["evil.example:8787", "127.0.0.2", "localhost.evil.example"] {
        assert_eq!(status_with_host(&app, host).await, 421, "{host}");
    }
    for host in [
        "localhost:8787",
        "LOCALHOST",
        "127.0.0.1:8787",
        "[::1]:8787",
        "chat.example.com",
        "chat.example.com:443",
    ] {
        assert_eq!(status_with_host(&app, host).await, 200, "{host}");
    }
}

#[tokio::test]
async fn missing_host_is_misdirected() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let app = spawn_app(TestCfg::default()).await;
    let addr = app.base.trim_start_matches("http://");
    let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET /api/health HTTP/1.0\r\n\r\n")
        .await
        .unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(
        text.starts_with("HTTP/1.") && text.contains(" 421 "),
        "{text}"
    );
}

#[tokio::test]
async fn admin_absent_without_admin_password() {
    let app = spawn_app(TestCfg::default()).await;
    let r = app
        .client
        .post(app.url("/api/admin/session"))
        .json(&json!({"password": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = app
        .client
        .get(app.url("/api/admin/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn admin_requires_admin_cookie() {
    let app = spawn_app(TestCfg {
        admin_password: pw("ap"),
        ..Default::default()
    })
    .await;
    let r = app
        .client
        .get(app.url("/api/admin/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    let r = app
        .client
        .post(app.url("/api/admin/session"))
        .json(&json!({"password": "bad"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
    app.login_admin("ap").await;
    let r = app
        .client
        .get(app.url("/api/admin/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn access_cookie_does_not_grant_admin() {
    let app = spawn_app(TestCfg {
        access_password: pw("pw"),
        admin_password: pw("ap"),
        ..Default::default()
    })
    .await;
    let r = app
        .client
        .post(app.url("/api/session"))
        .json(&json!({"password": "pw"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = app
        .client
        .get(app.url("/api/admin/models"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}
