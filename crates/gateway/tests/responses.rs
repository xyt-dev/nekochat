use axum::{Router, routing::post};
use futures_util::StreamExt;
use nekochat_core::{GenerationParameters, ReasoningStyle};
use nekochat_gateway::{ChatRequest, GatewayError, Provider, StreamEvent, stream_responses};
use serde_json::{Value, json};

async fn collect(events: &[Value]) -> Vec<Result<StreamEvent, GatewayError>> {
    collect_with_tools(events, false).await
}

async fn collect_with_tools(events: &[Value], tools: bool) -> Vec<Result<StreamEvent, GatewayError>> {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!(
            "event: {}\ndata: {}\n\n",
            event["type"].as_str().unwrap(),
            event
        ));
    }
    let app = Router::new().route(
        "/responses",
        post(move || {
            let body = body.clone();
            async move { ([("content-type", "text/event-stream")], body) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = Provider {
        key: "fixture".into(),
        label: "fixture".into(),
        base_url: format!("http://{address}"),
        models_url: None,
        api_key: "fixture-key".into(),
        session_header: Some("x-opencode-session".into()),
    };
    let parameters = GenerationParameters::default();
    let tool_context = nekochat_gateway::ToolContext::default();
    let request = ChatRequest {
        model: "gpt-fixture",
        messages: &[],
        reasoning: ReasoningStyle::Effort,
        effort: Some("high"),
        session_id: "stable-session",
        parameters: &parameters,
        tools: tools.then_some(&tool_context),
    };
    let result = stream_responses(&reqwest::Client::new(), &provider, &request)
        .await
        .unwrap()
        .collect()
        .await;
    task.abort();
    result
}

#[tokio::test]
async fn completed_response_preserves_text_reasoning_and_nonoverlapping_usage() {
    let events = collect(&[
        json!({"type":"response.created","response":{"status":"in_progress"}}),
        json!({"type":"response.reasoning_summary_text.delta","delta":"think"}),
        json!({"type":"response.output_text.delta","delta":"hello"}),
        json!({"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":100,"input_tokens_details":{"cached_tokens":80},"output_tokens":13,"output_tokens_details":{"reasoning_tokens":3}}}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events[0], StreamEvent::Reasoning("think".into()));
    assert_eq!(events[1], StreamEvent::Text("hello".into()));
    let StreamEvent::Usage(usage) = &events[2] else {
        panic!("missing final usage")
    };
    assert_eq!(
        (
            usage.cached_in,
            usage.uncached_in,
            usage.reasoning,
            usage.output
        ),
        (80, 20, 3, 10)
    );
}

#[tokio::test]
async fn go_billing_counts_take_precedence_over_api_native_counts() {
    let events = collect(&[json!({"type":"response.completed","response":{"status":"completed","usage":{
        "input_tokens":999,"output_tokens":999,"cost":0,
        "opencode":{"allowance":0.25,"tokens":{"input":20,"cache_read":80,"output":13,"reasoning":3}}
    }}})]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    let StreamEvent::Usage(usage) = &events[0] else {
        panic!("missing billing usage")
    };
    assert_eq!(
        (
            usage.cached_in,
            usage.uncached_in,
            usage.reasoning,
            usage.output
        ),
        (80, 20, 3, 10)
    );
    assert_eq!(usage.upstream_cost_usd, Some(0.0));
    assert_eq!(usage.allowance_usd, Some(0.25));
}

#[tokio::test]
async fn unexpected_eof_retains_partial_text_but_is_not_success() {
    let events = collect(&[json!({"type":"response.output_text.delta","delta":"partial"})]).await;
    assert_eq!(
        events[0].as_ref().unwrap(),
        &StreamEvent::Text("partial".into())
    );
    assert!(matches!(&events[1], Err(GatewayError::Transport(_))));
}

#[tokio::test]
async fn failure_and_output_limit_are_errors_even_with_a_usage_record() {
    for status in ["failed", "incomplete"] {
        let event = json!({"type":format!("response.{status}"),"response":{"status":status,
            "error":{"message":"upstream refused"},"incomplete_details":{"reason":"max_output_tokens"},
            "usage":{"input_tokens":5,"output_tokens":3}}});
        let events = collect(&[event]).await;
        assert!(
            matches!(&events[0], Ok(StreamEvent::Usage(usage)) if usage.uncached_in == 5 && usage.output == 3)
        );
        assert!(matches!(
            events.last(),
            Some(Err(GatewayError::Protocol(_)))
        ));
    }
}

#[tokio::test]
async fn model_refusal_is_visible_text_instead_of_an_empty_reply() {
    let events = collect(&[
        json!({"type":"response.refusal.delta","delta":"I cannot help with that."}),
        json!({"type":"response.completed","response":{"status":"completed"}}),
    ])
    .await
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .unwrap();
    assert_eq!(
        events,
        vec![StreamEvent::Text("I cannot help with that.".into())]
    );
}

#[tokio::test]
async fn final_only_text_and_refusal_are_visible_at_every_snapshot_level() {
    for (part_type, field, done_type, text) in [
        ("output_text", "text", "response.output_text.done", "你好🐈"),
        ("refusal", "refusal", "response.refusal.done", "无法回答。"),
    ] {
        let part = json!({"type":part_type,(field):text});
        let item = json!({"id":"msg_1","type":"message","role":"assistant","content":[part]});
        for snapshot in [
            json!({"type":done_type,"item_id":"msg_1","output_index":0,"content_index":0,(field):text}),
            json!({"type":"response.content_part.done","item_id":"msg_1","output_index":0,"content_index":0,"part":part}),
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"status":"completed","output":[item]}}),
        ] {
            let events = collect(&[
                snapshot,
                json!({"type":"response.completed","response":{"status":"completed"}}),
            ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
            assert_eq!(events, vec![StreamEvent::Text(text.into())]);
        }
    }
}

#[tokio::test]
async fn partial_deltas_and_repeated_unicode_snapshots_emit_each_suffix_once() {
    let item = json!({"id":"msg_1","type":"message","role":"assistant","content":[
        {"type":"output_text","text":"你🐈好"},
        {"type":"output_text","text":"第二段"},
        {"type":"refusal","refusal":"无法回答。"}
    ]});
    let events = collect(&[
        json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"content_index":0,"delta":"你"}),
        json!({"type":"response.output_text.done","item_id":"msg_1","output_index":1,"content_index":0,"text":"你🐈"}),
        json!({"type":"response.content_part.done","item_id":"msg_1","output_index":1,"content_index":0,"part":{"type":"output_text","text":"你🐈好"}}),
        json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"content_index":1,"delta":"第"}),
        json!({"type":"response.refusal.delta","item_id":"msg_1","output_index":1,"content_index":2,"delta":"无法"}),
        json!({"type":"response.output_item.done","output_index":1,"item":item}),
        json!({"type":"response.completed","response":{"status":"completed","output":[
            {"id":"reason_1","type":"reasoning","summary":[]},item
        ]}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![
        StreamEvent::Text("你".into()), StreamEvent::Text("🐈".into()),
        StreamEvent::Text("好".into()), StreamEvent::Text("第".into()),
        StreamEvent::Text("无法".into()), StreamEvent::Text("二段".into()),
        StreamEvent::Text("回答。".into()),
    ]);
}

#[tokio::test]
async fn completed_snapshot_keeps_native_tool_items_once_and_allows_tool_only_rounds() {
    let call = json!({"id":"fc_1","type":"function_call","call_id":"call_1","name":"lookup","arguments":"{}","status":"completed"});
    let reasoning = json!({"id":"rs_1","type":"reasoning","summary":[],"encrypted_content":"opaque"});
    for emit_done in [false, true] {
        let mut input = Vec::new();
        if emit_done {
            input.push(json!({"type":"response.output_item.done","output_index":0,"item":reasoning}));
            input.push(json!({"type":"response.output_item.done","output_index":1,"item":call}));
        }
        input.push(json!({"type":"response.completed","response":{"status":"completed","output":[reasoning,call]}}));
        let events = collect_with_tools(&input, true).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(events, vec![StreamEvent::ResponseItem(reasoning.clone()), StreamEvent::ResponseItem(call.clone())]);
        assert!(collect(&input).await.is_empty());
    }
}

#[tokio::test]
async fn final_reasoning_snapshots_preserve_only_unstreamed_summary_and_content() {
    let events = collect(&[
        json!({"type":"response.reasoning_summary_text.delta","item_id":"rs_1","output_index":0,"summary_index":0,"delta":"想"}),
        json!({"type":"response.reasoning_summary_text.done","item_id":"rs_1","output_index":0,"summary_index":0,"text":"想一想"}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[{"type":"summary_text","text":"想一想"}],"content":[{"type":"reasoning_text","text":"过程"}]}}),
        json!({"type":"response.completed","response":{"status":"completed"}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::Reasoning("想".into()), StreamEvent::Reasoning("一想".into()), StreamEvent::Reasoning("过程".into())]);
}

#[tokio::test]
async fn completed_envelope_with_failed_status_preserves_partial_snapshot_and_usage() {
    for status in ["failed", "incomplete"] {
        let events = collect(&[json!({"type":"response.completed","response":{
            "status":status,"incomplete_details":{"reason":"max_output_tokens"},
            "output":[{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"partial"}]}],
            "usage":{"input_tokens":9,"output_tokens":2}
        }})]).await;
        assert_eq!(events[0].as_ref().unwrap(), &StreamEvent::Text("partial".into()));
        assert!(matches!(&events[1], Ok(StreamEvent::Usage(usage)) if usage.uncached_in == 9 && usage.output == 2));
        assert!(matches!(events.last(), Some(Err(GatewayError::Protocol(_)))));
    }
}

#[tokio::test]
async fn separate_output_items_keep_independent_text_offsets() {
    let events = collect(&[
        json!({"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"第一"}),
        json!({"type":"response.output_text.delta","item_id":"msg_2","output_index":1,"content_index":0,"delta":"第"}),
        json!({"type":"response.completed","response":{"status":"completed","output":[
            {"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"第一条"}]},
            {"id":"msg_2","type":"message","role":"assistant","content":[{"type":"output_text","text":"第二条"}]}
        ]}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::Text("第一".into()), StreamEvent::Text("第".into()), StreamEvent::Text("条".into()), StreamEvent::Text("二条".into())]);
}

#[tokio::test]
async fn item_ids_reconcile_snapshots_when_output_index_is_absent() {
    let events = collect(&[
        json!({"type":"response.output_text.delta","item_id":"msg_1","content_index":0,"delta":"one"}),
        json!({"type":"response.output_text.delta","item_id":"msg_2","content_index":0,"delta":"two"}),
        json!({"type":"response.output_item.done","item":{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"one!"}]}}),
        json!({"type":"response.completed","response":{"status":"completed","output":[
            {"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":"one!"}]},
            {"id":"msg_2","type":"message","role":"assistant","content":[{"type":"output_text","text":"two!"}]}
        ]}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::Text("one".into()), StreamEvent::Text("two".into()), StreamEvent::Text("!".into()), StreamEvent::Text("!".into())]);
}

#[tokio::test]
async fn empty_completed_snapshots_remain_empty_without_gateway_errors() {
    let events = collect(&[
        json!({"type":"response.output_text.done","item_id":"msg_1","output_index":0,"content_index":0,"text":""}),
        json!({"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":""}]}}),
        json!({"type":"response.completed","response":{"status":"completed","output":[{"id":"msg_1","type":"message","role":"assistant","content":[{"type":"output_text","text":""}]}],"usage":{"input_tokens":31572,"output_tokens":189}}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(matches!(events.as_slice(), [StreamEvent::Usage(usage)] if usage.uncached_in == 31572 && usage.output == 189));
}

#[tokio::test]
async fn distinct_item_ids_override_reused_stream_indices_without_repeating_final_snapshots() {
    let reasoning = json!({"id":"reasoning-1","type":"reasoning","summary":[{"type":"summary_text","text":"检查公开资料"}],"encrypted_content":"opaque"});
    let message = json!({"id":"message-1","type":"message","role":"assistant","content":[{"type":"output_text","text":"WAL 同时只允许一个写入者。🐈"}]});
    for with_tools in [false, true] {
        let events = collect_with_tools(&[
            json!({"type":"response.reasoning_summary_text.delta","item_id":"reasoning-1","output_index":0,"summary_index":0,"delta":"检查公开资料"}),
            json!({"type":"response.output_text.delta","item_id":"message-1","output_index":0,"content_index":0,"delta":"WAL 同时只允许一个写入者。🐈"}),
            json!({"type":"response.output_text.done","item_id":"message-1","output_index":0,"content_index":0,"text":"WAL 同时只允许一个写入者。🐈"}),
            json!({"type":"response.output_item.done","output_index":0,"item":message}),
            json!({"type":"response.completed","response":{"status":"completed","output":[reasoning,message]}}),
        ], with_tools).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
        let text = events.iter().filter_map(|event| match event { StreamEvent::Text(text) => Some(text.as_str()), _ => None }).collect::<String>();
        let thought = events.iter().filter_map(|event| match event { StreamEvent::Reasoning(text) => Some(text.as_str()), _ => None }).collect::<String>();
        assert_eq!(text, "WAL 同时只允许一个写入者。🐈");
        assert_eq!(thought, "检查公开资料");
        if with_tools {
            let items = events.into_iter().filter_map(|event| match event { StreamEvent::ResponseItem(item) => Some(item), _ => None }).collect::<Vec<_>>();
            assert_eq!(items, vec![message.clone(), reasoning.clone()]);
        }
    }
}

#[tokio::test]
async fn equal_text_in_distinct_messages_is_preserved_despite_reused_indices() {
    let first = json!({"id":"message-1","type":"message","role":"assistant","content":[{"type":"output_text","text":"重复是正文内容。"}]});
    let second = json!({"id":"message-2","type":"message","role":"assistant","content":[{"type":"output_text","text":"重复是正文内容。"}]});
    let events = collect(&[
        json!({"type":"response.output_text.delta","item_id":"message-1","output_index":0,"content_index":0,"delta":"重复是正文内容。"}),
        json!({"type":"response.output_text.delta","item_id":"message-2","output_index":0,"content_index":0,"delta":"重复是正文内容。"}),
        json!({"type":"response.completed","response":{"status":"completed","output":[first,second]}}),
    ]).await.into_iter().collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(events, vec![StreamEvent::Text("重复是正文内容。".into()), StreamEvent::Text("重复是正文内容。".into())]);
}
