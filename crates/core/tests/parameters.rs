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
