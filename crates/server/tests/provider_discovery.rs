mod common;

use axum::{Json, Router, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, routing::get};
use common::{TestCfg, spawn_app};
use serde_json::{Value, json};

async fn catalog(headers: HeaderMap) -> Response {
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer draft-private-key") {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"models": {
        "gateway-alias": {
            "id": "canonical-vendor-id", "name": "Gateway model", "context_length": 256000,
            "architecture": {"input_modalities": ["text", "image"]},
            "pricing": {"prompt":"0.000002", "completion":"0.000008", "input_cache_read":"0.0000005"}
        },
        "unknown-price": {"id":"unknown-price"}
    }})).into_response()
}

#[tokio::test]
async fn draft_discovery_preserves_alias_and_prices_without_saving_configuration() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/nested/v1", listener.local_addr().unwrap());
    let upstream = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/nested/v1/models", get(catalog))).await.unwrap();
    });
    let app = spawn_app(TestCfg { admin_password: Some("ap".into()), ..Default::default() }).await;
    app.login_admin("ap").await;
    let draft = json!({"key":"relay", "label":"Relay", "base_url":base, "api_key":"draft-private-key"});
    let response = app.client.post(app.url("/api/admin/providers/discover")).json(&draft).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let models: Value = response.json().await.unwrap();
    let row = models.as_array().unwrap().iter().find(|row| row["upstream_id"] == "gateway-alias").unwrap();
    assert_eq!(row["defaults"]["display_name"], "Gateway model");
    assert_eq!(row["defaults"]["context_window"], 256000);
    assert_eq!(row["defaults"]["vision"], true);
    assert_eq!(row["defaults"]["price"], json!({"cached_in":0.5,"uncached_in":2.0,"output":8.0}));
    assert_eq!(row["price_source"], format!("{base}/models"));
    assert!(row["fetched_at_ms"].as_i64().unwrap() > 0);
    let unpriced = models.as_array().unwrap().iter().find(|row| row["upstream_id"] == "unknown-price").unwrap();
    assert_eq!(unpriced["defaults"]["price"], json!({"cached_in":null,"uncached_in":null,"output":null}));
    assert!(!models.to_string().contains("draft-private-key"));
    assert!(app.state.provider("relay").is_none());
    assert!(app.store.list_models().unwrap().is_empty());

    let saved = app.client.post(app.url("/api/admin/providers")).json(&draft).send().await.unwrap();
    assert_eq!(saved.status(), 201);
    let mut spec = row["defaults"].clone();
    spec["provider"] = json!("relay");
    spec["upstream_id"] = json!("gateway-alias");
    spec["sort"] = json!(0);
    spec["reasoning"] = json!({"style":"effort", "levels":["low","high","custom"], "default":"custom"});
    let response = app.client.post(app.url("/api/admin/models")).json(&spec).send().await.unwrap();
    assert_eq!(response.status(), 201);
    let model: Value = response.json().await.unwrap();
    let id = model["id"].as_str().unwrap();
    let stored = app.store.get_model(id).unwrap().unwrap();
    assert_eq!(stored.spec.context_window, Some(256000));
    assert_eq!(stored.spec.reasoning.default.as_deref(), Some("custom"));
    assert_eq!(stored.spec.price.output, Some(8.0));

    spec["reasoning"] = json!({"style":"effort", "levels":["low","high"], "default":null});
    let response = app.client.put(app.url(&format!("/api/admin/models/{id}"))).json(&spec).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(app.store.get_model(id).unwrap().unwrap().spec.reasoning.levels, vec!["low", "high"]);
    assert_eq!(app.client.delete(app.url(&format!("/api/admin/models/{id}"))).send().await.unwrap().status(), 204);
    assert!(app.store.get_model(id).unwrap().is_none());
    upstream.abort();
}

#[tokio::test]
async fn draft_discovery_requires_admin_and_does_not_echo_malformed_credentials() {
    let app = spawn_app(TestCfg { admin_password: Some("ap".into()), ..Default::default() }).await;
    let body = json!({"key":"relay", "label":"Relay", "base_url":"http://127.0.0.1:9", "api_key":1234567898765_u64});
    assert_eq!(app.client.post(app.url("/api/admin/providers/discover")).json(&body).send().await.unwrap().status(), 401);
    app.login_admin("ap").await;
    let response = app.client.post(app.url("/api/admin/providers/discover")).json(&body).send().await.unwrap();
    assert_eq!(response.status(), 400);
    assert!(!response.text().await.unwrap().contains("1234567898765"));
    assert!(app.state.provider("relay").is_none());
}
