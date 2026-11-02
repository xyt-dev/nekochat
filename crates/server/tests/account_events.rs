mod common;

use std::time::Duration;

use common::{PROVIDER_KEY, TestApp, TestCfg, spawn_app};
use futures_util::{StreamExt, stream::BoxStream};
use reqwest::{Client, Response};
use serde_json::{Value, json};

struct AccountStream {
    body: BoxStream<'static, reqwest::Result<axum::body::Bytes>>,
    pending: Vec<u8>,
}

impl AccountStream {
    async fn open(app: &TestApp, client: &Client) -> Self {
        let response = client.get(app.url("/api/account/events")).send().await.unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.headers()["content-type"], "text/event-stream");
        Self { body: response.bytes_stream().boxed(), pending: Vec::new() }
    }

    async fn read(&mut self) -> Option<Value> {
        loop {
            if let Some(end) = self.pending.windows(2).position(|w| w == b"\n\n") {
                let block = String::from_utf8(self.pending.drain(..end + 2).collect()).unwrap();
                if block.starts_with(':') { continue; }
                let mut name = None;
                let mut data = None;
                for line in block.lines() {
                    if let Some(value) = line.strip_prefix("event:") { name = Some(value.trim()); }
                    if let Some(value) = line.strip_prefix("data:") { data = Some(value.trim()); }
                }
                assert_eq!(name, Some("sync"));
                return Some(serde_json::from_str(data.unwrap()).unwrap());
            }
            let chunk = self.body.next().await?.unwrap();
            self.pending.extend_from_slice(&chunk);
        }
    }

    async fn expect(&mut self, id: Option<&str>) {
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(3), self.read()).await.unwrap(),
            Some(json!({"conversation_id": id})),
        );
    }
}

fn browser() -> Client {
    Client::builder().cookie_store(true).build().unwrap()
}

async fn register(app: &TestApp, client: &Client, username: &str) -> Value {
    let response = client.post(app.url("/api/account/register"))
        .json(&json!({"username": username, "password": "account-sync-password"}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    response.json().await.unwrap()
}

async fn fixture() -> (TestApp, String) {
    let app = spawn_app(TestCfg {
        anonymous: true,
        admin_password: Some("account-sync-admin".into()),
        upstream: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    }).await;
    let model = app.store.insert_model(&serde_json::from_value(json!({
        "provider": PROVIDER_KEY, "upstream_id": "sync", "display_name": "Sync",
        "sort": 0, "protocol": "chat_completions", "vision": false,
        "reasoning": {"style": "none", "levels": [], "default": null},
        "price": {"cached_in": null, "uncached_in": null, "output": null}
    })).unwrap()).unwrap();
    let character = app.store.create_character(&serde_json::from_value(json!({
        "name": "sync", "display_name": "Sync", "model_id": model.id
    })).unwrap()).unwrap();
    (app, character.id)
}

async fn create(app: &TestApp, client: &Client, character: &str) -> String {
    let response = client.post(app.url("/api/conversations"))
        .json(&json!({"character_id": character})).send().await.unwrap();
    assert_eq!(response.status(), 201);
    response.json::<Value>().await.unwrap()["id"].as_str().unwrap().into()
}

async fn clear(app: &TestApp, client: &Client, id: &str) -> Response {
    client.post(app.url(&format!("/api/conversations/{id}/clear"))).send().await.unwrap()
}

#[tokio::test]
async fn account_events_reconcile_and_deliver_committed_mutations_only_to_the_owner() {
    let (app, character) = fixture().await;
    let alice = browser();
    let alice_id = register(&app, &alice, "sync-alice").await;
    let other_browser = browser();
    assert_eq!(other_browser.post(app.url("/api/account/login"))
        .json(&json!({"username": "sync-alice", "password": "account-sync-password"}))
        .send().await.unwrap().status(), 200);
    let bob = browser();
    register(&app, &bob, "sync-bob").await;
    assert_eq!(browser().get(app.url("/api/account/events")).send().await.unwrap().status(), 401);
    let mut first = AccountStream::open(&app, &alice).await;
    let mut second = AccountStream::open(&app, &other_browser).await;
    let mut foreign = AccountStream::open(&app, &bob).await;
    first.expect(None).await;
    second.expect(None).await;
    foreign.expect(None).await;
    let own = create(&app, &alice, &character).await;
    assert_eq!(alice.post(app.url("/api/conversations"))
        .json(&json!({"character_id": "unavailable-role"})).send().await.unwrap().status(), 400);
    first.expect(Some(&own)).await;
    second.expect(Some(&own)).await;
    assert!(app.store.get_conversation(&own).unwrap().is_some());
    let other = create(&app, &bob, &character).await;
    foreign.expect(Some(&other)).await;
    assert_eq!(clear(&app, &bob, &own).await.status(), 404);
    // If Bob's create or rejected clear escaped filtering, it precedes this expected event.
    assert_eq!(clear(&app, &alice, &own).await.status(), 204);
    first.expect(Some(&own)).await;
    second.expect(Some(&own)).await;
    let snapshot: Value = alice.get(app.url(&format!("/api/conversations/{own}")))
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(snapshot["nodes"], json!([]));
    assert_eq!(snapshot["active_turn_id"], Value::Null);
    let node: nekochat_store::Node = serde_json::from_value(json!({
        "id": "sync-branch", "conversation_id": own, "parent_id": null,
        "role": "user", "content": "branch", "attachments": [], "reasoning": null,
        "model_id": null, "model_label": null, "effort": null, "character_id": null,
        "character_name": null, "character_avatar": null, "status": "complete",
        "metrics": null, "created_at": 1
    })).unwrap();
    app.store.insert_node(&node).unwrap();
    assert_eq!(alice.put(app.url(&format!("/api/conversations/{own}/leaf")))
        .json(&json!({"node_id": node.id, "descend": false})).send().await.unwrap().status(), 200);
    first.expect(Some(&own)).await;
    second.expect(Some(&own)).await;
    assert_eq!(app.store.get_conversation(&own).unwrap().unwrap().leaf_id.as_deref(), Some("sync-branch"));
    assert_eq!(alice.delete(app.url(&format!("/api/conversations/{own}")))
        .send().await.unwrap().status(), 204);
    first.expect(Some(&own)).await;
    second.expect(Some(&own)).await;
    assert!(app.store.get_conversation(&own).unwrap().is_none());
    assert!(!app.state.turn_running(&own));
    assert_eq!(clear(&app, &bob, &other).await.status(), 204);
    foreign.expect(Some(&other)).await;
    drop(second);
    let missed = create(&app, &alice, &character).await;
    let mut reconnected = AccountStream::open(&app, &other_browser).await;
    reconnected.expect(None).await;
    let list: Value = other_browser.get(app.url("/api/conversations"))
        .send().await.unwrap().json().await.unwrap();
    assert!(list.as_array().unwrap().iter().any(|item| item["id"] == missed));
    assert_eq!(app.store.list_user_conversations(alice_id["id"].as_str().unwrap()).unwrap().len(), 1);
}

#[tokio::test]
async fn account_event_observers_close_on_logout_password_disable_and_delete() {
    for action in ["logout", "password", "admin-password", "disable", "delete"] {
        let (app, character) = fixture().await;
        app.login_admin("account-sync-admin").await;
        let alice = browser();
        let user = register(&app, &alice, "revoked-alice").await;
        let other_session = browser();
        assert_eq!(other_session.post(app.url("/api/account/login"))
            .json(&json!({"username": "revoked-alice", "password": "account-sync-password"}))
            .send().await.unwrap().status(), 200);
        let bob = browser();
        register(&app, &bob, "unaffected-bob").await;
        let mut revoked = AccountStream::open(&app, &alice).await;
        let mut same_user = AccountStream::open(&app, &other_session).await;
        let mut unaffected = AccountStream::open(&app, &bob).await;
        revoked.expect(None).await;
        same_user.expect(None).await;
        unaffected.expect(None).await;
        let id = user["id"].as_str().unwrap();
        let response = match action {
            "logout" => alice.delete(app.url("/api/account/session")).send().await.unwrap(),
            "password" => alice.patch(app.url("/api/account"))
                .json(&json!({"current_password": "account-sync-password", "password": "new-sync-password"}))
                .send().await.unwrap(),
            "admin-password" => app.client.patch(app.url(&format!("/api/admin/users/{id}")))
                .json(&json!({"password": "new-sync-password"})).send().await.unwrap(),
            "disable" => app.client.patch(app.url(&format!("/api/admin/users/{id}")))
                .json(&json!({"disabled": true})).send().await.unwrap(),
            "delete" => app.client.delete(app.url(&format!("/api/admin/users/{id}"))).send().await.unwrap(),
            _ => unreachable!(),
        };
        assert!(response.status().is_success(), "{action}: {}", response.status());
        assert_eq!(tokio::time::timeout(Duration::from_secs(3), revoked.read()).await.unwrap(), None, "{action}");
        if action == "logout" {
            let own = create(&app, &other_session, &character).await;
            same_user.expect(Some(&own)).await;
        } else {
            assert_eq!(tokio::time::timeout(Duration::from_secs(3), same_user.read()).await.unwrap(), None, "{action}");
        }
        let other = create(&app, &bob, &character).await;
        unaffected.expect(Some(&other)).await;
    }
}
