mod common;
use common::{MockScript, PROVIDER_KEY, TestCfg, spawn_app, spawn_mock_upstream};
use futures_util::StreamExt;
use nekochat_server::chat::{TurnEvent, start_turn};
use serde_json::{Value, json};

#[tokio::test]
async fn role_parameters_are_server_owned_and_shared_service_is_transport_independent() {
    let mock = spawn_mock_upstream(MockScript::sse(&[
        json!({"choices":[{"delta":{"content":"hello"}}]}),
    ]))
    .await;
    let app = spawn_app(TestCfg {
        upstream: Some(mock.base_url.clone()),
        admin_password: Some("admin".into()),
        ..Default::default()
    })
    .await;
    app.login_admin("admin").await;
    let model = app.store.insert_model(&serde_json::from_value(json!({"provider":PROVIDER_KEY,"upstream_id":"deepseek-v4-pro","display_name":"billing model","sort":0,"protocol":"chat_completions","reasoning":{"style":"deepseek","levels":["high"],"default":"high"},"vision":false,"context_window":null,"price":{"cached_in":1,"uncached_in":2,"output":3}})).unwrap()).unwrap();
    let body = json!({"name":"private role", "display_name":"Public role", "model_id":model.id, "system_prompt":"ROLE SECRET", "parameters":{"effort":"high","temperature":0.7,"top_p":0.8,"max_output_tokens":1234,"seed":42,"frequency_penalty":0.1,"presence_penalty":-0.1,"stop":["END"]}});
    let response = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let role: Value = response.json().await.unwrap();
    let conv = app.store.create_conversation(role["id"].as_str()).unwrap();
    for (key, value) in [
        ("model_id", json!("attacker")),
        ("effort", json!("max")),
        ("parameters", json!({"temperature":2})),
        ("backend", json!("attacker")),
    ] {
        let mut request = json!({"action":"send","content":"hi","character_id":role["id"]});
        request[key] = value;
        assert_eq!(
            app.client
                .post(app.url(&format!("/api/conversations/{}/turns", conv.id)))
                .json(&request)
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    assert!(mock.requests.lock().unwrap().is_empty());
    let response = app
        .client
        .post(app.url(&format!("/api/conversations/{}/turns", conv.id)))
        .json(&json!({"action":"send","content":"hi","character_id":role["id"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let sse = response.text().await.unwrap();
    assert!(sse.contains("Public role"));
    assert!(!sse.contains("ROLE SECRET"));
    assert!(!sse.contains("billing model"));
    let mut alternate_body = body.clone();
    alternate_body["display_name"] = json!("Alternate role");
    let alternate: Value = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&alternate_body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let request = serde_json::from_value(
        json!({"action":"send","content":"Rust adapter","character_id":alternate["id"]}),
    )
    .unwrap();
    let mut events = start_turn(app.state.clone(), conv.id.clone(), request)
        .await
        .unwrap();
    let mut completed = false;
    while let Some(event) = events.next().await {
        if let TurnEvent::Done(node) = event {
            assert_eq!(node.character_name.as_deref(), Some("Alternate role"));
            completed = true;
        }
    }
    assert!(completed);
    let nodes = app.store.nodes(&conv.id).unwrap();
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.character_name.as_deref() == Some("Public role"))
            .count(),
        1
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node.character_name.as_deref() == Some("Alternate role"))
            .count(),
        1
    );
    assert_eq!(
        app.store
            .get_conversation(&conv.id)
            .unwrap()
            .unwrap()
            .character_id
            .as_deref(),
        alternate["id"].as_str()
    );
    let requests = mock.requests.lock().unwrap();
    let sent = &requests[0].body;
    assert_eq!(sent["model"], "deepseek-v4-pro");
    assert_eq!(sent["temperature"], 0.7);
    assert_eq!(sent["top_p"], 0.8);
    assert_eq!(sent["max_tokens"], 1234);
    assert_eq!(sent["seed"], 42);
    assert_eq!(sent["frequency_penalty"], 0.1);
    assert_eq!(sent["presence_penalty"], -0.1);
    assert_eq!(sent["stop"], json!(["END"]));
    assert_eq!(sent["reasoning_effort"], "high");
    assert_eq!(sent["messages"][0]["content"], "ROLE SECRET");
}
