mod common;

use common::{TestCfg, spawn_app};
use serde_json::{Value, json};

#[tokio::test]
async fn management_credentials_are_write_only_and_admin_scoped() {
    let app = spawn_app(TestCfg {
        admin_password: Some("search-admin-test-password".into()),
        ..Default::default()
    }).await;
    let endpoint = app.url("/api/admin/search/connection");
    let ordinary = app.client.get(&endpoint).send().await.unwrap();
    assert_eq!(ordinary.status(), 401);
    app.login_admin("search-admin-test-password").await;
    let response = app.client.put(&endpoint).json(&json!({
        "enabled": false,
        "base_url": "http://127.0.0.1:10011",
        "timeout_secs": 60,
        "max_calls_per_turn": 3,
        "max_results": 5,
        "access_token": "runtime-secret-not-for-the-browser",
        "admin_token": "management-secret-not-for-the-browser"
    })).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let saved = response.text().await.unwrap();
    for secret in ["runtime-secret-not-for-the-browser", "management-secret-not-for-the-browser"] {
        assert!(!saved.contains(secret), "a credential leaked through the save response");
    }
    let saved: Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(saved["has_access_token"], true);
    assert_eq!(saved["has_admin_token"], true);
    let loaded = app.client.get(&endpoint).send().await.unwrap();
    assert_eq!(loaded.status(), 200);
    let loaded = loaded.text().await.unwrap();
    assert!(!loaded.contains("runtime-secret-not-for-the-browser"));
    assert!(!loaded.contains("management-secret-not-for-the-browser"));
    let loaded: Value = serde_json::from_str(&loaded).unwrap();
    assert_eq!(loaded["has_access_token"], true);
    assert_eq!(loaded["has_admin_token"], true);
    let saved_again = app.client.put(&endpoint).json(&json!({
        "enabled": false,
        "base_url": "http://127.0.0.1:10011",
        "timeout_secs": 60,
        "max_calls_per_turn": 3,
        "max_results": 5
    })).send().await.unwrap();
    assert_eq!(saved_again.status(), 200);
    let saved_again: Value = saved_again.json().await.unwrap();
    assert_eq!(saved_again["has_access_token"], true, "omitting a secret must not clear it");
    assert_eq!(saved_again["has_admin_token"], true);
}

#[tokio::test]
async fn blank_tokens_preserve_explicit_clear_and_unconfigured_engine_isolated() {
    let app = spawn_app(TestCfg { admin_password: Some("admin".into()), ..Default::default() }).await;
    app.login_admin("admin").await;
    let endpoint = app.url("/api/admin/search/connection");
    let body = json!({"enabled":false,"base_url":"http://127.0.0.1:10011","access_token":"runtime-private","admin_token":"admin-private"});
    assert_eq!(app.client.put(&endpoint).json(&body).send().await.unwrap().status(), 200);
    let body = json!({"enabled":false,"base_url":"http://127.0.0.1:10011","access_token":"  ","admin_token":""});
    let saved: Value = app.client.put(&endpoint).json(&body).send().await.unwrap().json().await.unwrap();
    assert_eq!(saved["has_access_token"], true);
    assert_eq!(saved["has_admin_token"], true);
    let body = json!({"enabled":false,"clear_access_token":true,"clear_admin_token":true});
    let saved: Value = app.client.put(&endpoint).json(&body).send().await.unwrap().json().await.unwrap();
    assert_eq!(saved["has_access_token"], false);
    assert_eq!(saved["has_admin_token"], false);
    assert!(app.state.search.runtime().is_err());
    assert_eq!(app.client.get(app.url("/api/admin/search/services")).send().await.unwrap().status(), 503);
    assert_eq!(app.client.get(app.url("/api/admin/characters")).send().await.unwrap().status(), 200);
    assert_eq!(app.client.get(app.url("/api/characters")).send().await.unwrap().status(), 200);
    assert_eq!(app.client.post(app.url("/api/admin/characters")).json(&json!({"name":"plain","display_name":"Plain"})).send().await.unwrap().status(), 201);
}

const SERVICE_ID: &str = "0196f817-1eb0-7000-8000-000000000001";

async fn search_engine_fixture() -> (String, tokio::task::JoinHandle<()>) {
    use axum::{Json, Router, http::{HeaderMap, StatusCode}, routing::{get, post}};
    async fn catalog(headers: HeaderMap, admin: bool) -> Result<Json<Value>, StatusCode> {
        let expected = if admin { "Bearer admin-test" } else { "Bearer runtime-test" };
        if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some(expected) {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(Json(json!([{"id":SERVICE_ID,"name":"Fixture","kind":"deepseek","enabled":true,"auth_mode":"api_key","model":null,"has_api_key":true,"connected":true,"supports_fetch":true}])))
    }
    let router = Router::new()
        .route("/v1/services", get(|headers: HeaderMap| async { catalog(headers, false).await }))
        .route("/v1/admin/services", get(|headers: HeaderMap| async { catalog(headers, true).await }))
        .route("/v1/admin/services/{id}", axum::routing::delete(|headers: HeaderMap| async move {
            if headers.get("authorization").and_then(|h| h.to_str().ok()) == Some("Bearer admin-test") { StatusCode::NO_CONTENT } else { StatusCode::UNAUTHORIZED }
        }))
        .route("/v1/search", post(|headers: HeaderMap, Json(body): Json<Value>| async move {
            if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Bearer runtime-test")
                || body["service_id"] != SERVICE_ID
                || body["query"] != "public test"
                || body["request_id"].as_str().and_then(|id| uuid::Uuid::parse_str(id).ok()).is_none()
            {
                return Err(StatusCode::BAD_REQUEST);
            }
            Ok(Json(json!({"query":"public test","sources":[],"summary":null,"retrieved_at_ms":1,"usage":{"requests":1,"cached_in":0,"uncached_in":10,"output":2,"reasoning":0,"cost":null}})))
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    (base, task)
}

#[tokio::test]
async fn binding_validation_offline_preservation_picker_privacy_and_deletion_guard() {
    let app = spawn_app(TestCfg { upstream: Some("http://127.0.0.1:1".into()), admin_password: Some("admin".into()), ..Default::default() }).await;
    app.login_admin("admin").await;
    let model = app.store.insert_model(&serde_json::from_value(json!({
        "provider":"mock","upstream_id":"fixture","display_name":"Fixture","sort":0,
        "protocol":"chat_completions","reasoning":{"style":"none","levels":[],"default":null},
        "vision":false,"context_window":null,"price":{"cached_in":null,"uncached_in":null,"output":null}
    })).unwrap()).unwrap();
    let (base, engine) = search_engine_fixture().await;
    let mut connection = json!({"enabled":true,"base_url":base,"access_token":"runtime-test","admin_token":"admin-test","timeout_secs":1});
    assert_eq!(app.client.put(app.url("/api/admin/search/connection")).json(&connection).send().await.unwrap().status(), 200);
    assert!(nekochat_server::search::SearchBridge::new(app.store.clone(), app.state.codex.clone(), app.state.http.clone()).runtime().is_ok());
    let tested: Value = app.client.post(app.url("/api/admin/search/connection/test")).json(&json!({})).send().await.unwrap().json().await.unwrap();
    assert_eq!(tested, json!({"ok":true,"services":1}));
    let invalid = json!({"name":"invalid","display_name":"Invalid","search_service_id":"0196f817-1eb0-7000-8000-000000000002"});
    assert_eq!(app.client.post(app.url("/api/admin/characters")).json(&invalid).send().await.unwrap().status(), 400);
    assert_eq!(app.client.post(app.url("/api/admin/characters")).json(&json!({"name":"bad-id","display_name":"Bad ID","search_service_id":"../../admin"})).send().await.unwrap().status(), 400);
    let mut role = json!({"name":"bound","display_name":"Bound","model_id":model.id,"search_service_id":SERVICE_ID});
    let response = app.client.post(app.url("/api/admin/characters")).json(&role).send().await.unwrap();
    assert_eq!(response.status(), 201);
    let saved: Value = response.json().await.unwrap();
    let id = saved["id"].as_str().unwrap();
    let picker = app.client.get(app.url("/api/characters")).send().await.unwrap().text().await.unwrap();
    assert!(picker.contains(id), "the bound role must actually appear in the ordinary picker");
    assert!(!picker.contains("search_service_id"));
    assert!(!picker.contains(SERVICE_ID));
    assert_eq!(app.client.delete(app.url(&format!("/api/admin/search/services/{SERVICE_ID}"))).send().await.unwrap().status(), 409);
    engine.abort();
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    connection["base_url"] = json!(format!("http://{}", closed.local_addr().unwrap()));
    drop(closed);
    assert_eq!(app.client.put(app.url("/api/admin/search/connection")).json(&connection).send().await.unwrap().status(), 200);
    role["display_name"] = json!("Offline edited");
    let edited = app.client.put(app.url(&format!("/api/admin/characters/{id}"))).json(&role).send().await.unwrap();
    assert_eq!(edited.status(), 200);
    let edited: Value = edited.json().await.unwrap();
    assert_eq!(edited["search_service_id"], SERVICE_ID);
    let offline_picker: Value = app.client.get(app.url("/api/characters")).send().await.unwrap().json().await.unwrap();
    assert_eq!(offline_picker["characters"][0]["id"], id);
    assert_eq!(app.client.post(app.url("/api/admin/characters")).json(&role).send().await.unwrap().status(), 502);
    role["search_service_id"] = Value::Null;
    assert_eq!(app.client.put(app.url(&format!("/api/admin/characters/{id}"))).json(&role).send().await.unwrap().status(), 200);
}

#[tokio::test]
async fn connection_test_checks_both_auth_scopes_without_search_and_billed_test_records_metadata() {
    let app = spawn_app(TestCfg { admin_password: Some("admin".into()), ..Default::default() }).await;
    app.login_admin("admin").await;
    let (base, engine) = search_engine_fixture().await;
    for (runtime, admin, expected) in [("wrong", "admin-test", 502), ("runtime-test", "wrong", 502), ("runtime-test", "admin-test", 200)] {
        let connection = json!({"enabled":true,"base_url":base,"access_token":runtime,"admin_token":admin});
        assert_eq!(app.client.put(app.url("/api/admin/search/connection")).json(&connection).send().await.unwrap().status(), 200);
        assert_eq!(app.client.post(app.url("/api/admin/search/connection/test")).json(&json!({})).send().await.unwrap().status(), expected);
    }
    assert!(app.store.list_search_usage(0, i64::MAX).unwrap().is_empty());
    for _ in 0..2 {
        assert_eq!(app.client.post(app.url(&format!("/api/admin/search/services/{SERVICE_ID}/test"))).json(&json!({"query":"public test"})).send().await.unwrap().status(), 200);
    }
    let records = app.store.list_search_usage(0, i64::MAX).unwrap();
    assert_eq!(records.len(), 2);
    assert_ne!(records[0].id, records[1].id);
    assert_eq!(records[0].status, "complete");
    assert_eq!(records[0].usage.as_ref().unwrap()["cost"], Value::Null);
    assert_eq!(app.client.post(app.url(&format!("/api/admin/search/services/{SERVICE_ID}/test"))).json(&json!({"query":"rejected query"})).send().await.unwrap().status(), 400);
    let records = app.store.list_search_usage(0, i64::MAX).unwrap();
    assert_eq!(records.len(), 3);
    assert!(records.iter().any(|record| record.status == "error" && record.usage.is_none()));
    let response = app.client.get(app.url("/api/admin/search/usage")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let usage = response.text().await.unwrap();
    assert!(!usage.contains("public test"));
    assert!(!usage.contains("sources"));
    assert!(!usage.contains("rejected query"));
    let dashboard: Value = serde_json::from_str(&usage).unwrap();
    assert_eq!(dashboard["totals"], json!({
        "attempts":3,"requests":2,"cached_in":0,"uncached_in":20,"output":4,"reasoning":0,
        "known_costs":[],"unknown_costs":3
    }));
    assert_eq!(dashboard["records"].as_array().unwrap().len(), 3);
    assert_eq!(app.client.get(app.url("/api/admin/search/usage?from_ms=0&to_ms=9223372036854775807")).send().await.unwrap().status(), 400);
    engine.abort();
}

#[tokio::test]
async fn every_search_management_route_denies_ordinary_users() {
    let app = spawn_app(TestCfg { admin_password: Some("admin-only".into()), ..Default::default() }).await;
    for (method, path) in [
        ("GET", "/connection"), ("PUT", "/connection"), ("POST", "/connection/test"),
        ("GET", "/services"), ("POST", "/services"), ("PUT", "/services/id"), ("DELETE", "/services/id"),
        ("POST", "/services/id/test"), ("GET", "/usage")
    ] {
        let response = app.client.request(method.parse().unwrap(), app.url(&format!("/api/admin/search{path}"))).json(&json!({})).send().await.unwrap();
        assert_eq!(response.status(), 401, "{method} {path}");
    }
}

#[tokio::test]
async fn usage_dashboard_returns_full_period_totals_and_sparse_points_with_latest_hundred_details() {
    use nekochat_store::SearchUsageRecord;
    let app = spawn_app(TestCfg { admin_password: Some("admin".into()), ..Default::default() }).await;
    app.login_admin("admin").await;
    for index in 0..100 {
        app.store.insert_search_usage(&SearchUsageRecord {
            id: format!("a-{index:03}"), at_ms: 900_000 + index, service_id: "deleted-service-a".into(),
            status: "complete".into(),
            usage: Some(json!({"requests":2,"cached_in":1,"uncached_in":2,"output":3,"reasoning":4,"cost":{"amount":0.5,"currency":"CNY"}})),
        }).unwrap();
    }
    for (id, at_ms, status, usage) in [
        ("b-zero", 1_800_000, "complete", Some(json!({"requests":1,"cached_in":5,"uncached_in":6,"output":7,"reasoning":8,"cost":{"amount":0.0,"currency":"USD"}}))),
        ("b-paid", 1_800_001, "complete", Some(json!({"requests":1,"cached_in":1,"uncached_in":2,"output":3,"reasoning":4,"cost":{"amount":2.0,"currency":"USD"}}))),
        ("b-unknown", 1_800_002, "complete", Some(json!({"requests":3,"cached_in":1,"uncached_in":2,"output":3,"reasoning":4,"cost":null}))),
        ("b-error", 1_800_003, "error", None),
        ("b-cancel", 1_800_004, "cancelled", None),
    ] {
        app.store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "deleted-service-b".into(), status: status.into(), usage,
        }).unwrap();
    }
    for (id, at_ms) in [("excluded-before", 899_999), ("excluded-after", 1_800_005)] {
        app.store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "excluded-service".into(), status: "error".into(), usage: None,
        }).unwrap();
    }
    let response = app.client.get(app.url("/api/admin/search/usage?from_ms=900000&to_ms=1800004")).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let text = response.text().await.unwrap();
    for private in ["query", "sources", "evidence", "url", "excluded-service", "excluded-before", "excluded-after"] {
        assert!(!text.contains(private), "dashboard exposed excluded data: {private}");
    }
    let dashboard: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(dashboard["from_ms"], 900_000);
    assert_eq!(dashboard["to_ms"], 1_800_004);
    assert_eq!(dashboard["bucket_ms"], 900_000);
    assert_eq!(dashboard["totals"], json!({
        "attempts":105,"requests":205,"cached_in":107,"uncached_in":210,"output":313,"reasoning":416,
        "known_costs":[{"currency":"CNY","amount":50.0},{"currency":"USD","amount":2.0}],"unknown_costs":3
    }));
    let services = dashboard["services"].as_array().unwrap();
    assert_eq!(services.len(), 2);
    assert_eq!(services[0]["service_id"], "deleted-service-a");
    assert_eq!(services[0]["totals"]["attempts"], 100);
    assert_eq!(services[0]["points"], json!([{"at_ms":900000,"attempts":100,"requests":200,"cached_in":100,"uncached_in":200,"output":300,"reasoning":400}]));
    assert_eq!(services[1]["service_id"], "deleted-service-b");
    assert_eq!(services[1]["totals"]["unknown_costs"], 3);
    assert_eq!(services[1]["points"], json!([{"at_ms":1800000,"attempts":5,"requests":5,"cached_in":7,"uncached_in":10,"output":13,"reasoning":16}]));
    let records = dashboard["records"].as_array().unwrap();
    assert_eq!(records.len(), 100);
    assert_eq!(records[0]["id"], "b-cancel");
    assert_eq!(records[0]["usage"], Value::Null);
    assert_eq!(records[2]["usage"]["cost"], Value::Null);
    assert_eq!(records[99]["id"], "a-005");
}

#[tokio::test]
async fn usage_dashboard_rejects_invalid_ranges_and_preserves_empty_and_inclusive_bounds() {
    use nekochat_store::SearchUsageRecord;
    const DAY: i64 = 86_400_000;
    let app = spawn_app(TestCfg { admin_password: Some("admin".into()), ..Default::default() }).await;
    let endpoint = app.url("/api/admin/search/usage");
    let anonymous = reqwest::Client::new().get(&endpoint).send().await.unwrap();
    assert_eq!(anonymous.status(), 401);
    app.login_admin("admin").await;
    for (id, at_ms) in [("before", 9), ("tie-a", 10), ("tie-b", 10), ("end", 20), ("after", 21)] {
        app.store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "service".into(), status: "error".into(), usage: None,
        }).unwrap();
    }
    let inclusive: Value = app.client.get(app.url("/api/admin/search/usage?from_ms=10&to_ms=20")).send().await.unwrap().json().await.unwrap();
    assert_eq!(inclusive["totals"]["attempts"], 3);
    assert_eq!(inclusive["totals"]["unknown_costs"], 3);
    assert_eq!(inclusive["records"].as_array().unwrap().iter().map(|record| record["id"].as_str().unwrap()).collect::<Vec<_>>(),
        vec!["end", "tie-b", "tie-a"]);
    let single: Value = app.client.get(app.url("/api/admin/search/usage?from_ms=10&to_ms=10")).send().await.unwrap().json().await.unwrap();
    assert_eq!(single["totals"]["attempts"], 2);
    assert_eq!(single["services"][0]["points"][0]["attempts"], 2);
    let empty: Value = app.client.get(app.url("/api/admin/search/usage?from_ms=11&to_ms=19")).send().await.unwrap().json().await.unwrap();
    assert_eq!(empty["totals"], json!({
        "attempts":0,"requests":0,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"known_costs":[],"unknown_costs":0
    }));
    assert_eq!(empty["services"], json!([]));
    assert_eq!(empty["records"], json!([]));
    let maximum_range = app.client.get(format!("{endpoint}?from_ms=0&to_ms={}", 31 * DAY)).send().await.unwrap();
    assert_eq!(maximum_range.status(), 200);
    assert_eq!(maximum_range.json::<Value>().await.unwrap()["totals"]["attempts"], 5);
    let default_from: Value = app.client.get(format!("{endpoint}?to_ms=1")).send().await.unwrap().json().await.unwrap();
    assert_eq!(default_from["from_ms"], 0);
    assert_eq!(default_from["to_ms"], 1);
    for query in [
        "from_ms=-1&to_ms=1", "from_ms=2&to_ms=1", "from_ms=0&to_ms=2678400001",
        "from_ms=0&to_ms=9223372036854775807", "from_ms=-9223372036854775808&to_ms=9223372036854775807",
        "from_ms=invalid&to_ms=1", "from_ms=0&to_ms=1&query=private",
    ] {
        assert_eq!(app.client.get(format!("{endpoint}?{query}")).send().await.unwrap().status(), 400, "{query}");
    }
}

#[tokio::test]
async fn shared_binding_tracks_the_main_account_without_engine_credentials() {
    let data = tempfile::tempdir().unwrap();
    let engine = nekochat_search::server::Engine::open(data.path(), "runtime-test", "admin-test").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, engine.router()).await.unwrap(); });
    for connected in [false, true] {
        let app = spawn_app(TestCfg {
            upstream: Some("http://127.0.0.1:1".into()),
            admin_password: Some("admin".into()), shared_account: connected, ..Default::default()
        }).await;
        app.login_admin("admin").await;
        assert_eq!(app.state.codex.connected(), connected);
        assert_eq!(app.client.put(app.url("/api/admin/search/connection")).json(&json!({
            "enabled":true,"base_url":base,"access_token":"runtime-test","admin_token":"admin-test"
        })).send().await.unwrap().status(), 200);
        let body = json!({"name":"Private shared service","kind":"gpt","enabled":true,"auth_mode":"shared_chatgpt","model":"gpt-5.4"});
        let created = app.client.post(app.url("/api/admin/search/services")).json(&body).send().await.unwrap();
        assert_eq!(created.status(), 201);
        let created: Value = created.json().await.unwrap();
        let id = created["id"].as_str().unwrap();
        assert_eq!(created["connected"], connected);
        assert_eq!(created["has_api_key"], false);
        let catalog: Vec<Value> = app.client.get(app.url("/api/admin/search/services")).send().await.unwrap().json().await.unwrap();
        assert_eq!(catalog.iter().find(|service| service["id"] == id).unwrap()["connected"], connected);
        let updated: Value = app.client.put(app.url(&format!("/api/admin/search/services/{id}"))).json(&body).send().await.unwrap().json().await.unwrap();
        assert_eq!(updated["connected"], connected);
        let model = app.store.insert_model(&serde_json::from_value(json!({
            "provider":"mock","upstream_id":"fixture","display_name":"Private model","sort":0,
            "protocol":"chat_completions","reasoning":{"style":"none","levels":[],"default":null},
            "vision":false,"context_window":null,"price":{"cached_in":null,"uncached_in":null,"output":null}
        })).unwrap()).unwrap();
        let role = json!({"name":"shared-role","display_name":"Shared role","model_id":model.id,"search_service_id":id});
        let bound = app.client.post(app.url("/api/admin/characters")).json(&role).send().await.unwrap();
        assert_eq!(bound.status(), if connected { 201 } else { 400 });
        let saved: Value = if connected {
            bound.json().await.unwrap()
        } else {
            serde_json::to_value(app.store.create_character(&serde_json::from_value(role.clone()).unwrap()).unwrap()).unwrap()
        };
        let role_id = saved["id"].as_str().unwrap();
        let conv = app.store.create_owned_conversation(&app.user_id, Some(role_id)).unwrap();
        let prepared = app.client.post(app.url(&format!("/api/conversations/{}/turns", conv.id))).json(&json!({
            "action":"send","content":"public question","character_id":role_id
        })).send().await.unwrap();
        assert_eq!(prepared.status(), if connected { 200 } else { 503 }, "core preparation must use the same account availability as role validation");
        let stream = prepared.text().await.unwrap();
        for private in ["Private shared service","shared_chatgpt","synthetic-shared-access","synthetic-refresh-never-delegated",id] {
            assert!(!stream.contains(private));
        }
        if connected {
            let offline_http = reqwest::Client::builder().proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap()).build().unwrap();
            app.state.codex.logout(&offline_http).await.unwrap();
            assert!(!app.state.codex.connected());
            let catalog: Vec<Value> = app.client.get(app.url("/api/admin/search/services")).send().await.unwrap().json().await.unwrap();
            assert_eq!(catalog.iter().find(|service| service["id"] == id).unwrap()["connected"], false);
            assert_eq!(app.client.put(app.url(&format!("/api/admin/characters/{role_id}"))).json(&role).send().await.unwrap().status(), 200);
            let rejected = app.client.post(app.url(&format!("/api/admin/search/services/{id}/test"))).json(&json!({"query":"public question"})).send().await.unwrap();
            assert_eq!(rejected.status(), 409);
        }
        let raw = app.client.get(app.url("/api/characters")).send().await.unwrap().text().await.unwrap();
        for private in ["Private shared service","shared_chatgpt","synthetic-shared-access","synthetic-refresh-never-delegated","synthetic-id-never-delegated",id] {
            assert!(!raw.contains(private));
        }
        let engine_catalog: Value = reqwest::Client::new().get(format!("{base}/v1/admin/services")).bearer_auth("admin-test").send().await.unwrap().json().await.unwrap();
        assert_eq!(engine_catalog.as_array().unwrap().iter().find(|service| service["id"] == id).unwrap()["connected"], false);
    }
    task.abort();
}

