mod common;

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use common::{
    MockScript, MockUpstream, PROVIDER_KEY, RecordedRequest, TestApp, TestCfg, spawn_app,
    spawn_mock_upstream,
};
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use nekochat_core::{Price, Protocol, ReasoningSpec, ReasoningStyle, Usage};
use nekochat_server::chat::cost_usd;
use nekochat_store::{
    DEFAULT_TITLE, ModelSpec, Node, NodeRole, NodeStatus, SettingKey, UsagePurpose, new_id, now_ms,
};
use serde_json::{Value, json};

const MODEL_LABEL: &str = "Test Model · Mock";

fn price() -> Price {
    Price {
        cached_in: Some(0.1),
        uncached_in: Some(1.0),
        output: Some(2.0),
    }
}

fn spec(upstream_id: &str, style: ReasoningStyle) -> ModelSpec {
    let levels: Vec<String> = match style {
        ReasoningStyle::None => Vec::new(),
        _ => vec!["high".into()],
    };
    ModelSpec {
        provider: PROVIDER_KEY.into(),
        upstream_id: upstream_id.into(),
        display_name: "Test Model".into(),
        sort: 0,
        protocol: Protocol::ChatCompletions,
        reasoning: ReasoningSpec {
            default: levels.first().cloned(),
            levels,
            style,
        },
        vision: false,
        context_window: None,
        price: price(),
    }
}

fn sse_chunk(v: Value) -> Vec<u8> {
    format!("data: {v}\n\n").into_bytes()
}

fn text_chunk(t: &str) -> Vec<u8> {
    sse_chunk(json!({"choices": [{"delta": {"content": t}}]}))
}

fn done_chunk() -> Vec<u8> {
    b"data: [DONE]\n\n".to_vec()
}

/// DeepSeek-style reply: one reasoning delta, one text delta, then usage.
fn deepseek_reply(reasoning: &str, text: &str) -> MockScript {
    MockScript::sse(&[
        json!({"choices": [{"delta": {"reasoning_content": reasoning}}]}),
        json!({"choices": [{"delta": {"content": text}}]}),
        json!({"choices": [], "usage": {
            "prompt_tokens": 1204,
            "prompt_cache_hit_tokens": 896,
            "prompt_cache_miss_tokens": 308,
            "completion_tokens": 426,
            "completion_tokens_details": {"reasoning_tokens": 140}
        }}),
    ])
}

/// One text delta, then nothing for 30 s.
fn slow_script() -> MockScript {
    MockScript {
        status: 200,
        chunks: vec![
            (Duration::ZERO, text_chunk("x")),
            (Duration::from_secs(30), text_chunk("y")),
            (Duration::ZERO, done_chunk()),
        ],
        models: Vec::new(),
    }
}

/// Incremental reader of the app's `text/event-stream` responses.
struct SseReader {
    body: BoxStream<'static, reqwest::Result<Bytes>>,
    buf: Vec<u8>,
}

impl SseReader {
    fn new(resp: reqwest::Response) -> SseReader {
        assert_eq!(resp.status(), 200);
        assert_eq!(resp.headers()["content-type"], "text/event-stream");
        SseReader {
            body: resp.bytes_stream().boxed(),
            buf: Vec::new(),
        }
    }

    /// Next named event, skipping keep-alive comments; `None` once the stream ends.
    async fn next(&mut self) -> Option<(String, Value)> {
        loop {
            if let Some(end) = self.buf.windows(2).position(|w| w == b"\n\n") {
                let block = String::from_utf8(self.buf.drain(..end + 2).collect()).unwrap();
                let mut name = None;
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        name = Some(v.trim().to_owned());
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data.push_str(v.strip_prefix(' ').unwrap_or(v));
                    }
                }
                match name {
                    Some(name) => return Some((name, serde_json::from_str(&data).unwrap())),
                    None => continue,
                }
            }
            let chunk = self.body.next().await?.unwrap();
            self.buf.extend_from_slice(&chunk);
        }
    }

    async fn collect(mut self) -> Vec<(String, Value)> {
        let mut events = Vec::new();
        while let Some(e) = self.next().await {
            events.push(e);
        }
        events
    }
}

fn names(events: &[(String, Value)]) -> Vec<&str> {
    events.iter().map(|(n, _)| n.as_str()).collect()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

struct Fixture {
    app: TestApp,
    mock: MockUpstream,
    model_id: String,
}

async fn fixture(script: MockScript) -> Fixture {
    fixture_with(script, None).await
}

async fn fixture_with(script: MockScript, session_header: Option<&str>) -> Fixture {
    let mock = spawn_mock_upstream(script).await;
    let app = spawn_app(TestCfg {
        upstream: Some(mock.base_url.clone()),
        session_header: session_header.map(Into::into),
        ..Default::default()
    })
    .await;
    let model_id = app
        .store
        .insert_model(&spec("up-model", ReasoningStyle::DeepSeek))
        .unwrap()
        .id;
    Fixture {
        app,
        mock,
        model_id,
    }
}

impl Fixture {
    async fn create_conv(&self) -> String {
        let r = self
            .app
            .client
            .post(self.app.url("/api/conversations"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 201);
        let v: Value = r.json().await.unwrap();
        v["id"].as_str().unwrap().to_owned()
    }

    async fn post(&self, conv: &str, body: Value) -> reqwest::Response {
        self.app
            .client
            .post(self.app.url(&format!("/api/conversations/{conv}/turns")))
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    fn send_body(&self, content: &str) -> Value {
        json!({"action": "send", "content": content, "model_id": self.model_id, "effort": null})
    }

    async fn turn(&self, conv: &str, body: Value) -> Vec<(String, Value)> {
        SseReader::new(self.post(conv, body).await).collect().await
    }

    async fn send(&self, conv: &str, content: &str) -> Vec<(String, Value)> {
        self.turn(conv, self.send_body(content)).await
    }

    async fn conversation(&self, conv: &str) -> Value {
        let r = self
            .app
            .client
            .get(self.app.url(&format!("/api/conversations/{conv}")))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.mock.requests.lock().unwrap().clone()
    }

    fn idle(&self) -> bool {
        self.app.running.lock().unwrap().is_empty()
    }
}

#[tokio::test]
async fn send_streams_and_persists() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let content = format!("{}{}", "喵".repeat(20), "abcdefghijklmnopqrstuvwxyz");
    let events = f.send(&conv, &content).await;
    assert_eq!(names(&events), ["start", "reasoning", "text", "done"]);

    let start = &events[0].1;
    let user = &start["user_node"];
    assert_eq!(user["role"], "user");
    assert_eq!(user["content"], content.as_str());
    assert_eq!(user["parent_id"], Value::Null);
    assert_eq!(start["model_label"], MODEL_LABEL);
    assert_eq!(events[1].1, json!({"t": "想"}));
    assert_eq!(events[2].1, json!({"t": "喵~"}));

    let node = &events[3].1["node"];
    assert_eq!(node["id"], start["assistant_node_id"]);
    assert_eq!(node["parent_id"], user["id"]);
    assert_eq!(node["role"], "assistant");
    assert_eq!(node["status"], "complete");
    assert_eq!(node["content"], "喵~");
    assert_eq!(node["reasoning"], "想");
    assert_eq!(node["model_label"], MODEL_LABEL);
    assert_eq!(node["effort"], "high");
    let metrics = &node["metrics"];
    assert_eq!(
        metrics["usage"],
        json!({"cached_in": 896, "uncached_in": 308, "reasoning": 140, "output": 286})
    );
    assert!(metrics["ttft_ms"].is_u64());
    assert!(metrics["total_ms"].is_u64());

    let v = f.conversation(&conv).await;
    assert_eq!(v["leaf_id"], node["id"]);
    assert_eq!(v["title"], format!("{}abcdefghij", "喵".repeat(20)));

    let usage = f.app.store.usage_between(0, i64::MAX).unwrap();
    assert_eq!(usage.len(), 1);
    let rec = &usage[0];
    assert_eq!(
        rec.cost_usd,
        Some((896.0 * 0.1 + 308.0 * 1.0 + 426.0 * 2.0) / 1e6)
    );
    assert_eq!(
        (
            rec.provider.as_str(),
            rec.upstream_id.as_str(),
            rec.model_label.as_str(),
            rec.purpose
        ),
        (PROVIDER_KEY, "up-model", MODEL_LABEL, UsagePurpose::Chat)
    );

    let reqs = f.requests();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].body["model"], "up-model");
    assert_eq!(
        reqs[0].body["messages"],
        json!([{"role": "user", "content": content}])
    );
    assert_eq!(reqs[0].body["reasoning_effort"], "high");
    assert!(f.idle());
}

#[tokio::test]
async fn upstream_request_carries_system_prompt_and_history_without_reasoning() {
    let f = fixture_with(
        deepseek_reply("REASONING-SECRET", "第一轮回复"),
        Some("x-session-affinity"),
    )
    .await;
    let ch = f
        .app
        .store
        .create_character("Neko", "你是一只猫娘。")
        .unwrap();
    f.app
        .store
        .set_setting(SettingKey::DefaultCharacter, Some(&ch.id))
        .unwrap();
    let conv = f.create_conv().await;
    assert_eq!(names(&f.send(&conv, "第一句").await).last(), Some(&"done"));
    assert_eq!(names(&f.send(&conv, "第二句").await).last(), Some(&"done"));

    let reqs = f.requests();
    assert_eq!(reqs.len(), 2);
    let body = &reqs[1].body;
    assert_eq!(
        body["messages"],
        json!([
            {"role": "system", "content": "你是一只猫娘。"},
            {"role": "user", "content": "第一句"},
            {"role": "assistant", "content": "第一轮回复"},
            {"role": "user", "content": "第二句"},
        ])
    );
    assert!(!body.to_string().contains("REASONING-SECRET"));
    let key = f
        .app
        .store
        .get_conversation(&conv)
        .unwrap()
        .unwrap()
        .session_key;
    assert_eq!(reqs[1].headers["x-session-affinity"], key.as_str());
    assert!(
        reqs[1].headers["user-agent"]
            .to_str()
            .unwrap()
            .starts_with("nekochat/")
    );
}

#[tokio::test]
async fn regenerate_adds_sibling_reply() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let first = f.send(&conv, "你好").await;
    let user_id = first[0].1["user_node"]["id"].as_str().unwrap().to_owned();
    let old_reply = first[3].1["node"]["id"].as_str().unwrap().to_owned();

    let events = f
        .turn(
            &conv,
            json!({"action": "regenerate", "node_id": user_id, "model_id": f.model_id, "effort": null}),
        )
        .await;
    assert_eq!(names(&events), ["start", "reasoning", "text", "done"]);
    assert_eq!(events[0].1["user_node"], Value::Null);
    let new_reply = events[3].1["node"]["id"].as_str().unwrap().to_owned();
    assert_eq!(events[3].1["node"]["parent_id"], user_id.as_str());
    assert_ne!(new_reply, old_reply);

    let v = f.conversation(&conv).await;
    let replies: BTreeSet<&str> = v["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n["parent_id"] == user_id.as_str())
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        replies,
        BTreeSet::from([old_reply.as_str(), new_reply.as_str()])
    );
    assert_eq!(v["leaf_id"], new_reply.as_str());
    assert_eq!(
        f.requests()[1].body["messages"],
        json!([{"role": "user", "content": "你好"}])
    );
}

#[tokio::test]
async fn edit_adds_sibling_user_message() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    f.send(&conv, "一").await;
    let second = f.send(&conv, "二").await;
    let original = &second[0].1["user_node"];

    let events = f
        .turn(
            &conv,
            json!({"action": "edit", "node_id": original["id"], "content": "二改", "model_id": f.model_id, "effort": "high"}),
        )
        .await;
    assert_eq!(names(&events), ["start", "reasoning", "text", "done"]);
    let edited = &events[0].1["user_node"];
    assert_ne!(edited["id"], original["id"]);
    assert_eq!(edited["parent_id"], original["parent_id"]);
    assert_eq!(edited["content"], "二改");
    let reply = &events[3].1["node"];
    assert_eq!(reply["parent_id"], edited["id"]);
    assert_eq!(f.conversation(&conv).await["leaf_id"], reply["id"]);
    assert_eq!(
        f.requests()[2].body["messages"],
        json!([
            {"role": "user", "content": "一"},
            {"role": "assistant", "content": "喵~"},
            {"role": "user", "content": "二改"},
        ])
    );
}

#[tokio::test]
async fn upstream_error_is_reported_and_persisted() {
    let f = fixture(MockScript {
        status: 429,
        chunks: vec![(
            Duration::ZERO,
            br#"{"error":{"message":"rate limited"}}"#.to_vec(),
        )],
        models: Vec::new(),
    })
    .await;
    let conv = f.create_conv().await;
    let events = f.send(&conv, "在吗").await;
    assert_eq!(names(&events), ["start", "error"]);
    let message = events[1].1["message"].as_str().unwrap();
    assert!(message.contains("429"), "{message}");
    assert!(message.contains("rate limited"), "{message}");
    assert_eq!(events[1].1["node"], Value::Null);
    assert!(f.app.store.usage_between(0, i64::MAX).unwrap().is_empty());
    let first_user = events[0].1["user_node"]["id"].clone();

    // The process still serves the next turn, appended under the unanswered user message.
    let events = f.send(&conv, "还在吗").await;
    assert_eq!(names(&events), ["start", "error"]);
    let second_user = &events[0].1["user_node"];
    assert_eq!(second_user["parent_id"], first_user);
    let nodes = f.app.store.nodes(&conv).unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(nodes.iter().all(|n| n.role == NodeRole::User));
    assert_eq!(f.conversation(&conv).await["leaf_id"], second_user["id"]);
    assert!(f.idle());
}

#[tokio::test]
async fn midstream_error_keeps_partial_and_is_skipped_in_context() {
    // Reasoning only, then the body ends without `[DONE]`.
    let f = fixture(MockScript {
        status: 200,
        chunks: vec![(
            Duration::ZERO,
            sse_chunk(json!({"choices": [{"delta": {"reasoning_content": "半截思考"}}]})),
        )],
        models: Vec::new(),
    })
    .await;
    let conv = f.create_conv().await;
    let events = f.send(&conv, "一").await;
    assert_eq!(names(&events), ["start", "reasoning", "error"]);
    let error = &events[2].1;
    assert!(!error["message"].as_str().unwrap().is_empty());
    let node = &error["node"];
    assert_eq!(node["status"], "error");
    assert_eq!(node["content"], "");
    assert_eq!(node["reasoning"], "半截思考");
    assert_eq!(node["id"], events[0].1["assistant_node_id"]);
    assert_eq!(f.conversation(&conv).await["leaf_id"], node["id"]);

    f.send(&conv, "二").await;
    assert_eq!(
        f.requests()[1].body["messages"],
        json!([
            {"role": "user", "content": "一"},
            {"role": "user", "content": "二"},
        ])
    );
}

/// Inserts a node under `parent` (which becomes the leaf) and returns its id.
fn store_node(
    f: &Fixture,
    conv: &str,
    parent: Option<&str>,
    role: NodeRole,
    content: &str,
    reasoning: Option<&str>,
    status: NodeStatus,
) -> String {
    let node = Node {
        id: new_id(),
        conversation_id: conv.into(),
        parent_id: parent.map(Into::into),
        role,
        content: content.into(),
        reasoning: reasoning.map(Into::into),
        model_id: None,
        model_label: None,
        effort: None,
        status,
        metrics: None,
        created_at: now_ms(),
    };
    f.app.store.insert_node(&node).unwrap();
    node.id
}

#[tokio::test]
async fn empty_assistant_replies_are_skipped_in_context() {
    use NodeRole::{Assistant, User};
    use NodeStatus::{Complete, Interrupted};
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let u1 = store_node(&f, &conv, None, User, "一", None, Complete);
    // Client left during the thinking phase: reasoning only.
    let a1 = store_node(
        &f,
        &conv,
        Some(&u1),
        Assistant,
        "",
        Some("THOUGHT-ONLY"),
        Interrupted,
    );
    let u2 = store_node(&f, &conv, Some(&a1), User, "二", None, Complete);
    let a2 = store_node(&f, &conv, Some(&u2), Assistant, "回答", None, Complete);
    let u3 = store_node(&f, &conv, Some(&a2), User, "三", None, Complete);
    store_node(&f, &conv, Some(&u3), Assistant, "", None, Complete);

    assert_eq!(names(&f.send(&conv, "四").await).last(), Some(&"done"));
    let body = &f.requests()[0].body;
    assert_eq!(
        body["messages"],
        json!([
            {"role": "user", "content": "一"},
            {"role": "user", "content": "二"},
            {"role": "assistant", "content": "回答"},
            {"role": "user", "content": "三"},
            {"role": "user", "content": "四"},
        ])
    );
    assert!(!body.to_string().contains("THOUGHT-ONLY"));
}

#[tokio::test]
async fn client_disconnect_persists_interrupted_partial() {
    let f = fixture(MockScript {
        status: 200,
        chunks: vec![
            (Duration::ZERO, text_chunk("第一段")),
            (Duration::from_secs(2), text_chunk("第二段")),
            (Duration::ZERO, done_chunk()),
        ],
        models: Vec::new(),
    })
    .await;
    let conv = f.create_conv().await;
    let mut sse = SseReader::new(f.post(&conv, f.send_body("在吗")).await);
    assert_eq!(sse.next().await.unwrap().0, "start");
    assert_eq!(
        sse.next().await.unwrap(),
        ("text".to_owned(), json!({"t": "第一段"}))
    );
    drop(sse);

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let reply = f
            .app
            .store
            .nodes(&conv)
            .unwrap()
            .into_iter()
            .find(|n| n.role == NodeRole::Assistant);
        if let Some(reply) = reply
            && f.idle()
        {
            assert_eq!(reply.status, NodeStatus::Interrupted);
            assert_eq!(reply.content, "第一段");
            break;
        }
        assert!(Instant::now() < deadline, "no interrupted reply within 3 s");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn delete_during_stream_stops_and_leaves_nothing() {
    const MARKER: &str = "删除标记-MARKER-7F3A";
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut sse = SseReader::new(f.post(&conv, f.send_body(MARKER)).await);
    assert_eq!(sse.next().await.unwrap().0, "start");

    let r = f
        .app
        .client
        .delete(f.app.url(&format!("/api/conversations/{conv}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    tokio::time::timeout(Duration::from_secs(3), async {
        while sse.next().await.is_some() {}
    })
    .await
    .expect("SSE did not end within 3 s");
    assert!(f.idle());

    let r = f
        .app
        .client
        .get(f.app.url(&format!("/api/conversations/{conv}")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    for file in ["nekochat.db", "nekochat.db-wal"] {
        let p = f.app.dir.path().join(file);
        if p.exists() {
            assert!(
                !contains(&std::fs::read(&p).unwrap(), MARKER.as_bytes()),
                "{file}"
            );
        }
    }
    assert!(f.app.store.usage_between(0, i64::MAX).unwrap().is_empty());
}

#[tokio::test]
async fn clear_during_stream_stops_and_keeps_conversation() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut sse = SseReader::new(f.post(&conv, f.send_body("清空我")).await);
    assert_eq!(sse.next().await.unwrap().0, "start");

    let r = f
        .app
        .client
        .post(f.app.url(&format!("/api/conversations/{conv}/clear")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    tokio::time::timeout(Duration::from_secs(3), async {
        while sse.next().await.is_some() {}
    })
    .await
    .expect("SSE did not end within 3 s");
    assert!(f.idle());
    let v = f.conversation(&conv).await;
    assert_eq!(v["leaf_id"], Value::Null);
    assert_eq!(v["nodes"], json!([]));
}

#[tokio::test]
async fn double_submit_is_conflict() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut sse = SseReader::new(f.post(&conv, f.send_body("一")).await);
    assert_eq!(sse.next().await.unwrap().0, "start");
    assert_eq!(sse.next().await.unwrap().0, "text");

    let r = f.post(&conv, f.send_body("二")).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].is_string());
    assert_eq!(f.requests().len(), 1);
    assert_eq!(f.app.store.nodes(&conv).unwrap().len(), 1);
}

#[tokio::test]
async fn empty_input_is_400_and_no_upstream_call() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let user = store_node(
        &f,
        &conv,
        None,
        NodeRole::User,
        "原话",
        None,
        NodeStatus::Complete,
    );
    for content in ["", " \n\t　"] {
        for body in [
            json!({"action": "send", "content": content, "model_id": f.model_id, "effort": null}),
            json!({"action": "edit", "node_id": user, "content": content, "model_id": f.model_id, "effort": null}),
        ] {
            let r = f.post(&conv, body.clone()).await;
            assert_eq!(r.status(), 400, "{body}");
            let v: Value = r.json().await.unwrap();
            assert!(v["error"].is_string());
        }
    }
    assert_eq!(f.requests().len(), 0);
    assert_eq!(f.app.store.nodes(&conv).unwrap().len(), 1);
    assert_eq!(f.conversation(&conv).await["title"], DEFAULT_TITLE);
    assert!(f.idle());
}

#[tokio::test]
async fn effort_outside_levels_is_400_and_no_upstream_call() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let plain = f
        .app
        .store
        .insert_model(&spec("plain-model", ReasoningStyle::None))
        .unwrap()
        .id;
    let conv = f.create_conv().await;
    for (model_id, effort) in [(f.model_id.as_str(), "max"), (plain.as_str(), "high")] {
        let r = f
            .post(
                &conv,
                json!({"action": "send", "content": "hi", "model_id": model_id, "effort": effort}),
            )
            .await;
        assert_eq!(r.status(), 400, "{model_id} {effort}");
        let v: Value = r.json().await.unwrap();
        assert!(v["error"].is_string());
    }
    assert_eq!(f.requests().len(), 0);
    assert!(f.app.store.nodes(&conv).unwrap().is_empty());
    assert!(f.idle());
}

#[tokio::test]
async fn unlisted_model_is_400() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let r = f
        .post(
            &conv,
            json!({"action": "send", "content": "hi", "model_id": "no-such-model", "effort": null}),
        )
        .await;
    assert_eq!(r.status(), 400);
    let r = f.post("no-such-conversation", f.send_body("hi")).await;
    assert_eq!(r.status(), 404);
    assert_eq!(f.requests().len(), 0);
    assert!(f.idle());
}

#[tokio::test]
async fn regenerate_and_edit_need_a_user_node() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let events = f.send(&conv, "你好").await;
    let reply = events[3].1["node"]["id"].clone();
    for node_id in [reply, json!("no-such-node")] {
        for body in [
            json!({"action": "regenerate", "node_id": node_id, "model_id": f.model_id, "effort": null}),
            json!({"action": "edit", "node_id": node_id, "content": "改", "model_id": f.model_id, "effort": null}),
        ] {
            let r = f.post(&conv, body.clone()).await;
            assert_eq!(r.status(), 400, "{body}");
        }
    }
    assert_eq!(f.requests().len(), 1);
    assert!(f.idle());
}

#[test]
fn cost_requires_all_prices() {
    let u = Usage {
        cached_in: 896,
        uncached_in: 308,
        reasoning: 140,
        output: 286,
        ..Default::default()
    };
    assert_eq!(
        cost_usd(&price(), &u),
        Some((896.0 * 0.1 + 308.0 * 1.0 + 426.0 * 2.0) / 1e6)
    );
    let mut p = price();
    p.output = None;
    assert_eq!(cost_usd(&p, &u), None);
}
