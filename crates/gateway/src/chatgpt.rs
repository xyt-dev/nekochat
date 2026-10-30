//! Official Sign in with ChatGPT plan usage: public Responses, never backend-api.
//! https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations
use crate::{ChatRequest, ChatRole, GatewayError, Provider, StreamEvent, USER_AGENT};
use futures_util::stream::BoxStream;
use nekochat_core::{Protocol, ReasoningStyle};
use serde_json::{Value, json};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessage;
    use nekochat_core::GenerationParameters;

    #[test]
    fn native_subscription_body_preserves_history_without_system_items_or_tools() {
        let messages = vec![
            ChatMessage {
                role: ChatRole::System,
                content: "角色指令".into(),
                images: vec![],
            },
            ChatMessage {
                role: ChatRole::User,
                content: "你好".into(),
                images: vec![],
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: "你好呀".into(),
                images: vec![],
            },
        ];
        let parameters = GenerationParameters::default();
        let request = ChatRequest {
            model: "account-model",
            messages: &messages,
            reasoning: ReasoningStyle::Effort,
            effort: Some("high"),
            session_id: "conversation",
            parameters: &parameters,
            tools: None,
        };
        let value = body(&request).unwrap();
        assert_eq!(value["instructions"], "角色指令");
        assert_eq!(value["input"].as_array().unwrap().len(), 2);
        assert_eq!(value["input"][0]["role"], "user");
        assert_eq!(value["input"][1]["role"], "assistant");
        assert_eq!(value["store"], false);
        assert_eq!(value["stream"], true);
        assert_eq!(value["reasoning"]["effort"], "high");
        assert!(value.get("tools").is_none());
        assert!(value.get("previous_response_id").is_none());
    }

    #[test]
    fn subscription_body_rejects_unsupported_sampling_even_when_called_directly() {
        let parameters = GenerationParameters {
            temperature: Some(1.0),
            ..Default::default()
        };
        let request = ChatRequest {
            model: "account-model",
            messages: &[],
            reasoning: ReasoningStyle::None,
            effort: None,
            session_id: "conversation",
            parameters: &parameters,
            tools: None,
        };
        assert!(body(&request).is_err());
    }
}

fn body(request: &ChatRequest<'_>) -> Result<Value, GatewayError> {
    request
        .parameters
        .validate_for_protocol(&Protocol::ChatgptResponses)
        .map_err(|message| GatewayError::Protocol(message.into()))?;
    if request.reasoning == ReasoningStyle::DeepSeek {
        return Err(GatewayError::Protocol(
            "ChatGPT 订阅不支持 DeepSeek thinking".into(),
        ));
    }
    let initial_system_count = request
        .messages
        .iter()
        .take_while(|message| message.role == ChatRole::System)
        .count();
    let mut instructions = String::new();
    for (index, message) in request.messages[..initial_system_count].iter().enumerate() {
        if index != 0 {
            instructions.push_str("\n\n");
        }
        instructions.push_str(&message.content);
    }
    let mut input = crate::responses::input_messages(&request.messages[initial_system_count..]);
    if let Some(tools) = request.tools {
        tools.extend_responses(&mut input);
    }
    let mut value = json!({"model":request.model,"input":input,"stream":true,"store":false});
    if let Some(tools) = request.tools {
        value["include"] = json!(["reasoning.encrypted_content"]);
        if !tools.definitions.is_empty() {
            value["tools"] = tools.response_definitions(true);
        }
    }
    if !instructions.is_empty() {
        value["instructions"] = json!(instructions);
    }
    if let Some(effort) = request.effort {
        value["reasoning"] = json!({"effort":effort});
    }
    Ok(value)
}

pub async fn stream_chatgpt(
    http: &reqwest::Client,
    provider: &Provider,
    request: &ChatRequest<'_>,
) -> Result<BoxStream<'static, Result<StreamEvent, GatewayError>>, GatewayError> {
    let response = http
        .post(crate::http::endpoint(&provider.base_url, "/responses"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT, "text/event-stream")
        .bearer_auth(&provider.api_key)
        .json(&body(request)?)
        .send()
        .await
        .map_err(crate::http::transport)?;
    let response = crate::http::check_status(response).await?;
    Ok(crate::responses::event_stream(response, request.tools.is_some()))
}
