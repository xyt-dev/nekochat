mod common;

use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}};
use std::time::Duration;
use axum::{Json, Router, body::Body, extract::State, http::StatusCode, response::{IntoResponse, Response}, routing::{get, post}};
use common::{TestApp, TestCfg, PROVIDER_KEY, spawn_app};
use nekochat_store::{NodeRole, NodeStatus};
use serde_json::{Value, json};
use tokio::sync::Notify;

const SERVICE: &str = "0196f817-1eb0-7000-8000-000000000007";
const ANSWER: &str = "本回合未能联网核查。根据已有知识，SQLite WAL 同一时刻只允许一个写入者。";
const REFUSAL: &str = "我无法回答这个问题。";

#[derive(Clone, Copy)]
enum Reply {
    Blank { text: &'static str, usage: bool },
    Answer { text: &'static str, usage: bool },
    Search,
    MalformedTool(u8),
    Disconnect,
    InvalidJson,
    Rejected,
    Wait,
}

struct Fixture {
    native: bool,
    replies: Vec<Reply>,
    requests: Mutex<Vec<Value>>,
    search_calls: AtomicUsize,
    search_succeeds: AtomicBool,
    waiting: Notify,
    release: Notify,
}

async fn model(State(state): State<Arc<Fixture>>, Json(body): Json<Value>) -> Response {
    let index = {
        let mut requests = state.requests.lock().unwrap();
        let index = requests.len();
        requests.push(body);
        index
    };
    let Some(mut reply) = state.replies.get(index).copied() else { return StatusCode::BAD_REQUEST.into_response(); };
    if matches!(reply, Reply::Wait) {
        state.waiting.notify_one();
        state.release.notified().await;
        reply = Reply::Answer { text: ANSWER, usage: true };
    }
    if matches!(reply, Reply::Rejected) {
        return (StatusCode::FORBIDDEN, Json(json!({"error":{"message":"Model request rejected"}}))).into_response();
    }
    if matches!(reply, Reply::InvalidJson) {
        return ([("content-type", "text/event-stream")], "data: {invalid-json}\n\n").into_response();
    }
    let blank = matches!(reply, Reply::Blank { .. });
    let text = match reply { Reply::Blank { text, .. } | Reply::Answer { text, .. } => text, _ => "" };
    let reported_usage = !matches!(reply, Reply::Blank { usage: false, .. } | Reply::Answer { usage: false, .. });
    let mut events = Vec::new();
    if state.native {
        let mut output = Vec::new();
        if blank {
            events.push(json!({"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"reasoning-1","delta":"检查已有信息"}));
            let reasoning = json!({"type":"reasoning","id":"reasoning-1","summary":[],"encrypted_content":"opaque-recovery-state"});
            events.push(json!({"type":"response.output_item.done","output_index":0,"item":reasoning}));
            output.push(reasoning);
        }
        if matches!(reply, Reply::Search) {
            let item = json!({"type":"function_call","id":"function-1","call_id":"lookup-1","name":"web_search","arguments":"{\"query\":\"SQLite WAL\"}"});
            events.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
            output.push(item);
        } else if let Reply::MalformedTool(kind) = reply {
            let mut item = json!({"type":"function_call","id":"malformed","call_id":"lookup-1","name":"web_search","arguments":"{\"query\":\"SQLite WAL\"}"});
            match kind {
                0 => { item.as_object_mut().unwrap().remove("call_id"); },
                1 => item["arguments"] = json!({"query":"SQLite WAL"}),
                2 => item["namespace"] = json!("unauthorized"),
                _ => item["arguments"] = json!("invalid-json"),
            }
            events.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
            output.push(item);
        } else if !matches!(reply, Reply::Disconnect) {
            let output_index = output.len();
            let item = json!({"type":"message","id":format!("message-{index}"),"role":"assistant","status":"completed","content":[{"type":"output_text","text":text}]});
            events.push(json!({"type":"response.output_item.done","output_index":output_index,"item":item}));
            output.push(item);
        }
        let mut terminal = json!({"type":"response.completed","response":{"status":"completed","output":output}});
        if reported_usage {
            terminal["response"]["usage"] = json!({"input_tokens":if index == 0 {10} else {20},"input_tokens_details":{"cached_tokens":if index == 0 {0} else {5}},"output_tokens":if index == 0 {4} else {6},"output_tokens_details":{"reasoning_tokens":if index == 0 {2} else {0}}});
        }
        if matches!(reply, Reply::Disconnect) {
            events.push(json!({"type":"response.usage","usage":{"input_tokens":10,"output_tokens":4}}));
        } else { events.push(terminal); }
    } else {
        if blank { events.push(json!({"choices":[{"delta":{"reasoning_content":"检查已有信息"}}]})); }
        if matches!(reply, Reply::Search) {
            events.push(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"lookup-1","type":"function","function":{"name":"web_search","arguments":"{\"query\":\"SQLite WAL\"}"}}]}}]}));
        } else if let Reply::MalformedTool(kind) = reply {
            let mut calls = json!([{"index":0,"id":"lookup-1","type":"function","function":{"name":"web_search","arguments":"{\"query\":\"SQLite WAL\"}"}}]);
            match kind {
                0 => { calls[0].as_object_mut().unwrap().remove("index"); },
                1 => calls[0]["index"] = json!(-1),
                2 => calls[0]["index"] = json!("0"),
                3 => calls[0]["index"] = json!(0.5),
                _ => calls = json!({"index":0}),
            }
            events.push(json!({"choices":[{"delta":{"tool_calls":calls}}]}));
        } else if !text.is_empty() { events.push(json!({"choices":[{"delta":{"content":text}}]})); }
        if reported_usage {
            events.push(json!({"choices":[],"usage":{"prompt_tokens":if index == 0 {10} else {20},"prompt_tokens_details":{"cached_tokens":if index == 0 {0} else {5}},"completion_tokens":if index == 0 {4} else {6},"completion_tokens_details":{"reasoning_tokens":if index == 0 {2} else {0}}}}));
        }
    }
    let mut stream = events.into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
    if !state.native && !matches!(reply, Reply::Disconnect) { stream.push_str("data: [DONE]\n\n"); }
    Response::builder().header("content-type", "text/event-stream").body(Body::from(stream)).unwrap()
}

async fn services() -> Json<Value> {
    Json(json!([{"id":SERVICE,"name":"Research","kind":"deepseek","enabled":true,"auth_mode":"api_key","model":null,"has_api_key":true,"connected":true,"supports_fetch":true}]))
}

async fn search(State(state): State<Arc<Fixture>>) -> Response {
    state.search_calls.fetch_add(1, Ordering::SeqCst);
    if !state.search_succeeds.load(Ordering::SeqCst) { return StatusCode::FORBIDDEN.into_response(); }
    Json(json!({"query":"SQLite WAL","sources":[{"id":"provider-source","title":"SQLite WAL","url":"https://sqlite.org/wal.html","snippet":"Only one writer at a time.","summary":null,"content":null,"published_at":null}],"summary":null,"retrieved_at_ms":1,"usage":{"requests":1,"cached_in":0,"uncached_in":0,"reasoning":0,"output":0,"cost":{"currency":"CNY","amount":0.005}}})).into_response()
}

struct Rig {
    app: TestApp,
    conv: String,
    role: String,
    state: Arc<Fixture>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.state.release.notify_waiters();
        self.task.abort();
    }
}

async fn rig(native: bool, replies: Vec<Reply>) -> Rig {
    let state = Arc::new(Fixture { native, replies, requests: Mutex::new(Vec::new()), search_calls: AtomicUsize::new(0), search_succeeds: AtomicBool::new(false), waiting: Notify::new(), release: Notify::new() });
    // Only the selected protocol has a route, so an implicit protocol switch cannot pass.
    let router = Router::new().route(if native { "/responses" } else { "/chat/completions" }, post(model))
        .route("/v1/services", get(services)).route("/v1/search", post(search)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let app = spawn_app(TestCfg { upstream: Some(upstream.clone()), admin_password: Some("admin".into()), ..Default::default() }).await;
    app.login_admin("admin").await;
    let response = app.client.put(app.url("/api/admin/search/connection")).json(&json!({"enabled":true,"base_url":upstream,"access_token":"runtime-test","admin_token":"admin-test","max_calls_per_turn":3,"max_results":5,"timeout_secs":10})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let model = app.store.insert_model(&serde_json::from_value(json!({"provider":PROVIDER_KEY,"upstream_id":"fixture","display_name":"Fixture","sort":0,"protocol":if native {"responses"} else {"chat_completions"},"reasoning":{"style":"none","levels":[],"default":null},"vision":false,"context_window":null,"price":{"cached_in":1,"uncached_in":2,"output":3}})).unwrap()).unwrap();
    let role = app.store.create_character(&serde_json::from_value(json!({"name":"research","display_name":"Research","model_id":model.id,"system_prompt":"Answer accurately.","search_service_id":SERVICE})).unwrap()).unwrap();
    let conv = app.store.create_owned_conversation(&app.user_id, Some(&role.id)).unwrap();
    Rig { app, conv: conv.id, role: role.id, state, task }
}

async fn send(rig: &Rig) -> reqwest::Response {
    rig.app.client.post(rig.app.url(&format!("/api/conversations/{}/turns", rig.conv)))
        .json(&json!({"action":"send","content":"Explain SQLite WAL concurrency.","character_id":rig.role})).send().await.unwrap()
}

fn assert_closed_tools(rig: &Rig) {
    let requests = rig.state.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].get("tools").is_some());
    assert!(requests[1].get("tools").is_none());
    assert_eq!(requests[0]["model"], requests[1]["model"]);
    assert_eq!(rig.state.search_calls.load(Ordering::SeqCst), 0);
    if rig.state.native {
        assert!(requests[1]["input"].as_array().unwrap().iter().any(|item| item["encrypted_content"] == "opaque-recovery-state"));
    }
    assert!(rig.app.store.list_search_usage(0, i64::MAX).unwrap().is_empty());
}

#[tokio::test]
async fn completed_empty_tool_round_continues_once_in_the_selected_protocol_and_bills_both_rounds() {
    for native in [false, true] {
        for text in ["", " \n\t"] {
            let rig = rig(native, vec![Reply::Blank { text, usage: true }, Reply::Answer { text: ANSWER, usage: true }]).await;
            let stream = send(&rig).await.text().await.unwrap();
            assert!(stream.contains("event: done"), "{stream}");
            assert!(!stream.contains("event: error"));
            let nodes = rig.app.store.nodes(&rig.conv).unwrap();
            let answer = nodes.iter().find(|node| node.role == NodeRole::Assistant).unwrap();
            assert_eq!(answer.status, NodeStatus::Complete);
            assert_eq!(answer.content.trim(), ANSWER);
            assert!(!answer.content.contains("参考来源"));
            assert_eq!(answer.reasoning.as_deref(), Some("检查已有信息"));
            let usage = &answer.metrics.as_ref().unwrap().usage;
            assert_eq!((usage.cached_in, usage.uncached_in, usage.reasoning, usage.output), (5, 25, 2, 8));
            let bills = rig.app.store.usage_between(0, i64::MAX).unwrap();
            assert_eq!(bills.len(), 1);
            assert_eq!(bills[0].usage.uncached_in, 25);
            assert_eq!(stream.matches("event: start").count(), 1);
            assert_closed_tools(&rig);
        }
    }
}

#[tokio::test]
async fn second_empty_round_stops_and_retains_all_incurred_usage() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: true }, Reply::Blank { text: "", usage: true }]).await;
        let stream = send(&rig).await.text().await.unwrap();
        assert!(stream.contains("event: error"), "{stream}");
        assert!(!stream.contains("event: done"));
        let answer = rig.app.store.nodes(&rig.conv).unwrap().into_iter().find(|node| node.role == NodeRole::Assistant).unwrap();
        assert_eq!(answer.status, NodeStatus::Error);
        assert!(answer.content.trim().is_empty());
        assert_eq!(answer.metrics.unwrap().usage.uncached_in, 25);
        assert_closed_tools(&rig);
    }
}

#[tokio::test]
async fn recovery_preserves_the_models_final_refusal() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: true }, Reply::Answer { text: REFUSAL, usage: true }]).await;
        let stream = send(&rig).await.text().await.unwrap();
        assert!(stream.contains("event: done"), "{stream}");
        let answer = rig.app.store.nodes(&rig.conv).unwrap().into_iter().find(|node| node.role == NodeRole::Assistant).unwrap();
        assert_eq!(answer.content, REFUSAL);
        assert_eq!(answer.status, NodeStatus::Complete);
        assert_closed_tools(&rig);
    }
}

#[tokio::test]
async fn missing_first_round_usage_remains_unknown_instead_of_becoming_a_known_price() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: false }, Reply::Answer { text: ANSWER, usage: true }]).await;
        assert!(send(&rig).await.text().await.unwrap().contains("event: done"));
        let bills = rig.app.store.usage_between(0, i64::MAX).unwrap();
        assert_eq!(bills.len(), 1);
        assert_eq!((bills[0].usage.cached_in, bills[0].usage.uncached_in), (5, 15));
        assert!(bills[0].cost_usd.is_none());
        assert_closed_tools(&rig);
    }
}

#[tokio::test]
async fn broken_or_rejected_model_requests_do_not_enter_empty_round_recovery() {
    for native in [false, true] {
        for reply in [Reply::Disconnect, Reply::InvalidJson, Reply::Rejected, Reply::Answer { text: REFUSAL, usage: true }] {
            let rig = rig(native, vec![reply]).await;
            let stream = send(&rig).await.text().await.unwrap();
            if matches!(reply, Reply::Answer { .. }) { assert!(stream.contains("event: done")); }
            else { assert!(stream.contains("event: error"), "{stream}"); }
            assert_eq!(rig.state.requests.lock().unwrap().len(), 1);
            assert_eq!(rig.state.search_calls.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn disabled_recovery_tools_cannot_be_executed_by_a_later_model_round() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: true }, Reply::Search]).await;
        let stream = send(&rig).await.text().await.unwrap();
        assert!(stream.contains("event: error"));
        assert_eq!(rig.state.requests.lock().unwrap().len(), 2);
        assert_eq!(rig.state.search_calls.load(Ordering::SeqCst), 0);
        assert!(rig.app.store.list_search_usage(0, i64::MAX).unwrap().is_empty());
    }
}

#[tokio::test]
async fn empty_answer_after_search_refusal_does_not_start_a_third_model_request() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Search, Reply::Blank { text: "", usage: true }]).await;
        let stream = send(&rig).await.text().await.unwrap();
        assert!(stream.contains("event: error"));
        assert_eq!(rig.state.requests.lock().unwrap().len(), 2);
        assert_eq!(rig.state.search_calls.load(Ordering::SeqCst), 1);
        assert_eq!(rig.app.store.list_search_usage(0, i64::MAX).unwrap()[0].status, "error");
    }
}

#[tokio::test]
async fn stop_during_recovery_preserves_the_spent_round_and_does_not_retry() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: true }, Reply::Wait]).await;
        let response = send(&rig).await;
        tokio::time::timeout(Duration::from_secs(5), rig.state.waiting.notified()).await.unwrap();
        let view: Value = rig.app.client.get(rig.app.url(&format!("/api/conversations/{}", rig.conv))).send().await.unwrap().json().await.unwrap();
        let turn = view["active_turn_id"].as_str().unwrap();
        assert_eq!(rig.app.client.post(rig.app.url(&format!("/api/conversations/{}/turns/{turn}/stop", rig.conv))).send().await.unwrap().status(), 204);
        let _ = tokio::time::timeout(Duration::from_secs(5), response.text()).await.unwrap().unwrap();
        rig.state.release.notify_one();
        let answer = rig.app.store.nodes(&rig.conv).unwrap().into_iter().find(|node| node.role == NodeRole::Assistant).unwrap();
        assert_eq!(answer.status, NodeStatus::Interrupted);
        assert_eq!(rig.app.store.usage_between(0, i64::MAX).unwrap()[0].usage.uncached_in, 10);
        assert_closed_tools(&rig);
    }
}

#[tokio::test]
async fn deleting_during_recovery_discards_content_and_preserves_spent_usage() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: true }, Reply::Wait]).await;
        let response = send(&rig).await;
        tokio::time::timeout(Duration::from_secs(5), rig.state.waiting.notified()).await.unwrap();
        assert_eq!(rig.app.client.delete(rig.app.url(&format!("/api/conversations/{}", rig.conv))).send().await.unwrap().status(), 204);
        let _ = tokio::time::timeout(Duration::from_secs(5), response.text()).await.unwrap().unwrap();
        rig.state.release.notify_one();
        assert!(rig.app.store.get_conversation(&rig.conv).unwrap().is_none());
        assert!(rig.app.store.nodes(&rig.conv).unwrap().is_empty());
        assert_eq!(rig.app.store.usage_between(0, i64::MAX).unwrap()[0].usage.uncached_in, 10);
        assert_closed_tools(&rig);
    }
}

#[tokio::test]
async fn observer_reconnect_during_recovery_only_observes_the_existing_generation() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Blank { text: "", usage: true }, Reply::Wait]).await;
        let response = send(&rig).await;
        tokio::time::timeout(Duration::from_secs(5), rig.state.waiting.notified()).await.unwrap();
        drop(response);
        let observer = rig.app.client.get(rig.app.url(&format!("/api/conversations/{}/turns", rig.conv))).send().await.unwrap();
        rig.state.release.notify_one();
        let stream = tokio::time::timeout(Duration::from_secs(5), observer.text()).await.unwrap().unwrap();
        assert!(stream.contains("event: done"), "{stream}");
        assert_eq!(rig.app.store.nodes(&rig.conv).unwrap().len(), 2);
        assert_closed_tools(&rig);
    }
}

#[tokio::test]
async fn a_later_empty_round_retains_real_sources_without_repeating_search() {
    for native in [false, true] {
        let rig = rig(native, vec![Reply::Search, Reply::Blank { text: "", usage: true }, Reply::Answer { text: "根据已取得的资料，WAL 同时只允许一个写入者。[1]", usage: true }]).await;
        rig.state.search_succeeds.store(true, Ordering::SeqCst);
        let stream = send(&rig).await.text().await.unwrap();
        assert!(stream.contains("event: done"), "{stream}");
        let answer = rig.app.store.nodes(&rig.conv).unwrap().into_iter().find(|node| node.role == NodeRole::Assistant).unwrap();
        assert_eq!(answer.status, NodeStatus::Complete);
        assert_eq!(answer.content.matches("https://sqlite.org/wal.html").count(), 1);
        assert_eq!(answer.content.matches("**参考来源**").count(), 1);
        assert_eq!(answer.metrics.unwrap().usage.uncached_in, 40);
        assert_eq!(rig.state.requests.lock().unwrap().len(), 3);
        assert!(rig.state.requests.lock().unwrap()[2].get("tools").is_none());
        assert_eq!(rig.state.search_calls.load(Ordering::SeqCst), 1);
        let search_usage = rig.app.store.list_search_usage(0, i64::MAX).unwrap();
        assert_eq!(search_usage.len(), 1);
        assert_eq!(search_usage[0].status, "complete");
        assert_eq!(search_usage[0].usage.as_ref().unwrap()["cost"]["amount"], 0.005);
    }
}

#[tokio::test]
async fn malformed_tool_calls_fail_without_recovery_and_retain_reported_usage() {
    for native in [false, true] {
        for kind in 0..(if native { 4 } else { 5 }) {
            let rig = rig(native, vec![Reply::MalformedTool(kind), Reply::Answer { text: ANSWER, usage: true }]).await;
            let stream = send(&rig).await.text().await.unwrap();
            assert!(stream.contains("event: error"), "native={native}, kind={kind}, {stream}");
            assert!(!stream.contains("event: done"));
            assert_eq!(rig.state.requests.lock().unwrap().len(), 1);
            assert_eq!(rig.state.search_calls.load(Ordering::SeqCst), 0);
            let usage = rig.app.store.usage_between(0, i64::MAX).unwrap();
            assert_eq!(usage[0].usage.uncached_in, 10);
        }
    }
}
