mod common;

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};
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
    DEFAULT_TITLE, ModelSpec, Node, NodeRole, NodeStatus, UsagePurpose, new_id, now_ms,
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
    character_id: String,
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
    let character_id = app
        .store
        .create_character(
            &serde_json::from_value(json!({
                "name":"Test role", "display_name":"Test role", "model_id":model_id
            }))
            .unwrap(),
        )
        .unwrap()
        .id;
    Fixture {
        app,
        mock,
        model_id,
        character_id,
    }
}

impl Fixture {
    async fn create_conv(&self) -> String {
        let r = self
            .app
            .client
            .post(self.app.url("/api/conversations"))
            .json(&json!({}))
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
        json!({"action": "send", "content": content, "character_id": self.character_id})
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
    assert_eq!(start["character_name"], "Test role");
    assert_eq!(events[1].1, json!({"t": "想"}));
    assert_eq!(events[2].1, json!({"t": "喵~"}));

    let node = &events[3].1["node"];
    assert_eq!(node["id"], start["assistant_node_id"]);
    assert_eq!(node["parent_id"], user["id"]);
    assert_eq!(node["role"], "assistant");
    assert_eq!(node["status"], "complete");
    assert_eq!(node["content"], "喵~");
    assert_eq!(node["reasoning"], "想");
    assert_eq!(node["character_name"], "Test role");
    assert!(node.get("effort").is_none());
    assert!(node.get("model_label").is_none());
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
    let mut role = f
        .app
        .store
        .get_character(&f.character_id)
        .unwrap()
        .unwrap()
        .spec;
    role.system_prompt = "你是一只猫娘。".into();
    f.app
        .store
        .update_character(&f.character_id, &role)
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
            json!({"action": "regenerate", "node_id": user_id, "character_id": f.character_id}),
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
            json!({"action": "edit", "node_id": original["id"], "content": "二改", "character_id": f.character_id}),
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
        character_id: None,
        character_name: None,
        character_avatar: None,
        metrics: None,
        created_at: now_ms(),
        attachments: vec![],
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
async fn client_disconnect_leaves_core_generation_running_to_completion() {
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
            assert_eq!(reply.status, NodeStatus::Complete);
            assert_eq!(reply.content, "第一段第二段");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "core did not complete the detached reply within 3 s"
        );
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
async fn clear_during_stream_requires_explicit_stop_then_clears_history() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut sse = SseReader::new(f.post(&conv, f.send_body("清空我")).await);
    let start = sse.next().await.unwrap();
    let id = start.1["assistant_node_id"].as_str().unwrap();

    let r = f
        .app
        .client
        .post(f.app.url(&format!("/api/conversations/{conv}/clear")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    assert!(!f.idle());
    assert_eq!(
        f.app
            .client
            .post(
                f.app
                    .url(&format!("/api/conversations/{conv}/turns/{id}/stop"))
            )
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while sse.next().await.is_some() {}
    })
    .await
    .expect("SSE did not end within 3 s");
    assert!(f.idle());
    assert_eq!(
        f.app
            .client
            .post(f.app.url(&format!("/api/conversations/{conv}/clear")))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
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
async fn send_after_done_is_appended_under_the_reply() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let first = f.send(&conv, "一").await;
    let second = f.send(&conv, "二").await;
    assert_eq!(
        second[0].1["user_node"]["parent_id"],
        first[3].1["node"]["id"]
    );
    assert_eq!(f.conversation(&conv).await["title"], "一");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_sends_keep_a_single_branch() {
    // Sends only: every accepted turn extends the current branch, so the tree stays a chain
    // (no node has two children) and the title comes from the first accepted message. Workers
    // retry on 409 at once, so requests keep arriving while each turn saves its reply.
    const TURNS: usize = 30;
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let accepted = AtomicUsize::new(0);
    let workers = (0..8).map(|w| {
        let (f, conv, accepted) = (&f, conv.as_str(), &accepted);
        async move {
            let mut attempt = 0;
            while accepted.load(Ordering::SeqCst) < TURNS {
                attempt += 1;
                let r = f
                    .post(conv, f.send_body(&format!("消息{w}-{attempt}")))
                    .await;
                match r.status().as_u16() {
                    409 => {}
                    200 => {
                        SseReader::new(r).collect().await;
                        accepted.fetch_add(1, Ordering::SeqCst);
                    }
                    s => panic!("unexpected status {s}"),
                }
            }
        }
    });
    futures_util::future::join_all(workers).await;
    assert!(f.idle());

    let nodes = f.app.store.nodes(&conv).unwrap();
    let users = nodes.iter().filter(|n| n.role == NodeRole::User).count();
    assert_eq!(users, accepted.load(Ordering::SeqCst));
    let mut children: HashMap<Option<&str>, usize> = HashMap::new();
    for n in &nodes {
        *children.entry(n.parent_id.as_deref()).or_default() += 1;
    }
    let branched: Vec<_> = children.iter().filter(|(_, c)| **c > 1).collect();
    assert!(
        branched.is_empty(),
        "nodes with several children: {branched:?}"
    );
    let root = nodes.iter().find(|n| n.parent_id.is_none()).unwrap();
    assert_eq!(f.conversation(&conv).await["title"], root.content.as_str());
}

#[tokio::test]
async fn metrics_measure_the_stream() {
    let mut script = deepseek_reply("想", "喵~");
    // 200 ms between the reasoning delta and the text delta.
    script.chunks[1].0 = Duration::from_millis(200);
    let f = fixture(script).await;
    let conv = f.create_conv().await;
    let events = f.send(&conv, "你好").await;
    let m = &events[3].1["node"]["metrics"];
    let total = m["total_ms"].as_u64().unwrap();
    let ttft = m["ttft_ms"].as_u64().unwrap();
    assert!(total >= 200, "{m}");
    assert!(ttft <= total, "{m}");
    let thinking = m["reasoning_ms"].as_u64().unwrap();
    assert!((200..=total).contains(&thinking), "{m}");
    let rate = m["tokens_per_sec"].as_f64().unwrap();
    assert!(rate.is_finite() && rate > 0.0, "{m}");
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
            json!({"action": "send", "content": content, "character_id": f.character_id}),
            json!({"action": "edit", "node_id": user, "content": content, "character_id": f.character_id}),
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
async fn client_model_and_effort_overrides_are_400_and_no_upstream_call() {
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
                json!({"action": "send", "content": "hi", "character_id":f.character_id, "model_id": model_id, "effort": effort}),
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
async fn unknown_role_is_400() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let r = f
        .post(
            &conv,
            json!({"action": "send", "content": "hi", "character_id": "no-such-role"}),
        )
        .await;
    assert_eq!(r.status(), 400);
    let r = f.post("no-such-conversation", f.send_body("hi")).await;
    assert_eq!(r.status(), 404);
    assert_eq!(f.requests().len(), 0);
    assert!(f.idle());
}

#[tokio::test]
async fn malformed_turn_bodies_are_400_json() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    let unknown_action = json!({"action": "dance", "character_id": f.character_id});
    let r = f.post(&conv, unknown_action).await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"].as_str().unwrap().starts_with("请求格式错误"),
        "{v}"
    );
    let r = f
        .app
        .client
        .post(f.app.url(&format!("/api/conversations/{conv}/turns")))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].is_string());
    assert_eq!(f.requests().len(), 0);
    assert!(f.idle());
}

#[tokio::test]
async fn clear_resets_title_and_next_send_retitles() {
    let f = fixture(deepseek_reply("想", "喵~")).await;
    let conv = f.create_conv().await;
    f.send(&conv, "第一个标题").await;
    assert_eq!(f.conversation(&conv).await["title"], "第一个标题");
    let r = f
        .app
        .client
        .post(f.app.url(&format!("/api/conversations/{conv}/clear")))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(f.conversation(&conv).await["title"], DEFAULT_TITLE);
    f.send(&conv, "第二个标题").await;
    assert_eq!(f.conversation(&conv).await["title"], "第二个标题");
}

#[tokio::test]
async fn switching_leaf_during_a_turn_is_conflict() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut sse = SseReader::new(f.post(&conv, f.send_body("一")).await);
    let (name, start) = sse.next().await.unwrap();
    assert_eq!(name, "start");
    let user_id = start["user_node"]["id"].as_str().unwrap().to_owned();
    let r = f
        .app
        .client
        .put(f.app.url(&format!("/api/conversations/{conv}/leaf")))
        .json(&json!({"node_id": user_id, "descend": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].is_string());
}

#[tokio::test]
async fn stalled_upstream_times_out_with_error_event() {
    let mock = spawn_mock_upstream(slow_script()).await;
    let app = spawn_app(TestCfg {
        upstream: Some(mock.base_url.clone()),
        read_timeout: Some(Duration::from_millis(300)),
        ..Default::default()
    })
    .await;
    let model_id = app
        .store
        .insert_model(&spec("up-model", ReasoningStyle::DeepSeek))
        .unwrap()
        .id;
    let character_id = app
        .store
        .create_character(
            &serde_json::from_value(json!({
                "name":"Test role", "display_name":"Test role", "model_id":model_id
            }))
            .unwrap(),
        )
        .unwrap()
        .id;
    let f = Fixture {
        app,
        mock,
        model_id,
        character_id,
    };
    let conv = f.create_conv().await;
    let events = tokio::time::timeout(Duration::from_secs(5), f.send(&conv, "在吗"))
        .await
        .expect("a stalled upstream must time out");
    assert_eq!(names(&events), ["start", "text", "error"]);
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
            json!({"action": "regenerate", "node_id": node_id, "character_id": f.character_id}),
            json!({"action": "edit", "node_id": node_id, "content": "改", "character_id": f.character_id}),
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

async fn upload_png(f: &Fixture, conv: &str) -> Value {
    let response = f
        .app
        .client
        .post(f.app.url(&format!("/api/conversations/{conv}/attachments")))
        .header("content-type", "image/png")
        .body(b"\x89PNG\r\n\x1a\nimage fixture".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    response.json().await.unwrap()
}

#[tokio::test]
async fn image_attachment_reaches_upstream_and_survives_edit_and_regenerate() {
    let f = fixture(deepseek_reply("", "seen")).await;
    let mut model = f.app.store.get_model(&f.model_id).unwrap().unwrap().spec;
    model.vision = true;
    f.app.store.update_model(&f.model_id, &model).unwrap();
    let picker: Value = f
        .app
        .client
        .get(f.app.url("/api/characters"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let public_role = picker["characters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["id"] == f.character_id)
        .unwrap();
    assert_eq!(public_role["vision"], true);
    assert!(public_role.get("model_id").is_none());
    assert!(public_role.get("provider").is_none());
    let conv = f.create_conv().await;
    let attachment = upload_png(&f, &conv).await;
    let events = f.turn(&conv, json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":[attachment["id"]]})).await;
    let user = &events.iter().find(|(name, _)| name == "start").unwrap().1["user_node"];
    assert_eq!(user["attachments"][0]["id"], attachment["id"]);
    let first = f.requests();
    assert_eq!(
        first[0].body["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"][0]["type"],
        "image_url"
    );
    assert!(
        first[0].body["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"][0]["image_url"]["url"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
    let edited = f.turn(&conv, json!({"action":"edit","node_id":user["id"],"content":"describe","character_id":f.character_id})).await;
    let edited_user = &edited.iter().find(|(name, _)| name == "start").unwrap().1["user_node"];
    assert_eq!(edited_user["attachments"][0]["id"], attachment["id"]);
    f.turn(
        &conv,
        json!({"action":"regenerate","node_id":edited_user["id"],"character_id":f.character_id}),
    )
    .await;
    let requests = f.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[1].body["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"],
        requests[2].body["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["content"]
    );
}

#[tokio::test]
async fn image_capability_and_conversation_ownership_are_checked_before_saving() {
    let f = fixture(deepseek_reply("", "never")).await;
    let conv = f.create_conv().await;
    let other = f.create_conv().await;
    let attachment = upload_png(&f, &conv).await;
    let body = json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":[attachment["id"]]});
    assert_eq!(f.post(&conv, body.clone()).await.status(), 400);
    assert!(f.app.store.nodes(&conv).unwrap().is_empty());
    assert!(f.requests().is_empty());
    let mut model = f.app.store.get_model(&f.model_id).unwrap().unwrap().spec;
    model.vision = true;
    f.app.store.update_model(&f.model_id, &model).unwrap();
    assert_eq!(f.post(&other, body).await.status(), 400);
    assert!(f.app.store.nodes(&other).unwrap().is_empty());
    assert!(f.requests().is_empty());
}

#[tokio::test]
async fn clear_and_delete_remove_even_unused_uploads_and_serve_private_image_headers() {
    let f = fixture(deepseek_reply("", "")).await;
    let conv = f.create_conv().await;
    let attachment = upload_png(&f, &conv).await;
    let url = f.app.url(attachment["url"].as_str().unwrap());
    let response = f.app.client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(response.headers()["content-type"], "image/png");
    let other = f.create_conv().await;
    assert_eq!(
        f.app
            .client
            .get(f.app.url(&format!(
                "/api/conversations/{other}/attachments/{}",
                attachment["id"].as_str().unwrap()
            )))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        f.app
            .client
            .post(f.app.url(&format!("/api/conversations/{conv}/clear")))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(f.app.client.get(&url).send().await.unwrap().status(), 404);
    let second = upload_png(&f, &conv).await;
    assert_eq!(
        f.app
            .client
            .delete(f.app.url(&format!("/api/conversations/{conv}")))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        f.app
            .client
            .get(f.app.url(second["url"].as_str().unwrap()))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn sixth_image_and_remote_image_url_are_rejected_without_saving_or_upstream() {
    let f = fixture(deepseek_reply("", "never")).await;
    let mut model = f.app.store.get_model(&f.model_id).unwrap().unwrap().spec;
    model.vision = true;
    f.app.store.update_model(&f.model_id, &model).unwrap();
    let conv = f.create_conv().await;
    let mut ids = Vec::new();
    for _ in 0..6 {
        ids.push(upload_png(&f, &conv).await["id"].clone());
    }
    let request =
        json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":ids});
    assert_eq!(f.post(&conv, request).await.status(), 400);
    assert_eq!(f.post(&conv,json!({"action":"send","content":"url","character_id":f.character_id,"images":["https://example.com/private.png"]})).await.status(),400);
    assert!(f.requests().is_empty());
    assert!(f.app.store.nodes(&conv).unwrap().is_empty());
}

#[tokio::test]
async fn role_switch_to_text_only_refuses_image_history_before_saving_or_upstream() {
    let f = fixture(deepseek_reply("", "seen")).await;
    let mut model = f.app.store.get_model(&f.model_id).unwrap().unwrap().spec;
    model.vision = true;
    f.app.store.update_model(&f.model_id, &model).unwrap();
    let conv = f.create_conv().await;
    let image = upload_png(&f, &conv).await;
    f.turn(&conv,json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":[image["id"]]})).await;
    model.vision = false;
    f.app.store.update_model(&f.model_id, &model).unwrap();
    assert_eq!(f.post(&conv, f.send_body("continue")).await.status(), 400);
    assert_eq!(f.requests().len(), 1);
    assert_eq!(f.app.store.nodes(&conv).unwrap().len(), 2);
}

async fn upload_document(f: &Fixture, conv: &str, name: &str, bytes: Vec<u8>) -> Value {
    let response = f
        .app
        .client
        .post(f.app.url(&format!("/api/conversations/{conv}/attachments")))
        .header(
            "x-file-name",
            percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC)
                .to_string(),
        )
        .body(bytes)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    response.json().await.unwrap()
}

// A real single-page PDF: byte offsets and stream length are generated, not an extraction stub.
fn document_pdf(text: &str) -> Vec<u8> {
    let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".to_owned(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(b"xref\n0 6\n0000000000 65535 f \n");
    for offset in &offsets[1..] {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!("trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    pdf
}

#[tokio::test]
async fn real_pdf_and_utf8_markdown_reach_text_model_and_survive_history_edit_regenerate() {
    let f = fixture(deepseek_reply("", "accepted")).await;
    let conv = f.create_conv().await;
    let pdf = upload_document(
        &f,
        &conv,
        "report.pdf",
        document_pdf("Invoice total 731 dollars"),
    )
    .await;
    let md = upload_document(
        &f,
        &conv,
        "笔记.md",
        "# 独立内容\n订单编号：MK-832".as_bytes().to_vec(),
    )
    .await;
    assert_eq!(pdf["kind"], "document");
    assert_eq!(md["name"], "笔记.md");
    assert!(md.get("extracted_text").is_none());
    assert!(md.get("bytes").is_none());
    let sent = f.turn(&conv, json!({"action":"send","content":"summarize","character_id":f.character_id,"attachment_ids":[pdf["id"],md["id"]]})).await;
    let user = &sent.iter().find(|(name, _)| name == "start").unwrap().1["user_node"];
    let saved = f.app.store.nodes(&conv).unwrap();
    assert_eq!(saved[0].attachments.len(), 2);
    assert_eq!(saved[0].content, "summarize");
    f.turn(&conv, f.send_body("continue")).await;
    let edited = f.turn(&conv, json!({"action":"edit","node_id":user["id"],"content":"revise","character_id":f.character_id})).await;
    let edited_user = &edited.iter().find(|(name, _)| name == "start").unwrap().1["user_node"];
    f.turn(
        &conv,
        json!({"action":"regenerate","node_id":edited_user["id"],"character_id":f.character_id}),
    )
    .await;
    let requests = f.requests();
    assert_eq!(requests.len(), 4);
    for request in &requests {
        let document_turn = request.body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| {
                message["role"] == "user"
                    && message["content"]
                        .as_str()
                        .is_some_and(|text| text.contains("Invoice total 731 dollars"))
            })
            .expect("extracted PDF content must be user text, never an image or system prompt");
        let content = document_turn["content"].as_str().unwrap();
        assert!(content.contains("report.pdf"));
        assert!(content.contains("笔记.md"));
        assert!(content.contains("订单编号：MK-832"));
        assert!(!request.body.to_string().contains("image_url"));
    }
    assert_eq!(requests[2].body["messages"], requests[3].body["messages"]);
}

#[tokio::test]
async fn mixed_uploads_only_encode_images_and_reject_duplicate_or_wrong_conversation_ids() {
    let f = fixture(deepseek_reply("", "accepted")).await;
    let mut model = f.app.store.get_model(&f.model_id).unwrap().unwrap().spec;
    model.vision = true;
    f.app.store.update_model(&f.model_id, &model).unwrap();
    let conv = f.create_conv().await;
    let other = f.create_conv().await;
    let doc = upload_document(&f, &conv, "data.csv", b"item,total\ncoffee,23".to_vec()).await;
    let image = upload_png(&f, &conv).await;
    for (target, ids) in [
        (&conv, json!([doc["id"], doc["id"]])),
        (&other, json!([doc["id"]])),
    ] {
        assert_eq!(f.post(target, json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":ids})).await.status(), 400);
        assert!(f.app.store.nodes(target).unwrap().is_empty());
    }
    assert!(f.requests().is_empty());
    f.turn(&conv, json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":[doc["id"],image["id"]]})).await;
    let requests = f.requests();
    let parts = requests[0].body["messages"]
        .as_array()
        .unwrap()
        .last()
        .unwrap()["content"]
        .as_array()
        .unwrap();
    assert_eq!(
        parts
            .iter()
            .filter(|part| part["type"] == "image_url")
            .count(),
        1
    );
    assert!(
        parts
            .iter()
            .any(|part| part["type"] == "text"
                && part["text"].as_str().unwrap().contains("coffee,23"))
    );
}

#[tokio::test]
async fn documents_download_safely_and_unused_uploads_disappear_on_clear_and_delete() {
    let f = fixture(deepseek_reply("", "")).await;
    let conv = f.create_conv().await;
    let bytes = b"<html><script>alert(1)</script></html>".to_vec();
    let doc = upload_document(&f, &conv, "../网页.html", bytes.clone()).await;
    assert_eq!(doc["name"], "网页.html");
    let url = f.app.url(doc["url"].as_str().unwrap());
    let response = f.app.client.get(&url).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/plain; charset=utf-8"
    );
    assert_eq!(response.headers()["cache-control"], "private, no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert!(disposition.starts_with("attachment;"));
    assert!(disposition.contains("filename*=UTF-8''"));
    assert_eq!(response.bytes().await.unwrap().as_ref(), bytes.as_slice());
    assert_eq!(
        f.app
            .client
            .post(f.app.url(&format!("/api/conversations/{conv}/clear")))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(f.app.client.get(&url).send().await.unwrap().status(), 404);
    let doc = upload_document(&f, &conv, "unused.txt", b"delete private document".to_vec()).await;
    assert_eq!(
        f.app
            .client
            .delete(f.app.url(&format!("/api/conversations/{conv}")))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        f.app
            .client
            .get(f.app.url(doc["url"].as_str().unwrap()))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}

#[tokio::test]
async fn unsupported_binary_corrupt_pdf_and_oversize_documents_are_not_persisted() {
    let f = fixture(deepseek_reply("", "")).await;
    let conv = f.create_conv().await;
    for (name, bytes) in [
        ("archive.zip", b"PK\x03\x04binary".to_vec()),
        ("binary.txt", b"hello\0world".to_vec()),
        ("broken.pdf", b"%PDF-1.4\nnot a valid document".to_vec()),
        ("empty.pdf", document_pdf("")),
        ("too-much-text.md", vec![b'a'; 2 * 1024 * 1024 + 1]),
        ("too-large.txt", vec![b'a'; 16 * 1024 * 1024 + 1]),
    ] {
        let response = f
            .app
            .client
            .post(f.app.url(&format!("/api/conversations/{conv}/attachments")))
            .header("x-file-name", name)
            .body(bytes)
            .send()
            .await
            .unwrap();
        assert!(
            matches!(response.status().as_u16(), 400 | 413),
            "{name}: {}",
            response.status()
        );
    }
    assert!(f.app.store.nodes(&conv).unwrap().is_empty());
}

#[tokio::test]
async fn transport_neutral_turn_checks_actor_ownership_before_reading_or_saving() {
    let f = fixture(deepseek_reply("", "never")).await;
    let conv = f.create_conv().await;
    let request = serde_json::from_value(f.send_body("must not be stored")).unwrap();
    let result = nekochat_server::chat::start_turn(
        f.app.state.clone(),
        "different-user-id".into(),
        conv.clone(),
        request,
    )
    .await;
    let error = match result {
        Ok(_) => panic!("an unauthorised Rust channel must not start a turn"),
        Err(error) => error,
    };
    use axum::response::IntoResponse;
    assert_eq!(error.into_response().status(), 404);
    assert!(f.app.store.nodes(&conv).unwrap().is_empty());
    assert!(f.requests().is_empty());
    assert!(f.idle());
}

#[tokio::test]
async fn sixth_document_is_rejected_before_saving_or_contacting_a_text_only_model() {
    let f = fixture(deepseek_reply("", "never")).await;
    let conv = f.create_conv().await;
    let mut ids = Vec::new();
    for _ in 0..6 {
        ids.push(
            upload_document(&f, &conv, "note.md", b"uploaded document".to_vec()).await["id"]
                .clone(),
        );
    }
    assert_eq!(
        f.post(
            &conv,
            json!({"action":"send","content":"","character_id":f.character_id,"attachment_ids":ids})
        )
        .await
        .status(),
        400
    );
    assert!(f.app.store.nodes(&conv).unwrap().is_empty());
    assert!(f.requests().is_empty());
}

#[tokio::test]
async fn encrypted_pdf_is_rejected_even_when_empty_password_would_auto_decrypt() {
    use pdf_extract::encryption::{EncryptionState, EncryptionVersion, Permissions};
    use pdf_extract::{Document, Object};
    let f = fixture(deepseek_reply("", "")).await;
    let conv = f.create_conv().await;
    for user_password in ["fixture-user", ""] {
        let mut pdf =
            Document::load_mem(&document_pdf("Encrypted content must not be accepted")).unwrap();
        pdf.trailer.set(
            "ID",
            Object::Array(vec![
                Object::string_literal("document-regression-identifier"),
                Object::string_literal("document-regression-identifier"),
            ]),
        );
        let state = EncryptionState::try_from(EncryptionVersion::V2 {
            document: &pdf,
            owner_password: "fixture-owner",
            user_password,
            key_length: 128,
            permissions: Permissions::default(),
        })
        .unwrap();
        pdf.encrypt(&state).unwrap();
        let mut bytes = Vec::new();
        pdf.save_to(&mut bytes).unwrap();
        let response = f
            .app
            .client
            .post(f.app.url(&format!("/api/conversations/{conv}/attachments")))
            .header("x-file-name", "encrypted.pdf")
            .body(bytes)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 400);
        let error: Value = response.json().await.unwrap();
        assert!(error["error"].as_str().unwrap().contains("加密"));
    }
}

#[tokio::test]
async fn reconnect_observes_a_snapshot_then_remaining_deltas_without_another_call() {
    let f = fixture(MockScript {
        status: 200,
        chunks: vec![
            (Duration::ZERO, text_chunk("before-refresh")),
            (Duration::from_secs(2), text_chunk("-after-refresh")),
            (Duration::ZERO, done_chunk()),
        ],
        models: Vec::new(),
    })
    .await;
    let conv = f.create_conv().await;
    let mut original = SseReader::new(f.post(&conv, f.send_body("refresh")).await);
    let start = original.next().await.unwrap();
    let id = start.1["assistant_node_id"].as_str().unwrap().to_owned();
    assert_eq!(original.next().await.unwrap().1["t"], "before-refresh");
    drop(original);
    let mut resumed = SseReader::new(
        f.app
            .client
            .get(f.app.url(&format!("/api/conversations/{conv}/turns")))
            .send()
            .await
            .unwrap(),
    );
    let (kind, snapshot) = resumed.next().await.unwrap();
    assert_eq!(kind, "snapshot");
    assert_eq!(snapshot["node"]["id"], id);
    assert_eq!(snapshot["node"]["content"], "before-refresh");
    assert!(snapshot["node"].get("model_id").is_none());
    let remaining = tokio::time::timeout(Duration::from_secs(5), resumed.collect())
        .await
        .unwrap();
    assert_eq!(remaining.last().unwrap().0, "done");
    assert_eq!(
        remaining.last().unwrap().1["node"]["content"],
        "before-refresh-after-refresh"
    );
    assert_eq!(f.requests().len(), 1);
    assert_eq!(
        f.app
            .store
            .nodes(&conv)
            .unwrap()
            .iter()
            .filter(|n| n.role == NodeRole::Assistant)
            .count(),
        1
    );
}

#[tokio::test]
async fn explicit_stop_persists_partial_and_stale_stop_cannot_stop_the_active_turn() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut events = SseReader::new(f.post(&conv, f.send_body("stop")).await);
    let start = events.next().await.unwrap();
    let id = start.1["assistant_node_id"].as_str().unwrap();
    assert_eq!(events.next().await.unwrap().1["t"], "x");
    let stale = f
        .app
        .client
        .post(
            f.app
                .url(&format!("/api/conversations/{conv}/turns/stale/stop")),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    assert!(!f.idle());
    let stopped = f
        .app
        .client
        .post(
            f.app
                .url(&format!("/api/conversations/{conv}/turns/{id}/stop")),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(stopped.status(), 204);
    let final_events = tokio::time::timeout(Duration::from_secs(3), events.collect())
        .await
        .unwrap();
    let final_node = &final_events.last().unwrap().1["node"];
    assert_eq!(final_node["status"], "interrupted");
    assert_eq!(final_node["content"], "x");
    assert!(f.idle());
}

#[tokio::test]
async fn another_user_cannot_observe_or_stop_a_generation() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut events = SseReader::new(f.post(&conv, f.send_body("private")).await);
    let start = events.next().await.unwrap();
    let id = start.1["assistant_node_id"].as_str().unwrap();
    let other = reqwest::Client::builder()
        .cookie_store(true)
        .build()
        .unwrap();
    let registration = other
        .post(f.app.url("/api/account/register"))
        .json(&json!({"username":"other-observer","password":"isolated-test-password"}))
        .send()
        .await
        .unwrap();
    assert!(registration.status().is_success());
    assert_eq!(
        other
            .get(f.app.url(&format!("/api/conversations/{conv}/turns")))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        other
            .post(
                f.app
                    .url(&format!("/api/conversations/{conv}/turns/{id}/stop"))
            )
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert!(!f.idle());
    assert_eq!(
        f.app
            .client
            .post(
                f.app
                    .url(&format!("/api/conversations/{conv}/turns/{id}/stop"))
            )
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
}

#[tokio::test]
async fn revoked_observer_session_closes_but_core_can_be_observed_after_login() {
    let f = fixture(slow_script()).await;
    let conv = f.create_conv().await;
    let mut original = SseReader::new(f.post(&conv, f.send_body("private stream")).await);
    let start = original.next().await.unwrap();
    let id = start.1["assistant_node_id"].as_str().unwrap();
    assert_eq!(original.next().await.unwrap().1["t"], "x");
    assert_eq!(
        f.app
            .client
            .delete(f.app.url("/api/account/session"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(1), original.next())
            .await
            .expect("revoked observer kept access")
            .is_none()
    );
    assert!(!f.idle());
    assert_eq!(
        f.app
            .client
            .post(f.app.url("/api/account/login"))
            .json(&json!({"username":"fixture-user","password":"fixture-password-only"}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let mut resumed = SseReader::new(
        f.app
            .client
            .get(f.app.url(&format!("/api/conversations/{conv}/turns")))
            .send()
            .await
            .unwrap(),
    );
    assert_eq!(resumed.next().await.unwrap().1["node"]["content"], "x");
    assert_eq!(f.requests().len(), 1);
    assert_eq!(
        f.app
            .client
            .post(
                f.app
                    .url(&format!("/api/conversations/{conv}/turns/{id}/stop"))
            )
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
}
