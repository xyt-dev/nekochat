//! OpenAI-compatible Chat Completions adapter.

use std::collections::VecDeque;

use eventsource_stream::Eventsource;
use futures_util::stream::{self, BoxStream, StreamExt};
use nekochat_core::{ReasoningStyle, Usage};
use serde_json::{Value, json};

use crate::{ChatRequest, ChatRole, GatewayError, Provider, StreamEvent, USER_AGENT};

fn endpoint(base_url: &str, path: &str) -> String {
    let base = base_url.strip_suffix('/').unwrap_or(base_url);
    format!("{base}{path}")
}

fn role_name(r: ChatRole) -> &'static str {
    match r {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
    }
}

/// Builds the request body. Never includes `tools`.
pub fn build_body(req: &ChatRequest) -> Value {
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| json!({"role": role_name(m.role), "content": m.content}))
        .collect();
    let mut b = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    match (&req.reasoning, req.effort) {
        (ReasoningStyle::Effort, Some(l)) => {
            b["reasoning_effort"] = json!(l);
        }
        (ReasoningStyle::DeepSeek, Some("none")) => {
            b["thinking"] = json!({"type": "disabled"});
        }
        (ReasoningStyle::DeepSeek, Some(l)) => {
            b["thinking"] = json!({"type": "enabled"});
            b["reasoning_effort"] = json!(l);
        }
        _ => {}
    }
    b
}

fn u(v: &Value, ptr: &str) -> u64 {
    v.pointer(ptr).and_then(Value::as_u64).unwrap_or(0)
}

/// Normalizes an upstream `usage` object.
pub fn parse_usage(v: &Value) -> Usage {
    if let Some(t) = v.pointer("/opencode/tokens") {
        let reasoning = u(t, "/reasoning");
        return Usage {
            cached_in: u(t, "/cache_read"),
            uncached_in: u(t, "/input") + u(t, "/cache_write_5m") + u(t, "/cache_write_1h"),
            reasoning,
            output: u(t, "/output").saturating_sub(reasoning),
            upstream_cost_usd: v.get("cost").and_then(Value::as_f64),
            allowance_usd: v.pointer("/opencode/allowance").and_then(Value::as_f64),
        };
    }
    let (cached_in, uncached_in) = if v.get("prompt_cache_hit_tokens").is_some() {
        (
            u(v, "/prompt_cache_hit_tokens"),
            u(v, "/prompt_cache_miss_tokens"),
        )
    } else {
        let cached = u(v, "/prompt_tokens_details/cached_tokens");
        (cached, u(v, "/prompt_tokens").saturating_sub(cached))
    };
    let reasoning = u(v, "/completion_tokens_details/reasoning_tokens");
    Usage {
        cached_in,
        uncached_in,
        reasoning,
        output: u(v, "/completion_tokens").saturating_sub(reasoning),
        upstream_cost_usd: None,
        allowance_usd: None,
    }
}

fn chunk_events(v: &Value, out: &mut VecDeque<StreamEvent>) {
    let delta = v.pointer("/choices/0/delta");
    let text_of = |k: &str| {
        delta
            .and_then(|d| d.get(k))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    if let Some(r) = text_of("reasoning_content").or_else(|| text_of("reasoning")) {
        out.push_back(StreamEvent::Reasoning(r.to_owned()));
    }
    if let Some(t) = text_of("content") {
        out.push_back(StreamEvent::Text(t.to_owned()));
    }
    if let Some(us) = v.get("usage").filter(|x| x.is_object()) {
        out.push_back(StreamEvent::Usage(parse_usage(us)));
    }
}

fn transport(e: reqwest::Error) -> GatewayError {
    GatewayError::Transport(e.to_string())
}

async fn check_status(resp: reqwest::Response) -> Result<reqwest::Response, GatewayError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let text = resp.text().await.unwrap_or_default();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| text.chars().take(500).collect());
    Err(GatewayError::Http {
        status: status.as_u16(),
        message,
    })
}

/// POSTs `{base_url}/chat/completions` and streams parsed events.
/// Dropping the returned stream aborts the upstream body read.
pub async fn stream_chat(
    http: &reqwest::Client,
    p: &Provider,
    req: &ChatRequest<'_>,
) -> Result<BoxStream<'static, Result<StreamEvent, GatewayError>>, GatewayError> {
    let mut rb = http
        .post(endpoint(&p.base_url, "/chat/completions"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .bearer_auth(&p.api_key)
        .json(&build_body(req));
    if let Some(h) = &p.session_header {
        rb = rb.header(h.as_str(), req.session_id);
    }
    let resp = check_status(rb.send().await.map_err(transport)?).await?;
    let events = resp.bytes_stream().eventsource();

    // State: (sse stream, pending events, finished)
    let s = stream::unfold(
        (events, VecDeque::new(), false),
        |(mut events, mut pending, done)| async move {
            loop {
                if let Some(ev) = pending.pop_front() {
                    return Some((Ok(ev), (events, pending, done)));
                }
                if done {
                    return None;
                }
                match events.next().await {
                    None => {
                        let err = GatewayError::Transport("stream ended before [DONE]".into());
                        return Some((Err(err), (events, pending, true)));
                    }
                    Some(Err(e)) => {
                        let err = GatewayError::Transport(e.to_string());
                        return Some((Err(err), (events, pending, true)));
                    }
                    Some(Ok(ev)) => {
                        let data = ev.data.trim();
                        if data.is_empty() {
                            continue;
                        }
                        if data == "[DONE]" {
                            return None;
                        }
                        match serde_json::from_str::<Value>(data) {
                            Ok(v) => {
                                if let Some(e) = v.get("error").filter(|e| e.is_object()) {
                                    let message = e
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .map_or_else(|| e.to_string(), str::to_owned);
                                    let err = GatewayError::Protocol(message);
                                    return Some((Err(err), (events, pending, true)));
                                }
                                chunk_events(&v, &mut pending)
                            }
                            Err(e) => {
                                let err = GatewayError::Protocol(format!("invalid SSE JSON: {e}"));
                                return Some((Err(err), (events, pending, true)));
                            }
                        }
                    }
                }
            }
        },
    );
    Ok(s.boxed())
}

/// GETs `models_url` (or `{base_url}/models`) and returns `data[].id`.
pub async fn list_models(
    http: &reqwest::Client,
    p: &Provider,
) -> Result<Vec<String>, GatewayError> {
    let url = p
        .models_url
        .clone()
        .unwrap_or_else(|| endpoint(&p.base_url, "/models"));
    let resp = http
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .bearer_auth(&p.api_key)
        .send()
        .await
        .map_err(transport)?;
    let v: Value = check_status(resp).await?.json().await.map_err(transport)?;
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::Protocol("models response lacks data[]".into()))?;
    Ok(data
        .iter()
        .filter_map(|m| m.get("id")?.as_str().map(str::to_owned))
        .collect())
}
