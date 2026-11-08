mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{MockScript, PROVIDER_KEY, TestApp, TestCfg, spawn_app, spawn_mock_upstream};
use futures_util::StreamExt;
use nekochat_gateway::Provider;
use nekochat_server::chat::{TurnEvent, start_turn};
use nekochat_server::state::AppState;
use nekochat_store::{SettingKey, Store};
use serde_json::{Value, json};

async fn admin_app(upstream: Option<String>) -> TestApp {
    let app = spawn_app(TestCfg {
        admin_password: Some("admin".into()),
        upstream,
        ..Default::default()
    }).await;
    app.login_admin("admin").await;
    app
}

fn write_body(key: &str, base_url: &str) -> Value {
    json!({"key":key,"label":"Custom provider","base_url":base_url,"api_key":"  private-provider-key  "})
}

async fn save(app: &TestApp, key: Option<&str>, body: &Value) -> reqwest::Response {
    let request = match key {
        Some(key) => app.client.put(app.url(&format!("/api/admin/providers/{key}"))),
        None => app.client.post(app.url("/api/admin/providers")),
    };
    request.json(body).send().await.unwrap()
}

fn reconstruct(app: &TestApp, baseline: Vec<Provider>) -> AppState {
    AppState::new(
        (*app.state.config).clone(),
        baseline,
        Store::open(&app.dir.path().join("nekochat.db")).unwrap(),
    )
}

#[tokio::test]
async fn provider_crud_is_live_persisted_and_never_returns_credentials() {
    let upstream = spawn_mock_upstream(MockScript { status: 200, models: vec!["m1".into()], ..Default::default() }).await;
    let app = admin_app(None).await;
    let mut body = write_body("custom", &format!("{}/v1", upstream.base_url));
    body["models_url"] = json!(format!("{}/models", upstream.base_url));
    body["session_header"] = json!(" x-session-id ");
    let response = save(&app, None, &body).await;
    assert_eq!(response.status(), 201);
    let created: Value = response.json().await.unwrap();
    assert_eq!(created, json!({
        "key":"custom", "label":"Custom provider", "connected":true,
        "base_url":format!("{}/v1", upstream.base_url),
        "models_url":format!("{}/models", upstream.base_url),
        "session_header":"x-session-id", "managed":true, "has_api_key":true, "protocol":null
    }));
    assert!(!created.to_string().contains("private-provider-key"));
    let captured = app.state.provider("custom").unwrap();
    assert_eq!(captured.api_key, "private-provider-key");
    assert!(Arc::ptr_eq(&captured, &app.state.provider("custom").unwrap()));
    let response = app.client.get(app.url("/api/admin/providers/custom/models")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap()[0]["upstream_id"], "m1");
    let listed = app.client.get(app.url("/api/admin/providers")).send().await.unwrap().text().await.unwrap();
    assert!(!listed.contains("private-provider-key"));
    assert!(!listed.contains("\"api_key\""));
    let restored = reconstruct(&app, Vec::new());
    assert_eq!(restored.provider("custom").unwrap().api_key, "private-provider-key");
    assert_eq!(restored.provider("custom").unwrap().base_url, format!("{}/v1", upstream.base_url));
    assert_eq!(save(&app, None, &body).await.status(), 409);
}

#[tokio::test]
async fn provider_update_retains_blank_keys_and_replaces_explicit_keys() {
    let app = admin_app(Some("http://127.0.0.1:9/v1".into())).await;
    let original = app.state.provider(PROVIDER_KEY).unwrap();
    let mut body = json!({"label":"Edited TOML provider","base_url":"https://example.test/custom/v1","api_key":"   "});
    assert_eq!(save(&app, Some(PROVIDER_KEY), &body).await.status(), 200);
    assert_eq!(app.state.provider(PROVIDER_KEY).unwrap().api_key, "test-key");
    body.as_object_mut().unwrap().remove("api_key");
    assert_eq!(save(&app, Some(PROVIDER_KEY), &body).await.status(), 200);
    assert_eq!(app.state.provider(PROVIDER_KEY).unwrap().api_key, "test-key");
    body["key"] = json!("different");
    assert_eq!(save(&app, Some(PROVIDER_KEY), &body).await.status(), 400);
    body["key"] = json!(PROVIDER_KEY);
    body["api_key"] = json!(" replacement-key ");
    assert_eq!(save(&app, Some(PROVIDER_KEY), &body).await.status(), 200);
    assert_eq!(app.state.provider(PROVIDER_KEY).unwrap().api_key, "replacement-key");
    assert_eq!(original.api_key, "test-key");
    assert_eq!(original.base_url, "http://127.0.0.1:9/v1");
    let restored = reconstruct(&app, vec![(*original).clone()]);
    assert_eq!(restored.provider(PROVIDER_KEY).unwrap().api_key, "replacement-key");
    assert_eq!(restored.provider(PROVIDER_KEY).unwrap().base_url, "https://example.test/custom/v1");
}

#[tokio::test]
async fn deleting_config_provider_persists_tombstone_and_can_be_recreated() {
    let app = admin_app(Some("http://127.0.0.1:9".into())).await;
    let baseline = (*app.state.provider(PROVIDER_KEY).unwrap()).clone();
    let response = app.client.delete(app.url(&format!("/api/admin/providers/{PROVIDER_KEY}"))).send().await.unwrap();
    assert_eq!(response.status(), 204);
    assert!(app.state.provider(PROVIDER_KEY).is_none());
    assert!(reconstruct(&app, vec![baseline.clone()]).provider(PROVIDER_KEY).is_none());
    assert_eq!(save(&app, None, &write_body(PROVIDER_KEY, "https://example.test/v2")).await.status(), 201);
    assert_eq!(reconstruct(&app, vec![baseline]).provider(PROVIDER_KEY).unwrap().api_key, "private-provider-key");
}

#[tokio::test]
async fn provider_validation_and_subscription_reservation_do_not_mutate_registry() {
    let app = admin_app(None).await;
    for (field, value) in [
        ("base_url", json!("ftp://example.test/v1")),
        ("base_url", json!("https://username:password@example.test/v1")),
        ("models_url", json!("https://username@example.test/models")),
        ("session_header", json!("bad header")),
        ("api_key", json!("bad\r\nAuthorization: injected")),
        ("api_key", json!("  ")),
        ("protocol", json!("chatgpt_responses")),
        ("protocol", json!("unknown-protocol")),
        ("protocol", json!(42)),
    ] {
        let mut body = write_body("invalid", "https://example.test/v1");
        body[field] = value;
        let response = save(&app, None, &body).await;
        assert_eq!(response.status(), 400, "{field}");
        assert!(!response.text().await.unwrap().contains("injected"));
        assert!(app.state.provider("invalid").is_none());
    }
    let mut malformed = write_body("invalid", "https://example.test/v1");
    malformed["api_key"] = json!(98765432112345_u64);
    let response = save(&app, None, &malformed).await;
    assert_eq!(response.status(), 400);
    assert!(!response.text().await.unwrap().contains("98765432112345"));
    let collision = write_body("discover", "https://example.test/v1");
    let response = save(&app, None, &collision).await;
    assert_eq!(response.status(), 400);
    assert!(response.json::<Value>().await.unwrap()["error"].as_str().unwrap().contains("discover"));
    assert!(app.state.provider("discover").is_none());
    assert!(nekochat_server::providers::resolve_draft(&app.state, serde_json::from_value(collision).unwrap()).is_err());
    let before = app.state.codex.connected();
    let body = write_body("chatgpt-subscription", "https://example.test/v1");
    assert_eq!(save(&app, None, &body).await.status(), 400);
    assert_eq!(save(&app, Some("chatgpt-subscription"), &body).await.status(), 400);
    assert_eq!(app.client.delete(app.url("/api/admin/providers/chatgpt-subscription")).send().await.unwrap().status(), 400);
    assert_eq!(app.state.codex.connected(), before);
    assert!(app.state.provider_list().iter().any(|p| p.key == "chatgpt-subscription"));
    assert!(app.store.get_setting(SettingKey::Providers).unwrap().is_none());
}

#[tokio::test]
async fn deleting_provider_blocks_new_turns_but_preserves_captured_turn_and_history() {
    let mut script = MockScript::sse(&[json!({"choices":[{"delta":{"content":"completed after deletion"}}]})]);
    script.chunks[0].0 = Duration::from_millis(200);
    let upstream = spawn_mock_upstream(script).await;
    let app = admin_app(None).await;
    assert_eq!(save(&app, None, &write_body("custom", &upstream.base_url)).await.status(), 201);
    let model = app.store.insert_model(&serde_json::from_value(json!({
        "provider":"custom","upstream_id":"m1","display_name":"Model","sort":0,
        "protocol":"chat_completions","reasoning":{"style":"none","levels":[],"default":null},
        "vision":false,"context_window":null,"price":{"cached_in":null,"uncached_in":null,"output":null}
    })).unwrap()).unwrap();
    let character = app.store.create_character(&serde_json::from_value(json!({"name":"role","display_name":"Role","model_id":model.id})).unwrap()).unwrap();
    let conversation = app.store.create_owned_conversation(&app.user_id, Some(&character.id)).unwrap();
    let request = || serde_json::from_value(json!({"action":"send","content":"hello","character_id":character.id})).unwrap();
    let mut events = start_turn(app.state.clone(), app.user_id.clone(), conversation.id.clone(), request()).await.unwrap();
    assert_eq!(app.client.delete(app.url("/api/admin/providers/custom")).send().await.unwrap().status(), 204);
    let mut completed = false;
    while let Some(event) = events.next().await {
        if let TurnEvent::Done(node) = event.as_ref() {
            assert_eq!(node.content, "completed after deletion");
            completed = true;
        }
    }
    assert!(completed);
    assert_eq!(upstream.requests.lock().unwrap()[0].headers["authorization"], "Bearer private-provider-key");
    assert!(app.store.get_model(&model.id).unwrap().is_some());
    assert!(app.store.get_character(&character.id).unwrap().is_some());
    assert!(app.store.nodes(&conversation.id).unwrap().iter().any(|node| node.content == "completed after deletion"));
    assert!(start_turn(app.state.clone(), app.user_id.clone(), conversation.id.clone(), request()).await.is_err());
}

#[tokio::test]
async fn malformed_persisted_registry_fails_loudly_without_leaking_stored_secret() {
    let app = admin_app(None).await;
    for raw in ["not json private-provider-key", r#"{"chatgpt-subscription":null}"#, r#"{"discover":null}"#, r#"{"broken":{"label":"Broken","base_url":"ftp://example.test","api_key":"private-provider-key"}}"#] {
        app.store.set_setting(SettingKey::Providers, Some(raw)).unwrap();
        let app_copy = app.state.clone();
        let database = app.dir.path().join("nekochat.db");
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            AppState::new((*app_copy.config).clone(), Vec::new(), Store::open(&database).unwrap())
        })).err().expect("invalid persisted providers must fail construction");
        let message = panic.downcast_ref::<String>().map(String::as_str).or_else(|| panic.downcast_ref::<&str>().copied()).unwrap_or("");
        assert!(!message.contains("private-provider-key"));
    }
}

#[tokio::test]
async fn discovery_draft_uses_retained_key_without_changing_live_or_saved_provider() {
    let app = admin_app(Some("https://original.example.test/v1".into())).await;
    let draft = nekochat_server::providers::resolve_draft(&app.state, serde_json::from_value(json!({
        "key":PROVIDER_KEY,"label":"Unsaved draft","base_url":"https://draft.example.test/prefix/v1",
        "models_url":"https://draft.example.test/catalog/models","api_key":" ","protocol":"responses"
    })).unwrap()).unwrap();
    assert_eq!(draft.api_key, "test-key");
    assert_eq!(draft.base_url, "https://draft.example.test/prefix/v1");
    assert_eq!(app.state.provider(PROVIDER_KEY).unwrap().base_url, "https://original.example.test/v1");
    assert!(app.store.get_setting(SettingKey::Providers).unwrap().is_none());
}

#[tokio::test]
async fn startup_overrides_and_tombstones_do_not_require_obsolete_environment_keys() {
    let app = admin_app(Some("https://original.example.test/v1".into())).await;
    let mut config = (*app.state.config).clone();
    config.providers = serde_json::from_value(json!([
        {"key":"mock","label":"Original","base_url":"https://original.example.test/v1","api_key_env":"OBSOLETE_PROVIDER_ENV"}
    ])).unwrap();
    assert!(nekochat_server::providers::resolve_configured(&config, &app.store, |_| None).is_err());
    assert_eq!(save(&app, Some("mock"), &write_body("mock", "https://edited.example.test/v2")).await.status(), 200);
    let baseline = nekochat_server::providers::resolve_configured(&config, &app.store, |_| panic!("overridden provider must not resolve its obsolete environment key")).unwrap();
    let restored = AppState::new(config.clone(), baseline, Store::open(&app.dir.path().join("nekochat.db")).unwrap());
    assert_eq!(restored.provider("mock").unwrap().api_key, "private-provider-key");
    assert_eq!(restored.provider("mock").unwrap().base_url, "https://edited.example.test/v2");
    assert_eq!(app.client.delete(app.url("/api/admin/providers/mock")).send().await.unwrap().status(), 204);
    let baseline = nekochat_server::providers::resolve_configured(&config, &app.store, |_| panic!("deleted provider must not resolve its obsolete environment key")).unwrap();
    let restored = AppState::new(config.clone(), baseline, Store::open(&app.dir.path().join("nekochat.db")).unwrap());
    assert!(restored.provider("mock").is_none());
    config.providers.push(config.providers[0].clone());
    assert!(nekochat_server::providers::resolve_configured(&config, &app.store, |_| None).is_err());
    config.providers.truncate(1);
    config.providers[0].key = "chatgpt-subscription".into();
    assert!(nekochat_server::providers::resolve_configured(&config, &app.store, |_| None).is_err());
    config.providers[0].key = "discover".into();
    assert!(nekochat_server::providers::resolve_configured(&config, &app.store, |_| None).is_err());
}

#[tokio::test]
async fn endpoint_credentials_are_redacted_and_survive_unmodified_endpoint_edits() {
    let private_base = "https://hidden-user:hidden-password@example.test/v1?token=hidden-query#private";
    let app = admin_app(Some(private_base.into())).await;
    let response = app.client.get(app.url("/api/admin/providers")).send().await.unwrap().text().await.unwrap();
    for secret in ["hidden-user", "hidden-password", "hidden-query", "private"] {
        assert!(!response.contains(secret));
    }
    let body = json!({"label":"Edited","base_url":"https://example.test/v1"});
    assert_eq!(save(&app, Some(PROVIDER_KEY), &body).await.status(), 200);
    assert_eq!(app.state.provider(PROVIDER_KEY).unwrap().base_url, private_base);
    assert_eq!(reconstruct(&app, Vec::new()).provider(PROVIDER_KEY).unwrap().base_url, private_base);
    for (field, value) in [("base_url", "https://example.test/v1?api_key=hidden-query"), ("models_url", "https://example.test/models?token=hidden-query")] {
        let mut body = write_body("new", "https://example.test/v1");
        body[field] = json!(value);
        let response = save(&app, None, &body).await;
        assert_eq!(response.status(), 400);
        assert!(!response.text().await.unwrap().contains("hidden-query"));
    }
    for key in [".", ".."] {
        assert_eq!(save(&app, None, &write_body(key, "https://example.test/v1")).await.status(), 400);
    }
}

#[tokio::test]
async fn replacing_site_does_not_send_its_key_to_original_balance_endpoint() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let router = axum::Router::new().route("/balance", axum::routing::get(move || {
            let observed = observed.clone();
            async move { observed.fetch_add(1, Ordering::SeqCst); axum::Json(json!({"is_available":true,"balance_infos":[]})) }
        }));
        axum::serve(listener, router).await.unwrap();
    });
    let app = admin_app(Some(old_base.clone())).await;
    let mut config = (*app.state.config).clone();
    config.providers = serde_json::from_value(json!([{
        "key":PROVIDER_KEY,"label":"Original","base_url":old_base,"api_key_env":"OLD_ENV",
        "monitoring":{"kind":"deepseek","balance_url":format!("{old_base}/balance")}
    }])).unwrap();
    assert_eq!(save(&app, Some(PROVIDER_KEY), &write_body(PROVIDER_KEY, "https://replacement.example/v1")).await.status(), 200);
    let state = AppState::new(config, Vec::new(), Store::open(&app.dir.path().join("nekochat.db")).unwrap());
    nekochat_server::api::monitoring::quotas(axum::extract::State(state)).await.unwrap();
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn provider_protocol_survives_edits_and_restart_without_returning_key() {
    let app = admin_app(None).await;
    let mut body = write_body("protocol-state", "https://example.test/v1");
    body["protocol"] = json!("responses");
    let created = save(&app, None, &body).await;
    assert_eq!(created.status(), 201);
    let view: Value = created.json().await.unwrap();
    assert_eq!(view["protocol"], "responses");
    assert_eq!(view["has_api_key"], true);
    assert!(view.get("api_key").is_none());
    assert!(!view.to_string().contains("private-provider-key"));
    let model = app.store.insert_model(&serde_json::from_value(json!({
        "provider":"protocol-state","upstream_id":"existing","display_name":"Existing model","sort":0,
        "protocol":"chat_completions","reasoning":{"style":"none","levels":[],"default":null},
        "vision":false,"context_window":null,"price":{"cached_in":null,"uncached_in":null,"output":null}
    })).unwrap()).unwrap();
    body.as_object_mut().unwrap().remove("api_key");
    body.as_object_mut().unwrap().remove("protocol");
    body["label"] = json!("Renamed");
    let edited = save(&app, Some("protocol-state"), &body).await;
    assert_eq!(edited.status(), 200);
    assert_eq!(edited.json::<Value>().await.unwrap()["protocol"], "responses");
    body["protocol"] = Value::Null;
    let edited = save(&app, Some("protocol-state"), &body).await;
    assert_eq!(edited.status(), 200);
    assert_eq!(edited.json::<Value>().await.unwrap()["protocol"], "responses");
    assert_eq!(app.store.get_model(&model.id).unwrap().unwrap().spec.protocol, nekochat_core::Protocol::ChatCompletions);
    let restored = reconstruct(&app, Vec::new());
    let rows = serde_json::to_value(nekochat_server::providers::list(axum::extract::State(restored)).await.0).unwrap();
    let restored = rows.as_array().unwrap().iter().find(|row| row["key"] == "protocol-state").unwrap();
    assert_eq!(restored["protocol"], "responses");
    assert_eq!(restored["has_api_key"], true);
    assert!(restored.get("api_key").is_none());
    assert!(!rows.to_string().contains("private-provider-key"));
}

#[tokio::test]
async fn legacy_provider_delta_loads_without_claiming_a_saved_protocol() {
    let app = admin_app(None).await;
    app.store.set_setting(SettingKey::Providers, Some(r#"{"legacy":{"label":"Legacy","base_url":"https://example.test/v1","api_key":"legacy-private-key"}}"#)).unwrap();
    let restored = reconstruct(&app, Vec::new());
    assert_eq!(restored.provider("legacy").unwrap().api_key, "legacy-private-key");
    let rows = serde_json::to_value(nekochat_server::providers::list(axum::extract::State(restored)).await.0).unwrap();
    let legacy = rows.as_array().unwrap().iter().find(|row| row["key"] == "legacy").unwrap();
    assert_eq!(legacy["protocol"], Value::Null);
    assert_eq!(legacy["has_api_key"], true);
    assert!(legacy.get("api_key").is_none());
    assert!(!rows.to_string().contains("legacy-private-key"));
}

#[tokio::test]
async fn provider_protocol_changes_are_persisted_without_rewriting_models() {
    let app = admin_app(Some("https://example.test/v1".into())).await;
    let mut body = write_body(PROVIDER_KEY, "https://example.test/v1");
    app.store.insert_model(&serde_json::from_value(json!({
        "provider":PROVIDER_KEY,"upstream_id":"existing","display_name":"Existing model","sort":0,
        "protocol":"chat_completions","reasoning":{"style":"none","levels":[],"default":null},
        "vision":false,"context_window":null,"price":{"cached_in":null,"uncached_in":null,"output":null}
    })).unwrap()).unwrap();
    let before = serde_json::to_value(app.store.list_models().unwrap()).unwrap();
    assert!(!before.as_array().unwrap().is_empty());
    for protocol in ["responses", "chat_completions"] {
        body["protocol"] = json!(protocol);
        let response = save(&app, Some(PROVIDER_KEY), &body).await;
        assert_eq!(response.status(), 200);
        assert_eq!(response.json::<Value>().await.unwrap()["protocol"], protocol);
        let restored = reconstruct(&app, Vec::new());
        let rows = serde_json::to_value(nekochat_server::providers::list(axum::extract::State(restored)).await.0).unwrap();
        let row = rows.as_array().unwrap().iter().find(|row| row["key"] == PROVIDER_KEY).unwrap();
        assert_eq!(row["protocol"], protocol);
        assert_eq!(serde_json::to_value(app.store.list_models().unwrap()).unwrap(), before);
    }
    body["protocol"] = json!("chatgpt_responses");
    assert_eq!(save(&app, Some(PROVIDER_KEY), &body).await.status(), 400);
    let rows: Value = app.client.get(app.url("/api/admin/providers")).send().await.unwrap().json().await.unwrap();
    let row = rows.as_array().unwrap().iter().find(|row| row["key"] == PROVIDER_KEY).unwrap();
    assert_eq!(row["protocol"], "chat_completions");
}
