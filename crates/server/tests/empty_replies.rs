mod common;

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use axum::{Router, routing::post};
use common::{PROVIDER_KEY, TestCfg, spawn_app};
use nekochat_store::{Node, NodeRole, NodeStatus, new_id, now_ms};
use serde_json::{Value, json};

#[tokio::test]
async fn empty_finished_turns_are_errors_with_durable_usage_and_no_automatic_retry() {
    for native in [false, true] {
        for (text, reasoning, reported_usage) in [("", "", true), (" \n\t", "", true), ("", "thinking without an answer", true), ("", "", false)] {
            let mut events = Vec::new();
            if !reasoning.is_empty() {
                events.push(if native { json!({"type":"response.reasoning_summary_text.delta","delta":reasoning}) }
                    else { json!({"choices":[{"delta":{"reasoning_content":reasoning}}]}) });
            }
            if !text.is_empty() {
                events.push(if native { json!({"type":"response.output_text.delta","delta":text}) }
                    else { json!({"choices":[{"delta":{"content":text}}]}) });
            }
            events.push(if native {
                json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}],"usage":{"input_tokens":31572,"output_tokens":189}}})
            } else {
                json!({"choices":[],"usage":{"prompt_tokens":31572,"completion_tokens":189}})
            });
            if !reported_usage {
                if native { events.last_mut().unwrap()["response"].as_object_mut().unwrap().remove("usage"); }
                else { events.last_mut().unwrap().as_object_mut().unwrap().remove("usage"); }
            }
            let mut sse = events.into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
            if !native { sse.push_str("data: [DONE]\n\n"); }
            let calls = Arc::new(AtomicUsize::new(0));
            let seen = calls.clone();
            let handler = move || { let sse = sse.clone(); let seen = seen.clone(); async move {
                seen.fetch_add(1, Ordering::SeqCst);
                ([("content-type", "text/event-stream")], sse)
            }};
            let router = Router::new().route(if native { "/responses" } else { "/chat/completions" }, post(handler));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let upstream = format!("http://{}", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
            let app = spawn_app(TestCfg { upstream: Some(upstream), ..Default::default() }).await;
            let model = app.store.insert_model(&serde_json::from_value(json!({
                "provider":PROVIDER_KEY,"upstream_id":"model","display_name":"Model","sort":0,
                "protocol":if native {"responses"} else {"chat_completions"},
                "reasoning":{"style":"none","levels":[],"default":null},"vision":false,"context_window":null,
                "price":{"cached_in":0,"uncached_in":1,"output":2}
            })).unwrap()).unwrap();
            let role = app.store.create_character(&serde_json::from_value(json!({"name":"role","display_name":"Role","model_id":model.id})).unwrap()).unwrap();
            let conv = app.store.create_owned_conversation(&app.user_id, Some(&role.id)).unwrap();
            let response = app.client.post(app.url(&format!("/api/conversations/{}/turns", conv.id)))
                .json(&json!({"action":"send","character_id":role.id,"content":"hello"})).send().await.unwrap();
            let stream = response.text().await.unwrap();
            assert!(stream.contains("event: error"), "native={native}, {stream}");
            assert!(!stream.contains("event: done"));
            let nodes = app.store.nodes(&conv.id).unwrap();
            let reply = nodes.iter().find(|node| node.role == NodeRole::Assistant).expect("empty completion failure remains in history");
            assert_eq!(reply.status, NodeStatus::Error);
            assert_eq!(reply.content, text);
            assert_eq!(reply.reasoning.as_deref().unwrap_or(""), reasoning);
            assert_eq!(reply.metrics.as_ref().unwrap().usage.output, if reported_usage { 189 } else { 0 });
            let history: Value = app.client.get(app.url(&format!("/api/conversations/{}", conv.id))).send().await.unwrap().json().await.unwrap();
            let public = history["nodes"].as_array().unwrap().iter().find(|node| node["id"] == reply.id).unwrap();
            assert_eq!(public["status"], "error");
            assert_eq!(public["metrics"]["usage"]["output"], if reported_usage { 189 } else { 0 });
            let billing = app.store.usage_between(0, i64::MAX).unwrap();
            if reported_usage {
                assert_eq!(billing[0].usage.uncached_in, 31572);
                assert_eq!(billing[0].usage.output, 189);
            } else { assert!(billing.is_empty()); }
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            task.abort();
        }
    }
}

#[tokio::test]
async fn legacy_completed_blank_reply_is_reported_as_error_without_rewriting_history() {
    let app = spawn_app(TestCfg::default()).await;
    let conv = app.store.create_owned_conversation(&app.user_id, None).unwrap();
    let node: Node = serde_json::from_value(json!({
        "id":new_id(),"conversation_id":conv.id,"parent_id":null,"role":"assistant","content":" \n",
        "attachments":[],"reasoning":null,"model_id":null,"model_label":null,"effort":null,
        "character_id":null,"character_name":null,"character_avatar":null,"status":"complete","metrics":null,"created_at":now_ms()
    })).unwrap();
    app.store.insert_node(&node).unwrap();
    let history: Value = app.client.get(app.url(&format!("/api/conversations/{}", conv.id))).send().await.unwrap().json().await.unwrap();
    assert_eq!(history["nodes"][0]["status"], "error");
    assert_eq!(history["nodes"][0]["content"], " \n");
    assert_eq!(app.store.get_node(&conv.id, &node.id).unwrap().unwrap().status, NodeStatus::Complete);
}
