mod common;

use std::collections::BTreeSet;

use common::{PROVIDER_KEY, TestApp, TestCfg, spawn_app};
use nekochat_store::{Node, NodeRole, NodeStatus};
use serde_json::{Value, json};

async fn app() -> TestApp {
    let app = spawn_app(TestCfg {
        admin_password: Some("ap".into()),
        upstream: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    })
    .await;
    app.login_admin("ap").await;
    app
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

async fn create_conv(app: &TestApp) -> String {
    let r = app
        .client
        .post(app.url("/api/conversations"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        keys(&v),
        ["id", "title", "updated_at"].map(String::from).into()
    );
    v["id"].as_str().unwrap().to_owned()
}

fn node(conv: &str, id: &str, parent: Option<&str>, created_at: i64) -> Node {
    Node {
        id: id.into(),
        conversation_id: conv.into(),
        parent_id: parent.map(Into::into),
        role: NodeRole::User,
        content: "hi".into(),
        reasoning: None,
        model_id: None,
        model_label: None,
        effort: None,
        status: NodeStatus::Complete,
        metrics: None,
        created_at,
    }
}

#[tokio::test]
async fn models_endpoint_exposes_only_picker_fields() {
    let app = app().await;
    let r = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&json!({
            "provider": PROVIDER_KEY,
            "upstream_id": "deepseek-v4-pro",
            "display_name": "DeepSeek V4 Pro",
            "sort": 0,
            "protocol": "chat_completions",
            "reasoning": {"style": "deepseek", "levels": ["high"], "default": "high"},
            "vision": false,
            "context_window": 128000,
            "price": {"cached_in": 0.1, "uncached_in": 1.0, "output": 2.0}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);

    let v: Value = app
        .client
        .get(app.url("/api/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["default_model_id"], Value::Null);
    let m = &v["models"][0];
    assert_eq!(
        keys(m),
        [
            "id",
            "display_name",
            "provider_label",
            "label",
            "reasoning",
            "vision"
        ]
        .map(String::from)
        .into()
    );
    assert_eq!(
        keys(&m["reasoning"]),
        ["levels", "default"].map(String::from).into()
    );
    assert_eq!(m["label"], "DeepSeek V4 Pro · Mock");
    assert_eq!(m["reasoning"]["levels"], json!(["high"]));
}

#[tokio::test]
async fn conversation_view_has_no_character_data() {
    let app = app().await;
    let c: Value = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&json!({"name": "Neko", "system_prompt": "SECRET-PROMPT"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let r = app
        .client
        .put(app.url("/api/admin/settings"))
        .json(&json!({"default_model_id": null, "default_character_id": c["id"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);

    let id = create_conv(&app).await;
    let conv = app.store.get_conversation(&id).unwrap().unwrap();
    assert_eq!(conv.character_id.as_deref(), c["id"].as_str());
    app.store.insert_node(&node(&id, "n1", None, 1)).unwrap();

    let text = app
        .client
        .get(app.url(&format!("/api/conversations/{id}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains("SECRET-PROMPT"));
    assert!(!text.contains("character"));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        keys(&v),
        ["id", "title", "leaf_id", "nodes"].map(String::from).into()
    );
    assert_eq!(
        keys(&v["nodes"][0]),
        [
            "id",
            "parent_id",
            "role",
            "content",
            "reasoning",
            "model_label",
            "effort",
            "status",
            "metrics",
            "created_at"
        ]
        .map(String::from)
        .into()
    );
}

#[tokio::test]
async fn leaf_descend_picks_newest_branch() {
    let app = app().await;
    let id = create_conv(&app).await;
    let s = &app.store;
    s.insert_node(&node(&id, "a", None, 1)).unwrap();
    s.insert_node(&node(&id, "b1", Some("a"), 2)).unwrap();
    s.insert_node(&node(&id, "b2", Some("a"), 3)).unwrap();
    s.insert_node(&node(&id, "c2", Some("b2"), 4)).unwrap();

    let put = |descend: bool| {
        app.client
            .put(app.url(&format!("/api/conversations/{id}/leaf")))
            .json(&json!({"node_id": "a", "descend": descend}))
            .send()
    };
    let v: Value = put(true).await.unwrap().json().await.unwrap();
    assert_eq!(v, json!({"leaf_id": "c2"}));
    assert_eq!(
        s.get_conversation(&id).unwrap().unwrap().leaf_id.as_deref(),
        Some("c2")
    );
    let v: Value = put(false).await.unwrap().json().await.unwrap();
    assert_eq!(v, json!({"leaf_id": "a"}));
}

#[tokio::test]
async fn leaf_to_foreign_node_is_404() {
    let app = app().await;
    let a = create_conv(&app).await;
    let b = create_conv(&app).await;
    app.store.insert_node(&node(&b, "x", None, 1)).unwrap();
    for descend in [true, false] {
        let r = app
            .client
            .put(app.url(&format!("/api/conversations/{a}/leaf")))
            .json(&json!({"node_id": "x", "descend": descend}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404);
    }
}

#[tokio::test]
async fn delete_then_get_is_404() {
    let app = app().await;
    let id = create_conv(&app).await;
    app.store.insert_node(&node(&id, "n", None, 1)).unwrap();
    let r = app
        .client
        .post(app.url(&format!("/api/conversations/{id}/clear")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(app.store.nodes(&id).unwrap().is_empty());

    let r = app
        .client
        .delete(app.url(&format!("/api/conversations/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let r = app
        .client
        .get(app.url(&format!("/api/conversations/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = app
        .client
        .delete(app.url(&format!("/api/conversations/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}
