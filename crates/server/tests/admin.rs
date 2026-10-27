mod common;

use common::{MockScript, PROVIDER_KEY, TestApp, TestCfg, spawn_app, spawn_mock_upstream};
use serde_json::{Value, json};

async fn admin_app(upstream: Option<String>) -> TestApp {
    let app = spawn_app(TestCfg {
        admin_password: Some("ap".into()),
        upstream,
        ..Default::default()
    })
    .await;
    app.login_admin("ap").await;
    app
}

fn spec(provider: &str, upstream_id: &str) -> Value {
    json!({
        "provider": provider,
        "upstream_id": upstream_id,
        "display_name": upstream_id.to_uppercase(),
        "sort": 0,
        "protocol": "chat_completions",
        "reasoning": {"style": "none", "levels": [], "default": null},
        "vision": false,
        "context_window": null,
        "price": {"cached_in": null, "uncached_in": null, "output": null}
    })
}

#[tokio::test]
async fn provider_models_mark_listed() {
    let up = spawn_mock_upstream(MockScript {
        status: 200,
        chunks: vec![],
        models: vec!["m1".into(), "m2".into()],
    })
    .await;
    let app = admin_app(Some(up.base_url.clone())).await;

    let providers: Value = app
        .client
        .get(app.url("/api/admin/providers"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        providers,
        json!([
            {"key": PROVIDER_KEY, "label": "Mock", "connected": true},
            {"key": "chatgpt-subscription", "label": "ChatGPT 订阅（官方账号）", "connected": false}
        ])
    );

    let r = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&spec(PROVIDER_KEY, "m2"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let listed: Value = r.json().await.unwrap();
    let id = listed["id"].as_str().unwrap().to_owned();

    let r = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&spec(PROVIDER_KEY, "m2"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);

    let models: Value = app
        .client
        .get(app.url(&format!("/api/admin/providers/{PROVIDER_KEY}/models")))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rows = models.as_array().unwrap();
    assert!(
        rows.iter()
            .find(|model| model["upstream_id"] == "m1")
            .unwrap()["listed_model_id"]
            .is_null()
    );
    assert_eq!(
        rows.iter()
            .find(|model| model["upstream_id"] == "m2")
            .unwrap()["listed_model_id"],
        id
    );

    let all: Value = app
        .client
        .get(app.url("/api/admin/models"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(all.as_array().unwrap().len(), 1);

    let mut updated = spec(PROVIDER_KEY, "m2");
    updated["display_name"] = json!("Renamed");
    let r = app
        .client
        .put(app.url(&format!("/api/admin/models/{id}")))
        .json(&updated)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let r = app
        .client
        .put(app.url("/api/admin/models/nope"))
        .json(&updated)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = app
        .client
        .delete(app.url(&format!("/api/admin/models/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
}

#[tokio::test]
async fn listing_unknown_provider_is_400() {
    let app = admin_app(None).await;
    let r = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&spec("ghost", "m1"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let body: Value = r.json().await.unwrap();
    assert!(body["error"].is_string());
}

#[tokio::test]
async fn inconsistent_reasoning_specs_are_400() {
    let app = admin_app(Some("http://127.0.0.1:9".into())).await;
    let bad = [
        json!({"style": "none", "levels": [], "default": "high"}),
        json!({"style": "none", "levels": ["high"], "default": null}),
        json!({"style": "effort", "levels": ["low", "high"], "default": "max"}),
        json!({"style": "effort", "levels": [], "default": null}),
        json!({"style": "deepseek", "levels": ["high", "high"], "default": null}),
    ];
    let mut ok = spec(PROVIDER_KEY, "m1");
    let r = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&ok)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for reasoning in bad {
        let mut s = spec(PROVIDER_KEY, "m2");
        s["reasoning"] = reasoning.clone();
        let r = app
            .client
            .post(app.url("/api/admin/models"))
            .json(&s)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400, "create {reasoning}");
        assert!(r.json::<Value>().await.unwrap()["error"].is_string());
        ok["reasoning"] = reasoning.clone();
        let r = app
            .client
            .put(app.url(&format!("/api/admin/models/{id}")))
            .json(&ok)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400, "update {reasoning}");
        assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    }
    let mut s = spec(PROVIDER_KEY, "m3");
    s["reasoning"] = json!({"style": "effort", "levels": ["low", "high"], "default": "high"});
    let r = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&s)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
}

#[tokio::test]
async fn malformed_admin_bodies_are_400_json() {
    let app = admin_app(None).await;
    let r = app
        .client
        .post(app.url("/api/admin/characters"))
        .header("content-type", "application/json")
        .body("{\"name\": ")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"].as_str().unwrap().starts_with("请求格式错误"),
        "{v}"
    );
    // Missing content type and a missing field are format errors too.
    let r = app
        .client
        .post(app.url("/api/admin/characters"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    let r = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&json!({"name": "Neko"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
}

#[tokio::test]
async fn settings_reject_dangling_ids() {
    let app = admin_app(None).await;
    let r = app
        .client
        .put(app.url("/api/admin/settings"))
        .json(&json!({"default_model_id": "nope", "default_character_id": null}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);

    let c: Value = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&json!({"name": "Neko", "display_name":"Neko", "system_prompt": "meow"}))
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
    let s: Value = app
        .client
        .get(app.url("/api/admin/settings"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        s,
        json!({"default_model_id": null, "default_character_id": c["id"]})
    );
}

#[tokio::test]
async fn character_crud_round_trip() {
    let app = admin_app(None).await;
    let r = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&json!({"name": "Neko", "display_name":"Neko", "system_prompt": "meow"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let c: Value = r.json().await.unwrap();
    let id = c["id"].as_str().unwrap();
    assert_eq!(c["name"], "Neko");

    let r = app
        .client
        .put(app.url(&format!("/api/admin/characters/{id}")))
        .json(&json!({"name": "Tama", "display_name":"Tama", "system_prompt": "nya"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let list: Value = app
        .client
        .get(app.url("/api/admin/characters"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["name"], "Tama");
    assert_eq!(list[0]["system_prompt"], "nya");

    let r = app
        .client
        .delete(app.url(&format!("/api/admin/characters/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let list: Value = app
        .client
        .get(app.url("/api/admin/characters"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list, json!([]));
    let r = app
        .client
        .delete(app.url(&format!("/api/admin/characters/{id}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn native_gpt_protocol_and_role_parameter_constraints_are_enforced() {
    let app = admin_app(Some("https://opencode.ai/inference/go/openai/v1".into())).await;
    let mut model = spec(PROVIDER_KEY, "gpt-6-luna");
    let wrong = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&model)
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 400);
    model["protocol"] = json!("responses");
    model["reasoning"] = json!({"style":"effort","levels":["high"],"default":"high"});
    let listed: Value = app
        .client
        .post(app.url("/api/admin/models"))
        .json(&model)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut role = json!({"name":"GPT role","display_name":"GPT 娘","model_id":listed["id"],"parameters":{"seed":42}});
    let rejected = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&role)
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), 400);
    role["parameters"] = json!({"effort":"high","max_output_tokens":512});
    let accepted = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&role)
        .send()
        .await
        .unwrap();
    assert_eq!(accepted.status(), 201);
}
