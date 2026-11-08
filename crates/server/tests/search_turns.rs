mod common;

use std::sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use axum::{Json, Router, body::Body, extract::State, http::{HeaderMap, StatusCode}, response::{IntoResponse, Response}, routing::{get, post}};
use common::{TestApp, TestCfg, PROVIDER_KEY, spawn_app};
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

const SERVICE: &str = "0196f817-1eb0-7000-8000-000000000009";

const CHOICES: &str = "```neko-ui\n{\"version\":1,\"items\":[{\"type\":\"choices\",\"options\":[{\"label\":\"看例子\",\"prompt\":\"请举例。\"}]}]}\n```";
const STICKER: &str = "![本地表情](/api/resources/fixture/sticker.webp)";
const UNSEARCHED_ANSWER: &str = "本次未能联网核查。根据已有知识，SQLite WAL 允许读写并发，但同一时刻只有一个写者；最新细节请以 SQLite 官方文档为准。";

fn model_answer() -> String {
    let mut answer = String::from("WAL 允许读写并发，但同一时刻只有一个写者。[1]\n\n");
    answer.push_str(CHOICES);
    answer.push_str("\n\n**参考来源**\n\n");
    for index in 1..=5 {
        let url = if index == 1 { "https://sqlite.org/wal.html".to_owned() } else { format!("https://sqlite.org/source-{index}.html") };
        answer.push_str(&format!("[{index}] [模型来源 {index}]({url})\n"));
    }
    answer.push_str("\n");
    answer.push_str(STICKER);
    answer
}

struct Research {
    native: bool,
    invalid_read: bool,
    wait: bool,
    empty: AtomicBool,
    failure_status: AtomicUsize,
    parallel: AtomicBool,
    parallel_fetch: AtomicBool,
    expect_budget_exhausted: AtomicBool,
    invalid_parallel_source: AtomicBool,
    final_refusal: AtomicBool,
    request_again: AtomicBool,
    malformed_response: AtomicBool,
    engine_failure: AtomicUsize,
    unsafe_source: AtomicBool,
    model_calls: AtomicUsize,
    search_calls: AtomicUsize,
    fetch_calls: AtomicUsize,
    cancel_calls: AtomicUsize,
    started: Notify,
    release: Notify,
    cancellation: CancellationToken,
}

fn sse(events: Vec<Value>, chat: bool) -> Response {
    let mut body = String::new();
    for event in events { body.push_str(&format!("data: {event}\n\n")); }
    if chat { body.push_str("data: [DONE]\n\n"); }
    Response::builder().header("content-type", "text/event-stream").body(Body::from(body)).unwrap()
}

async fn model(State(state): State<Arc<Research>>, Json(body): Json<Value>) -> Response {
    let round = state.model_calls.fetch_add(1, Ordering::SeqCst);
    if round == 0 {
        let name = if state.invalid_read { "web_fetch" } else { "web_search" };
        let arguments = if state.invalid_read { r#"{"source_id":"file:///etc/passwd"}"# } else { r#"{"query":"SQLite WAL"}"# };
        let mut calls = vec![("lookup-1", name, arguments)];
        if state.parallel.load(Ordering::SeqCst) {
            if state.invalid_parallel_source.load(Ordering::SeqCst) {
                calls.push(("lookup-2", "web_fetch", r#"{"source_id":"999"}"#));
            } else {
                calls.push(("lookup-2", "web_search", r#"{"query":"SQLite WAL"}"#));
            }
            if state.parallel_fetch.load(Ordering::SeqCst) {
                calls.push(("read-1", "web_fetch", r#"{"source_id":"1"}"#));
            }
        }
        if state.native {
            let mut events = vec![
                json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"reasoning-1","delta":"核查公开来源"}),
                json!({"type":"response.output_item.done","output_index":0,"item":{"type":"reasoning","id":"reasoning-1","summary":[],"encrypted_content":"opaque-continuation-state"}}),
            ];
            events.extend(calls.iter().enumerate().map(|(index, (id, name, arguments))| json!({"type":"response.output_item.done","output_index":index + 1,"item":{"type":"function_call","id":format!("function-{id}"),"call_id":id,"name":name,"arguments":arguments}})));
            events.push(json!({"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":4,"output_tokens_details":{"reasoning_tokens":2}}}}));
            sse(events, false)
        } else {
            let calls: Vec<_> = calls.iter().enumerate().map(|(index, (id, name, arguments))| json!({"index":index,"id":id,"type":"function","function":{"name":name,"arguments":arguments}})).collect();
            sse(vec![
                json!({"choices":[{"delta":{"reasoning_content":"核查公开来源"}}]}),
                json!({"choices":[{"delta":{"tool_calls":calls}}]}),
                json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":4,"completion_tokens_details":{"reasoning_tokens":2}}}),
            ], true)
        }
    } else {
        if round != 1 || body.get("tools").is_some() {
            return StatusCode::BAD_REQUEST.into_response();
        }
        let outputs: Vec<_> = if state.native {
            let Some(input) = body["input"].as_array() else { return StatusCode::BAD_REQUEST.into_response(); };
            if !input.iter().any(|item| item["type"] == "reasoning" && item["encrypted_content"] == "opaque-continuation-state") {
                return StatusCode::BAD_REQUEST.into_response();
            }
            input.iter().filter(|item| item["type"] == "function_call_output").map(|item| (&item["call_id"], &item["output"])).collect()
        } else {
            let Some(messages) = body["messages"].as_array() else { return StatusCode::BAD_REQUEST.into_response(); };
            messages.iter().filter(|item| item["role"] == "tool").map(|item| (&item["tool_call_id"], &item["content"])).collect()
        };
        let expected_ids = if state.parallel_fetch.load(Ordering::SeqCst) { vec!["lookup-1", "lookup-2", "read-1"] } else if state.parallel.load(Ordering::SeqCst) { vec!["lookup-1", "lookup-2"] } else { vec!["lookup-1"] };
        if outputs.len() != expected_ids.len() { return StatusCode::BAD_REQUEST.into_response(); }
        let failed = state.failure_status.load(Ordering::SeqCst) != 0 || state.engine_failure.load(Ordering::SeqCst) == 4;
        for ((id, output), expected_id) in outputs.into_iter().zip(expected_ids) {
            if id.as_str() != Some(expected_id) { return StatusCode::BAD_REQUEST.into_response(); }
            let Some(evidence) = output.as_str().and_then(|text| serde_json::from_str::<Value>(text).ok()) else { return StatusCode::BAD_REQUEST.into_response(); };
            if state.expect_budget_exhausted.load(Ordering::SeqCst) && id.as_str() == Some("lookup-2") {
                if evidence["error"].as_str().is_none_or(str::is_empty) || evidence.get("sources").is_some() {
                    return StatusCode::BAD_REQUEST.into_response();
                }
            } else if failed && !(state.parallel_fetch.load(Ordering::SeqCst) && id.as_str() == Some("lookup-1")) {
                if evidence["error"]["code"] != "search_unavailable" || !evidence["error"]["cause"].is_string()
                    || evidence["error"]["message"].as_str().is_none_or(str::is_empty)
                    || evidence["sources"] != json!([]) || !evidence["generated_summary"].is_null()
                    || evidence["tools_disabled"] != true {
                    return StatusCode::BAD_REQUEST.into_response();
                }
            } else if !state.empty.load(Ordering::SeqCst) && (evidence["sources"][0]["url"] != "https://sqlite.org/wal.html" || evidence["sources"][0]["source_id"] != "1") {
                return StatusCode::BAD_REQUEST.into_response();
            }
        }
        if state.request_again.load(Ordering::SeqCst) {
            return if state.native {
                sse(vec![
                    json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"lookup-3","name":"web_search","arguments":"{\"query\":\"SQLite WAL\"}"}}),
                    json!({"type":"response.completed","response":{"usage":{"input_tokens":20,"input_tokens_details":{"cached_tokens":5},"output_tokens":6}}}),
                ], false)
            } else {
                sse(vec![
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"lookup-3","type":"function","function":{"name":"web_search","arguments":"{\"query\":\"SQLite WAL\"}"}}]}}]}),
                    json!({"choices":[],"usage":{"prompt_tokens":20,"prompt_tokens_details":{"cached_tokens":5},"completion_tokens":6}}),
                ], true)
            };
        }
        let answer = if state.final_refusal.load(Ordering::SeqCst) { "我无法回答这个问题。".to_owned() } else if failed { UNSEARCHED_ANSWER.to_owned() } else if state.empty.load(Ordering::SeqCst) { format!("{CHOICES}\n\nFinal prose without evidence.") } else { model_answer() };
        if state.native {
            sse(vec![
                json!({"type":"response.output_text.delta","delta":answer}),
                json!({"type":"response.completed","response":{"usage":{"input_tokens":20,"input_tokens_details":{"cached_tokens":5},"output_tokens":6}}}),
            ], false)
        } else {
            sse(vec![
                json!({"choices":[{"delta":{"content":answer}}]}),
                json!({"choices":[],"usage":{"prompt_tokens":20,"prompt_tokens_details":{"cached_tokens":5},"completion_tokens":6}}),
            ], true)
        }
    }
}

async fn services(headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer runtime-test") { return Err(StatusCode::UNAUTHORIZED); }
    Ok(Json(json!([{"id":SERVICE,"name":"private-search-name","kind":"gpt","enabled":true,"auth_mode":"api_key","model":"private-gpt-model","has_api_key":true,"connected":true,"supports_fetch":true}])))
}

async fn search(State(state): State<Arc<Research>>, headers: HeaderMap, Json(body): Json<Value>) -> Result<Json<Value>, Response> {
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer runtime-test")
        || body["service_id"] != SERVICE || body["query"] != "SQLite WAL" {
        return Err(StatusCode::BAD_REQUEST.into_response());
    }
    let call = state.search_calls.fetch_add(1, Ordering::SeqCst);
    state.started.notify_one();
    let failure_status = state.failure_status.load(Ordering::SeqCst);
    if failure_status != 0 && (!state.parallel_fetch.load(Ordering::SeqCst) || call > 0) {
        return Err(StatusCode::from_u16(failure_status as u16).unwrap().into_response());
    }
    if state.malformed_response.load(Ordering::SeqCst) { return Ok(Json(json!({"invalid":"search response"}))); }
    let engine_error = match state.engine_failure.load(Ordering::SeqCst) {
        1 => Some("invalid_provider_response"),
        2 => Some("private_storage_unavailable"),
        3 => Some("unknown_error_with_private_details"),
        4 => Some("provider_access_denied"),
        _ => None,
    };
    if let Some(code) = engine_error { return Err((StatusCode::BAD_GATEWAY, Json(json!({"error":code}))).into_response()); }
    if state.wait {
        tokio::select! {
            () = state.release.notified() => {},
            () = state.cancellation.cancelled() => return Err(StatusCode::from_u16(499).unwrap().into_response()),
        }
    }
    let sources: Vec<_> = (1..=10).map(|index| json!({"id":format!("provider-source-{index}"),"title":format!("Source {index}"),"url":if state.unsafe_source.load(Ordering::SeqCst) { "file:///etc/passwd".to_owned() } else if index == 1 { "https://sqlite.org/wal.html".to_owned() } else { format!("https://sqlite.org/source-{index}.html") },"snippet":"Only one writer at a time.","summary":null,"content":"Readers and a writer may proceed concurrently.","published_at":null})).collect();
    let sources = if state.empty.load(Ordering::SeqCst) { Vec::new() } else { sources };
    Ok(Json(json!({"query":"SQLite WAL","sources":sources,"summary":null,"retrieved_at_ms":1,"usage":{"requests":1,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"cost":{"currency":"CNY","amount":0.005}}})))
}

async fn fetch(State(state): State<Arc<Research>>) -> StatusCode {
    state.fetch_calls.fetch_add(1, Ordering::SeqCst);
    StatusCode::SERVICE_UNAVAILABLE
}

async fn cancel(State(state): State<Arc<Research>>, headers: HeaderMap, Json(body): Json<Value>) -> StatusCode {
    if headers.get("authorization").and_then(|value| value.to_str().ok()) != Some("Bearer runtime-test") || body["request_id"].as_str().and_then(|id| uuid::Uuid::parse_str(id).ok()).is_none() {
        return StatusCode::UNAUTHORIZED;
    }
    state.cancel_calls.fetch_add(1, Ordering::SeqCst);
    state.cancellation.cancel();
    StatusCode::NO_CONTENT
}

async fn rig(native: bool, invalid_read: bool, wait: bool) -> (TestApp, String, String, Arc<Research>, tokio::task::JoinHandle<()>) {
    rig_with_budget(native, invalid_read, wait, 1).await
}

async fn rig_with_budget(native: bool, invalid_read: bool, wait: bool, budget: u32) -> (TestApp, String, String, Arc<Research>, tokio::task::JoinHandle<()>) {
    let state = Arc::new(Research { native, invalid_read, wait, empty: AtomicBool::new(false), failure_status: AtomicUsize::new(0), parallel: AtomicBool::new(false), parallel_fetch: AtomicBool::new(false), expect_budget_exhausted: AtomicBool::new(false), invalid_parallel_source: AtomicBool::new(false), final_refusal: AtomicBool::new(false), request_again: AtomicBool::new(false), malformed_response: AtomicBool::new(false), engine_failure: AtomicUsize::new(0), unsafe_source: AtomicBool::new(false), model_calls: AtomicUsize::new(0), search_calls: AtomicUsize::new(0), fetch_calls: AtomicUsize::new(0), cancel_calls: AtomicUsize::new(0), started: Notify::new(), release: Notify::new(), cancellation: CancellationToken::new() });
    let router = Router::new().route("/chat/completions", post(model)).route("/responses", post(model))
        .route("/v1/services", get(services)).route("/v1/search", post(search)).route("/v1/fetch", post(fetch)).route("/v1/cancel", post(cancel)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let app = spawn_app(TestCfg { upstream: Some(base.clone()), admin_password: Some("admin".into()), ..Default::default() }).await;
    app.login_admin("admin").await;
    let response = app.client.put(app.url("/api/admin/search/connection")).json(&json!({"enabled":true,"base_url":base,"access_token":"runtime-test","admin_token":"admin-test","max_calls_per_turn":budget,"max_results":10,"timeout_secs":10})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let model = app.store.insert_model(&serde_json::from_value(json!({"provider":PROVIDER_KEY,"upstream_id":"fixture","display_name":"private-model-name","sort":0,"protocol":if native {"responses"} else {"chat_completions"},"reasoning":{"style":"none","levels":[],"default":null},"vision":false,"context_window":null,"price":{"cached_in":1.0,"uncached_in":2.0,"output":3.0}})).unwrap()).unwrap();
    let role = app.store.create_character(&serde_json::from_value(json!({"name":"research","display_name":"Research","model_id":model.id,"system_prompt":"PRIVATE ROLE INSTRUCTIONS","search_service_id":SERVICE})).unwrap()).unwrap();
    let conv = app.store.create_owned_conversation(&app.user_id, Some(&role.id)).unwrap();
    (app, conv.id, role.id, state, task)
}

#[tokio::test]
async fn authorized_search_without_results_still_finalizes_choices_without_a_footer() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig(native, false, false).await;
        state.empty.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"public question","character_id":role})).send().await.unwrap();
        let stream = response.text().await.unwrap();
        let terminal: Value = stream.split("\n\n").find_map(|event| {
            if !event.lines().any(|line| line == "event: done") { return None; }
            event.lines().find_map(|line| line.strip_prefix("data: ")).map(|data| serde_json::from_str(data).unwrap())
        }).expect("terminal node");
        let nodes = app.store.nodes(&conv).unwrap();
        let stored = nodes.iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert_eq!(terminal["node"]["content"], stored.content);
        assert!(stored.content.trim_end().ends_with(CHOICES), "{}", stored.content);
        assert!(stored.content.find("Final prose without evidence.").unwrap() < stored.content.find(CHOICES).unwrap());
        assert!(!stored.content.contains("参考来源"));
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn search_rounds_persist_sources_and_all_usage_without_exposing_private_configuration() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig(native, false, false).await;
        let endpoint = app.url(&format!("/api/conversations/{conv}/turns"));
        let rejected = app.client.post(&endpoint).json(&json!({"action":"send","content":"public question","character_id":role,"service_id":"attacker"})).send().await.unwrap();
        assert_eq!(rejected.status(), 400);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 0);
        let response = app.client.post(&endpoint).json(&json!({"action":"send","content":"public question","character_id":role})).send().await.unwrap();
        assert_eq!(response.status(), 200);
        let stream = tokio::time::timeout(Duration::from_secs(5), response.text()).await.unwrap().unwrap();
        assert!(stream.contains("event: done"), "{stream}");
        for private in [SERVICE, "private-search-name", "private-gpt-model", "private-model-name", "PRIVATE ROLE INSTRUCTIONS", "runtime-test", "opaque-continuation-state"] {
            assert!(!stream.contains(private), "private configuration leaked");
        }
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let nodes = app.store.nodes(&conv).unwrap();
        let answer = nodes.iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert_eq!(answer.status, nekochat_store::NodeStatus::Complete);
        assert_eq!(answer.content.matches("**参考来源**").count(), 1, "{}", answer.content);
        for index in 1..=10 {
            let url = if index == 1 { "https://sqlite.org/wal.html".to_owned() } else { format!("https://sqlite.org/source-{index}.html") };
            assert_eq!(answer.content.matches(url.as_str()).count(), 1, "{}", answer.content);
        }
        assert!(answer.content.contains("WAL 允许读写并发，但同一时刻只有一个写者。[1]"));
        assert!(answer.content.contains(STICKER));
        assert!(answer.content.trim_end().ends_with(CHOICES), "{}", answer.content);
        assert_eq!(answer.content.matches(CHOICES).count(), 1);
        assert!(answer.content.find(STICKER).unwrap() < answer.content.find("**参考来源**").unwrap());
        let terminal: Value = stream.split("\n\n").find_map(|event| {
            if !event.lines().any(|line| line == "event: done") { return None; }
            event.lines().find_map(|line| line.strip_prefix("data: ")).map(|data| serde_json::from_str(data).unwrap())
        }).expect("terminal node");
        assert_eq!(terminal["node"]["content"], answer.content);
        let usage = &answer.metrics.as_ref().unwrap().usage;
        assert_eq!((usage.cached_in, usage.uncached_in, usage.reasoning, usage.output), (5, 25, 2, 8));
        let bills = app.store.usage_between(0, nekochat_store::now_ms() + 1000).unwrap();
        assert_eq!(bills.len(), 1);
        assert_eq!(bills[0].usage.output, 8);
        let search_bills = app.store.list_search_usage(0, nekochat_store::now_ms() + 1000).unwrap();
        assert_eq!(search_bills[0].usage.as_ref().unwrap()["cost"]["currency"], "CNY");
        task.abort();
    }
}

#[tokio::test]
async fn detached_observer_reconnects_without_repeating_search_or_model_requests() {
    let (app, conv, role, state, task) = rig(false, false, true).await;
    let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns"))).json(&json!({"action":"send","content":"public question","character_id":role})).send().await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), state.started.notified()).await.unwrap();
    drop(response);
    let observer = app.client.get(app.url(&format!("/api/conversations/{conv}/turns"))).send().await.unwrap();
    assert_eq!(observer.status(), 200);
    state.release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), observer.text()).await.unwrap().unwrap();
    assert!(result.contains("event: done"), "{result}");
    assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
    assert_eq!(app.store.nodes(&conv).unwrap().len(), 2);
    task.abort();
}

#[tokio::test]
async fn stop_cancels_the_engine_without_a_second_model_request() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig(native, false, true).await;
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns"))).json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), state.started.notified()).await.unwrap();
        let view: Value = app.client.get(app.url(&format!("/api/conversations/{conv}"))).send().await.unwrap().json().await.unwrap();
        let turn_id = view["active_turn_id"].as_str().unwrap();
        assert_eq!(app.client.post(app.url(&format!("/api/conversations/{conv}/turns/{turn_id}/stop"))).send().await.unwrap().status(), 204);
        tokio::time::timeout(Duration::from_secs(5), response.text()).await.unwrap().unwrap();
        assert_eq!(state.cancel_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 1);
        let nodes = app.store.nodes(&conv).unwrap();
        assert_eq!(nodes.last().unwrap().status, nekochat_store::NodeStatus::Interrupted);
        assert_eq!(app.store.list_search_usage(0, nekochat_store::now_ms() + 1000).unwrap()[0].status, "cancelled");
        task.abort();
    }
}

#[tokio::test]
async fn model_cannot_read_a_path_or_url_in_place_of_an_authorized_source() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig(native, true, false).await;
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns"))).json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let stream = response.text().await.unwrap();
        assert!(stream.contains("event: error"), "{stream}");
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 1);
        assert!(app.store.list_search_usage(0, nekochat_store::now_ms() + 1000).unwrap().is_empty());
        task.abort();
    }
}

#[tokio::test]
async fn deleting_during_search_discards_content_but_keeps_already_billed_model_usage() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig(native, false, true).await;
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns"))).json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), state.started.notified()).await.unwrap();
        assert_eq!(app.client.delete(app.url(&format!("/api/conversations/{conv}"))).send().await.unwrap().status(), 204);
        tokio::time::timeout(Duration::from_secs(5), response.text()).await.unwrap().unwrap();
        assert!(app.store.get_conversation(&conv).unwrap().is_none());
        assert!(app.store.nodes(&conv).unwrap().is_empty());
        assert_eq!(state.cancel_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 1);
        let bills = app.store.usage_between(0, nekochat_store::now_ms() + 1000).unwrap();
        assert_eq!(bills.len(), 1, "deleting private content must not erase anonymous provider billing");
        assert_eq!(bills[0].usage.uncached_in, 10);
        assert_eq!(app.store.list_search_usage(0, i64::MAX).unwrap()[0].status, "cancelled");
        task.abort();
    }
}

#[tokio::test]
async fn refused_search_continues_without_evidence_and_preserves_billed_turns() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.failure_status.store(403, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"public question","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: done"),"{events}");
        assert!(!events.contains("event: error"));
        let reply = app.store.nodes(&conv).unwrap().into_iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert_eq!(reply.status, nekochat_store::NodeStatus::Complete);
        assert_eq!(reply.content, UNSEARCHED_ANSWER);
        assert!(!reply.content.contains("参考来源"));
        assert!(!reply.content.contains("[1]"));
        assert_eq!(reply.reasoning.as_deref(), Some("核查公开来源"));
        let usage = &reply.metrics.as_ref().unwrap().usage;
        assert_eq!((usage.cached_in, usage.uncached_in, usage.reasoning, usage.output), (5, 25, 2, 8));
        let bills = app.store.usage_between(0, i64::MAX).unwrap();
        assert_eq!(bills.len(), 1);
        assert_eq!(bills[0].usage.uncached_in, 25);
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let usage = app.store.list_search_usage(0, i64::MAX).unwrap();
        assert_eq!(usage.len(), 1);
        assert!(usage[0].usage.is_none(), "failed searches must not invent known fees");
        assert_eq!(usage[0].status, "error");
        task.abort();
    }
}

#[tokio::test]
async fn unavailable_search_completes_all_parallel_call_ids_without_more_network_requests() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.failure_status.store(503, Ordering::SeqCst);
        state.parallel.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: done"), "{events}");
        let reply = app.store.nodes(&conv).unwrap().into_iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert_eq!(reply.content, UNSEARCHED_ANSWER);
        assert_eq!(reply.metrics.as_ref().unwrap().usage.uncached_in, 25);
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let usage = app.store.list_search_usage(0, i64::MAX).unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].status, "error");
        assert!(usage[0].usage.is_none());
        task.abort();
    }
}

#[tokio::test]
async fn failed_search_disables_tools_and_rejects_a_later_tool_round() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.failure_status.store(403, Ordering::SeqCst);
        state.request_again.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: error"), "{events}");
        assert!(events.contains("模型在工具额度用尽后仍请求工具"), "{events}");
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let bills = app.store.usage_between(0, i64::MAX).unwrap();
        assert_eq!(bills.len(), 1);
        assert_eq!(bills[0].usage.uncached_in, 25);
        task.abort();
    }
}

#[tokio::test]
async fn invalid_search_responses_and_unsafe_sources_still_fail_the_turn() {
    for native in [false, true] {
        for malformed in [false, true] {
            let (app, conv, role, state, task) = rig(native, false, false).await;
            state.malformed_response.store(malformed, Ordering::SeqCst);
            state.unsafe_source.store(!malformed, Ordering::SeqCst);
            let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
                .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
            let events = response.text().await.unwrap();
            assert!(events.contains("event: error"), "{events}");
            assert!(!events.contains("event: done"));
            assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
            assert_eq!(state.model_calls.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }
}

#[tokio::test]
async fn failed_search_skips_parallel_fetch_but_retains_only_previously_received_sources() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.failure_status.store(503, Ordering::SeqCst);
        state.parallel.store(true, Ordering::SeqCst);
        state.parallel_fetch.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: done"), "{events}");
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 2);
        assert_eq!(state.fetch_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let usage = app.store.list_search_usage(0, i64::MAX).unwrap();
        assert_eq!(usage.len(), 2);
        assert_eq!(usage.iter().filter(|record| record.status == "complete").count(), 1);
        assert_eq!(usage.iter().filter(|record| record.status == "error" && record.usage.is_none()).count(), 1);
        let reply = app.store.nodes(&conv).unwrap().into_iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert!(reply.content.contains("https://sqlite.org/wal.html"));
        assert!(!reply.content.contains("file:///"));
        task.abort();
    }
}

#[tokio::test]
async fn parallel_calls_cannot_exceed_the_configured_search_budget() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig(native, false, false).await;
        state.parallel.store(true, Ordering::SeqCst);
        state.expect_budget_exhausted.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: done"), "{events}");
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let usage = app.store.list_search_usage(0, i64::MAX).unwrap();
        assert_eq!(usage.len(), 1);
        assert_eq!(usage[0].status, "complete");
        task.abort();
    }
}

#[tokio::test]
async fn failed_search_does_not_hide_an_invalid_parallel_source_reference() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.failure_status.store(403, Ordering::SeqCst);
        state.parallel.store(true, Ordering::SeqCst);
        state.invalid_parallel_source.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: error"), "{events}");
        assert!(events.contains("来源不属于当前搜索回合"), "{events}");
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.fetch_calls.load(Ordering::SeqCst), 0);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn search_failure_preserves_the_models_own_final_refusal() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.failure_status.store(403, Ordering::SeqCst);
        state.final_refusal.store(true, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: done"), "{events}");
        let reply = app.store.nodes(&conv).unwrap().into_iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert_eq!(reply.content, "我无法回答这个问题。");
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        task.abort();
    }
}

#[tokio::test]
async fn engine_storage_and_malformed_provider_errors_remain_fatal() {
    for native in [false, true] {
        for failure in 1..=3 {
            let (app, conv, role, state, task) = rig(native, false, false).await;
            state.engine_failure.store(failure, Ordering::SeqCst);
            let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
                .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
            let events = response.text().await.unwrap();
            assert!(events.contains("event: error"), "{events}");
            assert!(!events.contains("event: done"));
            assert!(!events.contains("unknown_error_with_private_details"));
            assert_eq!(state.model_calls.load(Ordering::SeqCst), 1);
            let usage = app.store.list_search_usage(0, i64::MAX).unwrap();
            assert_eq!(usage[0].status, "error");
            assert!(usage[0].usage.is_none());
            task.abort();
        }
    }
}

#[tokio::test]
async fn provider_refusal_through_the_engines_502_envelope_continues_without_search() {
    for native in [false, true] {
        let (app, conv, role, state, task) = rig_with_budget(native, false, false, 3).await;
        state.engine_failure.store(4, Ordering::SeqCst);
        let response = app.client.post(app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":role})).send().await.unwrap();
        let events = response.text().await.unwrap();
        assert!(events.contains("event: done"), "{events}");
        let reply = app.store.nodes(&conv).unwrap().into_iter().find(|node| node.role == nekochat_store::NodeRole::Assistant).unwrap();
        assert_eq!(reply.content, UNSEARCHED_ANSWER);
        assert_eq!(reply.metrics.as_ref().unwrap().usage.uncached_in, 25);
        assert_eq!(state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(state.model_calls.load(Ordering::SeqCst), 2);
        let usage = app.store.list_search_usage(0, i64::MAX).unwrap();
        assert_eq!(usage[0].status, "error");
        assert!(usage[0].usage.is_none());
        task.abort();
    }
}
