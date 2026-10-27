//! Stateless native OpenAI Responses, including OpenCode Go's usage extensions.
use std::collections::VecDeque;

use eventsource_stream::Eventsource;
use futures_util::{
    StreamExt,
    stream::{self, BoxStream},
};
use nekochat_core::{Protocol, ReasoningStyle, Usage};
use serde_json::{Value, json};

use crate::http::{check_status, endpoint, role_name, transport};
use crate::{ChatRequest, GatewayError, Provider, StreamEvent, USER_AGENT};

fn body(request: &ChatRequest<'_>) -> Result<Value, GatewayError> {
    request
        .parameters
        .validate_for_protocol(&Protocol::Responses)
        .map_err(|message| GatewayError::Protocol(message.into()))?;
    if request.reasoning == ReasoningStyle::DeepSeek {
        return Err(GatewayError::Protocol(
            "Responses 不支持 DeepSeek thinking 模式".into(),
        ));
    }
    let input: Vec<_> = request
        .messages
        .iter()
        .map(|message| json!({"role":role_name(message.role),"content":message.content}))
        .collect();
    let mut body = json!({"model":request.model,"input":input,"stream":true,"store":false});
    if let Some(effort) = request.effort {
        body["reasoning"] = json!({"effort":effort,"summary":"auto"});
    }
    let parameters = request.parameters;
    for (key, value) in [
        (
            "temperature",
            parameters.temperature.map(|value| json!(value)),
        ),
        ("top_p", parameters.top_p.map(|value| json!(value))),
        (
            "max_output_tokens",
            parameters.max_output_tokens.map(|value| json!(value)),
        ),
    ] {
        if let Some(value) = value {
            body[key] = value;
        }
    }
    Ok(body)
}

fn usage(value: &Value) -> Usage {
    if value.pointer("/opencode/tokens").is_some() {
        return crate::parse_usage(value);
    }
    let count = |path: &str| value.pointer(path).and_then(Value::as_u64).unwrap_or(0);
    let money = |path: &str| {
        value
            .pointer(path)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
    };
    let cached_in = count("/input_tokens_details/cached_tokens");
    let reasoning = count("/output_tokens_details/reasoning_tokens");
    Usage {
        cached_in,
        uncached_in: count("/input_tokens").saturating_sub(cached_in),
        reasoning,
        output: count("/output_tokens").saturating_sub(reasoning),
        upstream_cost_usd: money("/cost"),
        allowance_usd: money("/opencode/allowance"),
    }
}

fn terminal_message(value: &Value, incomplete: bool) -> String {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .map(|message| message.chars().take(500).collect())
        .unwrap_or_else(|| {
            if incomplete {
                let reason = value
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                format!("Responses 回复未完成：{reason}")
            } else {
                "Responses 上游返回失败".into()
            }
        })
}

pub async fn stream_responses(
    http: &reqwest::Client,
    provider: &Provider,
    request: &ChatRequest<'_>,
) -> Result<BoxStream<'static, Result<StreamEvent, GatewayError>>, GatewayError> {
    let mut builder = http
        .post(endpoint(&provider.base_url, "/responses"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .bearer_auth(&provider.api_key)
        .json(&body(request)?);
    if let Some(header) = &provider.session_header {
        builder = builder.header(header.as_str(), request.session_id);
    }
    let response = check_status(builder.send().await.map_err(transport)?).await?;
    let events = response.bytes_stream().eventsource();
    Ok(stream::unfold(
        (events, VecDeque::new(), false),
        |(mut events, mut pending, done)| async move {
            let mut done = done;
            loop {
                if let Some(event) = pending.pop_front() {
                    return Some((event, (events, pending, done)));
                }
                if done {
                    return None;
                }
                let event = match events.next().await {
                    Some(Ok(event)) => event,
                    Some(Err(error)) => {
                        return Some((
                            Err(GatewayError::Transport(error.to_string())),
                            (events, pending, true),
                        ));
                    }
                    None => {
                        return Some((
                            Err(GatewayError::Transport(
                                "Responses stream ended before response.completed".into(),
                            )),
                            (events, pending, true),
                        ));
                    }
                };
                let data = event.data.trim();
                if data.is_empty() {
                    continue;
                }
                if data == "[DONE]" {
                    return Some((
                        Err(GatewayError::Transport(
                            "Responses ended without response.completed".into(),
                        )),
                        (events, pending, true),
                    ));
                }
                let value: Value = match serde_json::from_str(data) {
                    Ok(value) => value,
                    Err(error) => {
                        return Some((
                            Err(GatewayError::Protocol(format!(
                                "invalid Responses SSE JSON: {error}"
                            ))),
                            (events, pending, true),
                        ));
                    }
                };
                let kind = value
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or(&event.event);
                match kind {
                    "response.output_text.delta" | "response.refusal.delta" => {
                        if let Some(text) = value
                            .get("delta")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                        {
                            pending.push_back(Ok(StreamEvent::Text(text.to_owned())));
                        }
                    }
                    "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                        if let Some(text) = value
                            .get("delta")
                            .and_then(Value::as_str)
                            .filter(|text| !text.is_empty())
                        {
                            pending.push_back(Ok(StreamEvent::Reasoning(text.to_owned())));
                        }
                    }
                    "response.completed" | "response.failed" | "response.incomplete" => {
                        if let Some(record) = value
                            .pointer("/response/usage")
                            .filter(|value| value.is_object())
                        {
                            pending.push_back(Ok(StreamEvent::Usage(usage(record))));
                        }
                        if kind != "response.completed" {
                            pending.push_back(Err(GatewayError::Protocol(terminal_message(
                                &value,
                                kind == "response.incomplete",
                            ))));
                        }
                        done = true;
                    }
                    "error" => {
                        pending.push_back(Err(GatewayError::Protocol(terminal_message(
                            &value, false,
                        ))));
                        done = true;
                    }
                    _ => {}
                }
            }
        },
    )
    .boxed())
}
