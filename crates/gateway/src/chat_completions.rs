//! OpenAI-compatible Chat Completions adapter.

use std::collections::VecDeque;

use eventsource_stream::Eventsource;
use futures_util::stream::{self, BoxStream, StreamExt};
use nekochat_core::{ReasoningStyle, Usage};
use serde_json::{Value, json};

use crate::http::{check_status, endpoint, role_name, transport};
use crate::{ChatRequest, GatewayError, Provider, StreamEvent, USER_AGENT};

/// Builds one completion; tools are exclusively supplied by the core's authorized role policy.
pub fn build_body(req: &ChatRequest) -> Value {
    let mut messages: Vec<Value> =
        req.messages
            .iter()
            .map(|m| {
                let content = if m.images.is_empty() {
                    json!(m.content)
                } else {
                    let mut parts = Vec::with_capacity(m.images.len() + 1);
                    if !m.content.is_empty() {
                        parts.push(json!({"type":"text","text":m.content}));
                    }
                    parts.extend(m.images.iter().map(
                        |image| json!({"type":"image_url","image_url":{"url":image.data_url()}}),
                    ));
                    json!(parts)
                };
                json!({"role": role_name(m.role), "content": content})
            })
            .collect();
    if let Some(tools) = req.tools {
        tools.extend_chat(&mut messages, req.reasoning == ReasoningStyle::DeepSeek);
    }
    let mut b = json!({
        "model": req.model,
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
    });
    if let Some(tools) = req.tools.filter(|tools| !tools.definitions.is_empty()) {
        b["tools"] = tools.chat_definitions();
    }
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
    let p = req.parameters;
    for (key, value) in [
        ("temperature", p.temperature.map(|v| json!(v))),
        ("top_p", p.top_p.map(|v| json!(v))),
        (
            if req.reasoning == ReasoningStyle::DeepSeek {
                "max_tokens"
            } else {
                "max_completion_tokens"
            },
            p.max_output_tokens.map(|v| json!(v)),
        ),
        ("seed", p.seed.map(|v| json!(v))),
        ("frequency_penalty", p.frequency_penalty.map(|v| json!(v))),
        ("presence_penalty", p.presence_penalty.map(|v| json!(v))),
    ] {
        if let Some(value) = value {
            b[key] = value;
        }
    }
    if !p.stop.is_empty() {
        b["stop"] = json!(p.stop);
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
            upstream_cost_usd: v
                .get("cost")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0),
            allowance_usd: v
                .pointer("/opencode/allowance")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite() && *v >= 0.0),
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
    if let Some(calls) = delta.and_then(|delta| delta.get("tool_calls")).and_then(Value::as_array) {
        for call in calls {
            let Some(index) = call.get("index").and_then(Value::as_u64).and_then(|index| usize::try_from(index).ok()) else {
                continue;
            };
            out.push_back(StreamEvent::ToolCallDelta(crate::ToolCallDelta {
                index,
                id: call.get("id").and_then(Value::as_str).map(str::to_owned),
                name: call.pointer("/function/name").and_then(Value::as_str).map(str::to_owned),
                arguments: call.pointer("/function/arguments").and_then(Value::as_str).unwrap_or_default().to_owned(),
            }));
        }
    }
    if let Some(us) = v.get("usage").filter(|x| x.is_object()) {
        out.push_back(StreamEvent::Usage(parse_usage(us)));
    }
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
    let mut resp = http
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .bearer_auth(&p.api_key)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(transport)?;
    if !resp.status().is_success() {
        return Err(GatewayError::Http {
            status: resp.status().as_u16(),
            message: "model catalog request failed".into(),
        });
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await.map_err(transport)? {
        if body.len() + chunk.len() > 2 * 1024 * 1024 {
            return Err(GatewayError::Protocol(
                "model catalog response too large".into(),
            ));
        }
        body.extend_from_slice(&chunk);
    }
    let v: Value = serde_json::from_slice(&body)
        .map_err(|_| GatewayError::Protocol("invalid model catalog JSON".into()))?;
    let data = v
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::Protocol("models response lacks data[]".into()))?;
    Ok(data
        .iter()
        .filter_map(|m| m.get("id")?.as_str().map(str::to_owned))
        .collect())
}
