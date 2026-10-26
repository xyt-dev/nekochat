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
