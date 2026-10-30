mod common;

use common::{TestCfg, spawn_app};
use serde_json::Value;

#[tokio::test]
async fn account_removal_is_admin_only_and_rejects_connected_or_unknown_registrations() {
    let app = spawn_app(TestCfg { admin_password: Some("cleanup-admin".into()), shared_account: true, ..Default::default() }).await;
    app.login_admin("cleanup-admin").await;
    let path = app.dir.path().join("chatgpt-account.json");
    let before = std::fs::read(&path).unwrap();
    let endpoint = app.url("/api/admin/chatgpt/accounts/test-client");
    assert_eq!(app.client.delete(&endpoint).send().await.unwrap().status(), 400);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(app.state.codex.connected());
    let anonymous = reqwest::Client::new().delete(&endpoint).send().await.unwrap();
    assert_eq!(anonymous.status(), 401);
    assert_eq!(app.client.delete(app.url("/api/admin/chatgpt/accounts/missing")).send().await.unwrap().status(), 404);
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[tokio::test]
async fn logged_out_registration_removal_persists_and_preserves_the_host_identity() {
    let app = spawn_app(TestCfg { admin_password: Some("cleanup-admin".into()), shared_account: true, ..Default::default() }).await;
    app.login_admin("cleanup-admin").await;
    let path = app.dir.path().join("chatgpt-account.json");
    let before: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let offline = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap()).build().unwrap();
    app.state.codex.logout(&offline).await.unwrap();
    let response = app.client.delete(app.url("/api/admin/chatgpt/accounts/test-client")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    for secret in ["synthetic-shared-access", "synthetic-refresh-never-delegated", "synthetic-id-never-delegated"] {
        assert!(!text.contains(secret));
    }
    let status: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(status["accounts"], serde_json::json!([]));
    assert_eq!(status["active_client_id"], Value::Null);
    assert_eq!(status["connected"], false);
    let after: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(after["host_id"], before["host_id"]);
    assert_eq!(after["welcomed"], before["welcomed"]);
    assert_eq!(after["registrations"], serde_json::json!([]));
    assert_eq!(after["active"], Value::Null);
    let restored = nekochat_server::codex::CodexAccount::new(app.dir.path());
    assert!(!restored.connected());
    assert_eq!(serde_json::to_value(restored.status().await).unwrap()["accounts"], serde_json::json!([]));
}
