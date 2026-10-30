use axum::{Router, routing::post};
use futures_util::StreamExt;
use nekochat_core::{GenerationParameters, ReasoningStyle};
use nekochat_gateway::{ChatRequest, GatewayError, Provider, StreamEvent, stream_responses};
use serde_json::{Value, json};

async fn collect(events: &[Value]) -> Vec<Result<StreamEvent, GatewayError>> {
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
    let request = ChatRequest {
        model: "gpt-fixture",
        messages: &[],
        reasoning: ReasoningStyle::Effort,
        effort: Some("high"),
        session_id: "stable-session",
        parameters: &parameters,
        tools: None,
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
