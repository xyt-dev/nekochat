use nekochat_core::GenerationParameters;

#[test]
fn parameters_reject_arbitrary_request_overrides() {
    for field in ["model", "messages", "tools", "headers"] {
        let body = format!(r#"{{"{field}":null}}"#);
        assert!(serde_json::from_str::<GenerationParameters>(&body).is_err());
    }
}

#[test]
fn parameter_ranges_are_enforced() {
    for body in [
        r#"{"temperature":-1}"#,
        r#"{"top_p":1.1}"#,
        r#"{"max_output_tokens":0}"#,
        r#"{"frequency_penalty":3}"#,
    ] {
        let parameters: GenerationParameters = serde_json::from_str(body).unwrap();
        assert!(parameters.validate().is_err());
    }
    let parameters: GenerationParameters = serde_json::from_str(
        r#"{"temperature":0.7,"top_p":0.9,"max_output_tokens":2048,"seed":42,"stop":["END"]}"#,
    )
    .unwrap();
    assert!(parameters.validate().is_ok());
}

#[test]
fn responses_rejects_parameters_its_native_api_does_not_accept() {
    for body in [
        r#"{"seed":1}"#,
        r#"{"frequency_penalty":0}"#,
        r#"{"presence_penalty":0}"#,
        r#"{"stop":["END"]}"#,
    ] {
        let parameters: GenerationParameters = serde_json::from_str(body).unwrap();
        assert!(
            parameters
                .validate_for_protocol(&nekochat_core::Protocol::ChatCompletions)
                .is_ok()
        );
        assert!(
            parameters
                .validate_for_protocol(&nekochat_core::Protocol::Responses)
                .is_err()
        );
    }
    let parameters: GenerationParameters =
        serde_json::from_str(r#"{"effort":"high","max_output_tokens":2048}"#).unwrap();
    assert!(
        parameters
            .validate_for_protocol(&nekochat_core::Protocol::Responses)
            .is_ok()
    );
}

#[test]
fn subscription_responses_rejects_all_unsupported_generation_fields() {
    for body in [
        r#"{"temperature":0.7}"#,
        r#"{"top_p":0.9}"#,
        r#"{"max_output_tokens":2048}"#,
        r#"{"seed":1}"#,
        r#"{"frequency_penalty":0}"#,
        r#"{"presence_penalty":0}"#,
        r#"{"stop":["END"]}"#,
    ] {
        let parameters: GenerationParameters = serde_json::from_str(body).unwrap();
        assert!(
            parameters
                .validate_for_protocol(&nekochat_core::Protocol::ChatgptResponses)
                .is_err()
        );
    }
    let parameters: GenerationParameters = serde_json::from_str(r#"{"effort":"high"}"#).unwrap();
    assert!(
        parameters
            .validate_for_protocol(&nekochat_core::Protocol::ChatgptResponses)
            .is_ok()
    );
}
