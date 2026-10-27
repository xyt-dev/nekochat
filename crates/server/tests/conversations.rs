mod common;

use std::collections::BTreeSet;

use common::{PROVIDER_KEY, TestApp, TestCfg, spawn_app};
use nekochat_store::{Node, NodeRole, NodeStatus, SettingKey};
use serde_json::{Value, json};

async fn app() -> TestApp {
    let app = spawn_app(TestCfg {
        admin_password: Some("ap".into()),
        upstream: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    })
    .await;
    app.login_admin("ap").await;
    let model = app
        .store
        .insert_model(&model_spec(
            PROVIDER_KEY,
            "test",
            json!({"style":"none","levels":[],"default":null}),
        ))
        .unwrap();
    app.store
        .create_character(
            &serde_json::from_value(
                json!({"name":"test","display_name":"test","model_id":model.id}),
            )
            .unwrap(),
        )
        .unwrap();
    app
}

fn keys(v: &Value) -> BTreeSet<String> {
    v.as_object().unwrap().keys().cloned().collect()
}

async fn create_conv(app: &TestApp) -> String {
    let r = app
        .client
        .post(app.url("/api/conversations"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        keys(&v),
        ["id", "title", "updated_at", "character_id"]
            .map(String::from)
            .into()
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
        character_id: None,
        character_name: None,
        status: NodeStatus::Complete,
        metrics: None,
        created_at,
    }
}

#[tokio::test]
async fn characters_endpoint_only_exposes_public_role_fields() {
    let app = app().await;
    let v: Value = app
        .client
        .get(app.url("/api/characters"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        keys(&v),
        ["default_character_id", "characters"]
            .map(String::from)
            .into()
    );
    assert_eq!(
        keys(&v["characters"][0]),
        ["id", "display_name", "background"]
            .map(String::from)
            .into()
    );
    assert_eq!(
        app.client
            .get(app.url("/api/models"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

fn model_spec(provider: &str, upstream_id: &str, reasoning: Value) -> nekochat_store::ModelSpec {
    serde_json::from_value(json!({
        "provider": provider,
        "upstream_id": upstream_id,
        "display_name": upstream_id,
        "sort": 0,
        "protocol": "chat_completions",
        "reasoning": reasoning,
        "vision": false,
        "context_window": null,
        "price": {"cached_in": null, "uncached_in": null, "output": null}
    }))
    .unwrap()
}

async fn picker(app: &TestApp) -> Value {
    app.client
        .get(app.url("/api/characters"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn unavailable_and_unbound_roles_are_not_offered() {
    let app = app().await;
    let ghost = app
        .store
        .insert_model(&model_spec(
            "ghost",
            "g1",
            json!({"style":"none","levels":[],"default":null}),
        ))
        .unwrap();
    let role = app
        .store
        .create_character(
            &serde_json::from_value(
                json!({"name":"ghost","display_name":"ghost","model_id":ghost.id}),
            )
            .unwrap(),
        )
        .unwrap();
    app.store
        .set_setting(SettingKey::DefaultCharacter, Some(&role.id))
        .unwrap();
    app.store
        .create_character(
            &serde_json::from_value(json!({"name":"unbound","display_name":"unbound"})).unwrap(),
        )
        .unwrap();
    let v = picker(&app).await;
    assert_eq!(v["characters"].as_array().unwrap().len(), 1);
    assert_ne!(v["default_character_id"], role.id);
}

#[tokio::test]
async fn conversation_view_exposes_role_snapshot_not_private_configuration() {
    let app = app().await;
    let model_id = app.store.list_models().unwrap()[0].id.clone();
    let c: Value = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&json!({"name": "Neko", "display_name":"Neko", "model_id":model_id, "system_prompt": "SECRET-PROMPT"}))
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
    let mut historical = node(&id, "n1", None, 1);
    historical.model_label = Some("PRIVATE OLD MODEL".into());
    historical.effort = Some("high".into());
    app.store.insert_node(&historical).unwrap();

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
    assert!(!text.contains("PRIVATE OLD MODEL"));
    assert!(!text.contains("model_label"));
    assert!(!text.contains("\"effort\""));
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        keys(&v),
        ["id", "title", "character_id", "leaf_id", "nodes"]
            .map(String::from)
            .into()
    );
    assert_eq!(
        keys(&v["nodes"][0]),
        [
            "id",
            "parent_id",
            "role",
            "content",
            "reasoning",
            "character_id",
            "character_name",
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

#[tokio::test]
async fn node_metrics_hide_cost_fields() {
    let app = app().await;
    let id = create_conv(&app).await;
    let mut n = node(&id, "n", None, 1);
    n.metrics = Some(nekochat_core::Metrics {
        ttft_ms: Some(10),
        total_ms: 100,
        tokens_per_sec: Some(5.0),
        reasoning_ms: Some(7),
        usage: nekochat_core::Usage {
            cached_in: 1,
            uncached_in: 2,
            reasoning: 3,
            output: 4,
            upstream_cost_usd: Some(0.5),
            allowance_usd: Some(0.25),
        },
    });
    app.store.insert_node(&n).unwrap();
    let text = app
        .client
        .get(app.url(&format!("/api/conversations/{id}")))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!text.contains("cost"));
    assert!(text.contains("\"reasoning_ms\":7"));
    assert!(!text.contains("allowance"));
    let v: Value = serde_json::from_str(&text).unwrap();
    let m = &v["nodes"][0]["metrics"];
    assert_eq!(
        keys(m),
        [
            "ttft_ms",
            "total_ms",
            "tokens_per_sec",
            "reasoning_ms",
            "usage"
        ]
        .map(String::from)
        .into()
    );
    assert_eq!(
        m["usage"],
        json!({"cached_in": 1, "uncached_in": 2, "reasoning": 3, "output": 4})
    );
}
