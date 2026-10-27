use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use futures_util::StreamExt;
use nekochat_core::ReasoningStyle;
use nekochat_gateway::*;
use serde_json::json;

const MSGS: &[ChatMessage] = &[];
static DEFAULT_PARAMETERS: std::sync::LazyLock<nekochat_core::GenerationParameters> =
    std::sync::LazyLock::new(Default::default);

fn req(style: ReasoningStyle, effort: Option<&'static str>) -> ChatRequest<'static> {
    ChatRequest {
        model: "m",
        messages: MSGS,
        reasoning: style,
        effort,
        session_id: "sess-1",
        parameters: &DEFAULT_PARAMETERS,
    }
}

#[test]
fn typed_native_parameters_do_not_override_model_messages_or_streaming() {
    let parameters: nekochat_core::GenerationParameters = serde_json::from_value(json!({
        "temperature":0.4,"top_p":0.8,"max_output_tokens":1234,"seed":7,
        "frequency_penalty":0.2,"presence_penalty":-0.2,"stop":["END"]
    }))
    .unwrap();
    for style in [ReasoningStyle::DeepSeek, ReasoningStyle::Effort] {
        let request = ChatRequest {
            parameters: &parameters,
            ..req(style.clone(), Some("high"))
        };
        let body = build_body(&request);
        assert_eq!(body["model"], "m");
        assert_eq!(body["messages"], json!([]));
        assert_eq!(body["stream"], true);
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["top_p"], 0.8);
        assert_eq!(body["seed"], 7);
        assert_eq!(body["frequency_penalty"], 0.2);
        assert_eq!(body["presence_penalty"], -0.2);
        assert_eq!(body["stop"], json!(["END"]));
        let key = if style == ReasoningStyle::DeepSeek {
            "max_tokens"
        } else {
            "max_completion_tokens"
        };
        assert_eq!(body[key], 1234);
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("tools").is_none());
    }
}

fn provider(base: &str) -> Provider {
    Provider {
        key: "p".into(),
        label: "P".into(),
        base_url: format!("{base}/v1/"),
        models_url: None,
        api_key: "k".into(),
        session_header: Some("x-opencode-session".into()),
    }
}

async fn serve(app: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

async fn collect(base: &str) -> Vec<StreamEvent> {
    let http = reqwest::Client::new();
    let s = stream_chat(&http, &provider(base), &req(ReasoningStyle::None, None))
        .await
        .unwrap();
    s.map(|e| e.unwrap()).collect().await
}

#[test]
fn deepseek_none_disables_thinking() {
    let b = build_body(&req(ReasoningStyle::DeepSeek, Some("none")));
    assert_eq!(b["thinking"], json!({"type":"disabled"}));
    assert!(b.get("reasoning_effort").is_none());
}

#[test]
fn deepseek_level_enables_thinking() {
    let b = build_body(&req(ReasoningStyle::DeepSeek, Some("max")));
    assert_eq!(b["thinking"], json!({"type":"enabled"}));
    assert_eq!(b["reasoning_effort"], "max");
}

#[test]
fn effort_style_sets_reasoning_effort_only() {
    let b = build_body(&req(ReasoningStyle::Effort, Some("xhigh")));
    assert!(b.get("thinking").is_none());
    assert_eq!(b["reasoning_effort"], "xhigh");
}

#[test]
fn body_always_streams_with_usage_and_no_tools() {
    let b = build_body(&req(ReasoningStyle::None, None));
    assert_eq!(b["stream"], true);
    assert_eq!(b["stream_options"], json!({"include_usage": true}));
    assert!(b.get("tools").is_none() && b.get("reasoning_effort").is_none());
}

#[test]
fn body_serializes_messages() {
    let msgs = [
        ChatMessage {
            role: ChatRole::System,
            content: "s".into(),
        },
        ChatMessage {
            role: ChatRole::User,
            content: "u".into(),
        },
        ChatMessage {
            role: ChatRole::Assistant,
            content: "a".into(),
        },
    ];
    let r = ChatRequest {
        messages: &msgs,
        ..req(ReasoningStyle::None, None)
    };
    let b = build_body(&r);
    assert_eq!(b["model"], "m");
    assert_eq!(
        b["messages"],
        json!([{"role":"system","content":"s"},{"role":"user","content":"u"},{"role":"assistant","content":"a"}])
    );
}

#[test]
fn usage_opencode() {
    let u = parse_usage(
        &json!({"prompt_tokens":1200,"completion_tokens":300,"cost":0.0,
      "opencode":{"allowance":0.0041,"tokens":{"input":1200,"output":300,"reasoning":120,"cache_read":8000,"cache_write_5m":10,"cache_write_1h":0}}}),
    );
    assert_eq!(
        (u.cached_in, u.uncached_in, u.reasoning, u.output),
        (8000, 1210, 120, 180)
    );
    assert_eq!(
        (u.upstream_cost_usd, u.allowance_usd),
        (Some(0.0), Some(0.0041))
    );
}

#[test]
fn negative_billing_values_are_unknown_not_credit() {
    let usage = parse_usage(&json!({
        "cost": -1.0,
        "opencode": {"allowance": -2.0, "tokens": {"input": 1, "output": 0}}
    }));
    assert_eq!((usage.upstream_cost_usd, usage.allowance_usd), (None, None));
}

#[test]
fn usage_deepseek() {
    let u = parse_usage(
        &json!({"prompt_tokens":1204,"prompt_cache_hit_tokens":896,"prompt_cache_miss_tokens":308,
      "completion_tokens":426,"completion_tokens_details":{"reasoning_tokens":140}}),
    );
    assert_eq!(
        (u.cached_in, u.uncached_in, u.reasoning, u.output),
        (896, 308, 140, 286)
    );
}

#[test]
fn usage_openai_style() {
    let u = parse_usage(
        &json!({"prompt_tokens":1000,"prompt_tokens_details":{"cached_tokens":600},"completion_tokens":50}),
    );
    assert_eq!(
        (u.cached_in, u.uncached_in, u.reasoning, u.output),
        (600, 400, 0, 50)
    );
}

#[tokio::test]
async fn stream_yields_reasoning_text_usage_in_order() {
    let body = concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"想\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"喵\"}}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n",
        "data: [DONE]\n\n",
    );
    let base =
        serve(Router::new().route("/v1/chat/completions", post(move || async move { body }))).await;
    let ev = collect(&base).await;
    assert_eq!(ev.len(), 3);
    assert_eq!(ev[0], StreamEvent::Reasoning("想".into()));
    assert_eq!(ev[1], StreamEvent::Text("喵".into()));
    match &ev[2] {
        StreamEvent::Usage(u) => assert_eq!((u.uncached_in, u.output), (10, 2)),
        e => panic!("{e:?}"),
    }
}

#[tokio::test]
async fn reasoning_field_fallback() {
    let body = "data: {\"choices\":[{\"delta\":{\"reasoning\":\"r\"}}]}\n\ndata: [DONE]\n\n";
    let base =
        serve(Router::new().route("/v1/chat/completions", post(move || async move { body }))).await;
    assert_eq!(
        collect(&base).await,
        vec![StreamEvent::Reasoning("r".into())]
    );
}

#[tokio::test]
async fn stream_sends_user_agent_and_session_header() {
    let seen: Arc<Mutex<Option<(HeaderMap, serde_json::Value)>>> = Arc::default();
    let s2 = seen.clone();
    let app = Router::new().route(
        "/v1/chat/completions",
        post(
            move |h: HeaderMap, axum::Json(v): axum::Json<serde_json::Value>| async move {
                *s2.lock().unwrap() = Some((h, v));
                "data: [DONE]\n\n"
            },
        ),
    );
    let base = serve(app).await;
    assert!(collect(&base).await.is_empty());
    let (h, v) = seen.lock().unwrap().take().unwrap();
    assert!(h["user-agent"].to_str().unwrap().starts_with("nekochat/"));
    assert_eq!(h["x-opencode-session"], "sess-1");
    assert_eq!(h["authorization"], "Bearer k");
    assert_eq!(v["stream"], true);
}

#[tokio::test]
async fn utf8_split_across_chunks_is_intact() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            let full =
                "data: {\"choices\":[{\"delta\":{\"content\":\"喵喵\"}}]}\n\ndata: [DONE]\n\n"
                    .as_bytes();
            let cut = full.windows(3).position(|w| w == "喵".as_bytes()).unwrap() + 2;
            let (a, b) = (full[..cut].to_vec(), full[cut..].to_vec());
            let s = async_stream(a, b);
            Body::from_stream(s)
        }),
    );
    let base = serve(app).await;
    let text: String = collect(&base)
        .await
        .into_iter()
        .map(|e| match e {
            StreamEvent::Text(t) => t,
            e => panic!("{e:?}"),
        })
        .collect();
    assert_eq!(text, "喵喵");
}

fn async_stream(
    a: Vec<u8>,
    b: Vec<u8>,
) -> impl futures_util::Stream<Item = Result<Vec<u8>, std::io::Error>> {
    futures_util::stream::unfold(0u8, move |i| {
        let (a, b) = (a.clone(), b.clone());
        async move {
            match i {
                0 => Some((Ok(a), 1)),
                1 => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    Some((Ok(b), 2))
                }
                _ => None,
            }
        }
    })
}

#[tokio::test]
async fn early_close_is_transport_and_bad_json_is_protocol() {
    let app = Router::new()
        .route(
            "/a/v1/chat/completions",
            post(|| async { "data: {\"choices\":[]}\n\n" }),
        )
        .route(
            "/b/v1/chat/completions",
            post(|| async { "data: nope\n\n" }),
        );
    let base = serve(app).await;
    let http = reqwest::Client::new();
    let r = req(ReasoningStyle::None, None);
    let last = |s: Vec<Result<StreamEvent, GatewayError>>| s.into_iter().last().unwrap();
    let a = stream_chat(&http, &provider(&format!("{base}/a")), &r)
        .await
        .unwrap();
    assert!(matches!(
        last(a.collect().await),
        Err(GatewayError::Transport(_))
    ));
    let b = stream_chat(&http, &provider(&format!("{base}/b")), &r)
        .await
        .unwrap();
    assert!(matches!(
        last(b.collect().await),
        Err(GatewayError::Protocol(_))
    ));
}

#[tokio::test]
async fn in_stream_error_object_ends_with_protocol_error() {
    let app = Router::new()
        .route(
            "/a/v1/chat/completions",
            post(|| async {
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
                    "data: {\"error\":{\"message\":\"overloaded\",\"code\":529}}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"after\"}}]}\n\n",
                    "data: [DONE]\n\n"
                )
            }),
        )
        .route(
            "/b/v1/chat/completions",
            post(|| async { "data: {\"error\":{\"code\":42}}\n\ndata: [DONE]\n\n" }),
        );
    let base = serve(app).await;
    let http = reqwest::Client::new();
    let r = req(ReasoningStyle::None, None);
    let a: Vec<_> = stream_chat(&http, &provider(&format!("{base}/a")), &r)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(a.len(), 2, "{a:?}");
    assert_eq!(a[0].as_ref().unwrap(), &StreamEvent::Text("hi".into()));
    assert!(
        matches!(&a[1], Err(GatewayError::Protocol(m)) if m == "overloaded"),
        "{a:?}"
    );
    let b: Vec<_> = stream_chat(&http, &provider(&format!("{base}/b")), &r)
        .await
        .unwrap()
        .collect()
        .await;
    assert_eq!(b.len(), 1, "{b:?}");
    assert!(
        matches!(&b[0], Err(GatewayError::Protocol(m)) if m.contains("42")),
        "{b:?}"
    );
}

#[tokio::test]
async fn http_error_carries_status_and_message() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(json!({"error":{"message":"rate limited"}})),
            )
                .into_response()
        }),
    );
    let base = serve(app).await;
    let http = reqwest::Client::new();
    let err = stream_chat(&http, &provider(&base), &req(ReasoningStyle::None, None))
        .await
        .err()
        .unwrap();
    let text = err.to_string();
    match err {
        GatewayError::Http { status, message } => {
            assert_eq!(status, 429);
            assert_eq!(message, "rate limited");
        }
        e => panic!("{e:?}"),
    }
    assert!(text.contains("429") && text.contains("rate limited"));
}

#[tokio::test]
async fn http_error_without_json_uses_raw_text() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async { (StatusCode::BAD_GATEWAY, "x".repeat(600)).into_response() }),
    );
    let base = serve(app).await;
    let http = reqwest::Client::new();
    match stream_chat(&http, &provider(&base), &req(ReasoningStyle::None, None)).await {
        Err(GatewayError::Http {
            status: 502,
            message,
        }) => assert_eq!(message.len(), 500),
        _ => panic!(),
    }
}

#[tokio::test]
async fn list_models_uses_models_url() {
    let app = Router::new().route(
        "/v1/models",
        get(|h: HeaderMap| async move {
            assert!(h["user-agent"].to_str().unwrap().starts_with("nekochat/"));
            axum::Json(json!({"data":[{"id":"a"},{"id":"b"}]}))
        }),
    );
    let base = serve(app).await;
    let mut p = provider("http://127.0.0.1:1");
    p.models_url = Some(format!("{base}/v1/models"));
    let http = reqwest::Client::new();
    assert_eq!(list_models(&http, &p).await.unwrap(), vec!["a", "b"]);
    // base_url fallback with trailing slash trimmed
    assert_eq!(
        list_models(&http, &provider(&base)).await.unwrap(),
        vec!["a", "b"]
    );
}
