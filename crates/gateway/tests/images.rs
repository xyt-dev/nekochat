use axum::{Json, Router, routing::post};
use futures_util::StreamExt;
use nekochat_core::{GenerationParameters, ReasoningStyle};
use nekochat_gateway::{
    ChatImage, ChatMessage, ChatRequest, ChatRole, Provider, stream_chat, stream_responses,
};
use serde_json::Value;

#[tokio::test]
async fn local_images_are_encoded_as_protocol_image_parts_not_filename_text() {
    for responses in [false, true] {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let app = Router::new().route(if responses { "/responses" } else { "/chat/completions" },post(move |Json(body):Json<Value>| {
            let tx=tx.clone();
            async move {
                tx.send(body).await.unwrap();
                ([ ("content-type","text/event-stream") ], if responses { "data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\"}}\n\n" } else { "data: [DONE]\n\n" })
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = Provider {
            key: "test".into(),
            label: "test".into(),
            base_url: format!("http://{address}"),
            models_url: None,
            api_key: "fixture".into(),
            session_header: None,
        };
        let parameters = GenerationParameters::default();
        let messages = [
            ChatMessage {
                role: ChatRole::System,
                content: "role instructions".into(),
                images: vec![],
            },
            ChatMessage {
                role: ChatRole::User,
                content: "describe".into(),
                images: vec![ChatImage {
                    media_type: "image/png".into(),
                    base64: "iVBORw0KGgo=".into(),
                }],
            },
            ChatMessage {
                role: ChatRole::Assistant,
                content: "old reply".into(),
                images: vec![],
            },
            ChatMessage {
                role: ChatRole::User,
                content: String::new(),
                images: vec![ChatImage {
                    media_type: "image/webp".into(),
                    base64: "UklGRg==".into(),
                }],
            },
        ];
        let request = ChatRequest {
            model: "vision",
            messages: &messages,
            reasoning: ReasoningStyle::None,
            effort: None,
            session_id: "session",
            parameters: &parameters,
            tools: None,
        };
        let http = reqwest::Client::new();
        let stream = if responses {
            stream_responses(&http, &provider, &request).await
        } else {
            stream_chat(&http, &provider, &request).await
        }
        .unwrap();
        stream.collect::<Vec<_>>().await;
        let body = rx.recv().await.unwrap();
        let input = &body[if responses { "input" } else { "messages" }];
        assert_eq!(
            input[0]["role"],
            if responses { "developer" } else { "system" }
        );
        assert_eq!(input[2]["content"], "old reply");
        assert_eq!(
            input[1]["content"][0]["type"],
            if responses { "input_text" } else { "text" }
        );
        let image = &input[1]["content"][1];
        assert_eq!(
            image["type"],
            if responses {
                "input_image"
            } else {
                "image_url"
            }
        );
        assert_eq!(
            if responses {
                &image["image_url"]
            } else {
                &image["image_url"]["url"]
            },
            "data:image/png;base64,iVBORw0KGgo="
        );
        assert_eq!(input[3]["content"].as_array().unwrap().len(), 1);
        assert_eq!(body.get("previous_response_id"), None);
        assert_eq!(body.get("tools"), None);
        task.abort();
    }
}
