use axum::{body::{Body, to_bytes}, http::{Request, StatusCode}};
use nekochat_search::{client::SearchClient, server::Engine, ServiceWrite};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn call(engine: &Engine, method: &str, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request = Request::builder().method(method).uri(path).header("content-type", "application/json");
    if let Some(token) = token { request = request.header("authorization", format!("Bearer {token}")); }
    let response = engine.router().oneshot(request.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn service(key: Option<&str>, clear: bool) -> Value {
    json!({"name":"Private search", "kind":"deepseek", "enabled":true,"auth_mode":"api_key", "api_key":key,"clear_api_key":clear})
}

#[tokio::test]
async fn runtime_and_admin_scopes_never_substitute_for_each_other() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(), "runtime-secret", "admin-secret").unwrap();
    assert_eq!(call(&engine,"GET","/healthz",None,json!(null)).await, (StatusCode::OK,json!({"status":"ok"})));
    for (path, token, expected) in [("/v1/services",None,StatusCode::UNAUTHORIZED),("/v1/admin/services",Some("runtime-secret"),StatusCode::FORBIDDEN),("/v1/services",Some("admin-secret"),StatusCode::FORBIDDEN),("/v1/admin/services",Some("admin-secret"),StatusCode::OK)] {
        assert_eq!(call(&engine,"GET",path,token,json!(null)).await.0, expected);
    }
    assert!(Engine::open(dir.path(), "same", "same").is_err());
}

#[tokio::test]
async fn credentials_preserve_clear_redact_and_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(), "runtime-secret", "admin-secret").unwrap();
    let (status, created) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),service(Some("never-return-this-key"),false)).await;
    assert_eq!(status,StatusCode::CREATED);
    assert_eq!(created["has_api_key"],true);
    assert!(!created.to_string().contains("never-return-this-key"));
    let path = format!("/v1/admin/services/{}",created["id"].as_str().unwrap());
    for key in [None,Some("")] {
        let (_, updated) = call(&engine,"PUT",&path,Some("admin-secret"),service(key,false)).await;
        assert_eq!(updated["has_api_key"],true);
    }
    assert!(Engine::open(dir.path(),"runtime-secret","admin-secret").is_err(),"exclusive engine data lock");
    drop(engine);
    let engine = Engine::open(dir.path(),"runtime-secret","admin-secret").unwrap();
    let (_, views) = call(&engine,"GET","/v1/admin/services",Some("admin-secret"),json!(null)).await;
    assert_eq!(views[0]["has_api_key"],true);
    assert!(!views.to_string().contains("never-return-this-key"));
    let (_, cleared) = call(&engine,"PUT",&path,Some("admin-secret"),service(None,true)).await;
    assert_eq!(cleared["has_api_key"],false);
    assert_eq!(cleared["connected"],false);
    let write: ServiceWrite = serde_json::from_value(service(Some("never-debug-this-key"),false)).unwrap();
    assert!(!format!("{write:?}").contains("never-debug-this-key"));
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(dir.path().join("services.json")).unwrap().permissions().mode() & 0o777,0o600);
    }
}

#[tokio::test]
async fn cancel_before_registration_prevents_provider_work() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(),"runtime-secret","admin-secret").unwrap();
    let (_, created) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),service(Some("not-a-real-key"),false)).await;
    let id = "c453b2d8-3877-4373-9eeb-ef8263703002";
    assert_eq!(call(&engine,"POST","/v1/cancel",Some("runtime-secret"),json!({"request_id":id})).await.0,StatusCode::NO_CONTENT);
    let request = json!({"request_id":id,"service_id":created["id"],"query":"public test","limit":5});
    let (status, error) = call(&engine,"POST","/v1/search",Some("runtime-secret"),request).await;
    assert_eq!(status,StatusCode::CONFLICT);
    assert!(error["error"].as_str().unwrap().contains("cancel"));
}

#[tokio::test]
async fn fetch_rejects_local_destinations_before_provider_work() {
    let dir = tempfile::tempdir().unwrap();
    let engine = Engine::open(dir.path(),"runtime-secret","admin-secret").unwrap();
    let (_, created) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),service(Some("not-a-real-key"),false)).await;
    for url in ["file:///etc/passwd","/etc/passwd","http://127.0.0.1/","http://[::1]/","http://169.254.169.254/","http://192.168.1.1/","http://user:pass@example.com/","http://localhost/","http://intranet/","http://metadata.google.internal/","http://[::ffff:127.0.0.1]/"] {
        let request = json!({"request_id":uuid::Uuid::new_v4().to_string(),"service_id":created["id"],"url":url});
        assert_eq!(call(&engine,"POST","/v1/fetch",Some("runtime-secret"),request).await.0,StatusCode::BAD_REQUEST,"{url}");
    }
}

#[test]
fn pooled_client_rejects_credential_bearing_base_urls_and_debug_redacts_tokens() {
    assert!(SearchClient::new("http://user:password@localhost:10011", "secret",std::time::Duration::from_secs(10)).is_err());
    let client = SearchClient::new("http://127.0.0.1:10011", "never-debug-token",std::time::Duration::from_secs(10)).unwrap();
    assert!(!format!("{client:?}").contains("never-debug-token"));
}

#[cfg(unix)]
#[test]
fn private_vault_rejects_symlinked_credential_files_without_touching_the_target() {
    use std::os::unix::fs::symlink;
    let directory = tempfile::tempdir().unwrap();
    let data = directory.path().join("data");
    std::fs::create_dir(&data).unwrap();
    let target = directory.path().join("unrelated-file");
    std::fs::write(&target,"must remain unchanged").unwrap();
    symlink(&target,data.join("services.json")).unwrap();
    assert!(Engine::open(&data,"runtime-secret","admin-secret").is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(),"must remain unchanged");
}

#[tokio::test]
async fn malformed_secret_inputs_get_fixed_errors_and_disabled_services_stay_out_of_runtime_catalog() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let mut invalid = service(None,false);
    invalid["api_key"] = json!({"never-echo-this-secret":true});
    let (status,error) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),invalid).await;
    assert_eq!(status,StatusCode::BAD_REQUEST);
    assert!(!error.to_string().contains("never-echo-this-secret"));
    let mut disabled = service(Some("secret-key"),false);
    disabled["enabled"] = json!(false);
    assert_eq!(call(&engine,"POST","/v1/admin/services",Some("admin-secret"),disabled).await.0,StatusCode::CREATED);
    assert_eq!(call(&engine,"GET","/v1/services",Some("runtime-secret"),json!(null)).await.1,json!([]));
    assert_eq!(call(&engine,"GET","/v1/admin/services",Some("admin-secret"),json!(null)).await.1.as_array().unwrap().len(),1);
}

#[tokio::test]
async fn query_limits_count_unicode_characters_and_json_requests_are_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    for (query,limit) in [(" ".to_owned(),5),("猫".repeat(129),5),("valid".to_owned(),0),("valid".to_owned(),11)] {
        let request = json!({"request_id":uuid::Uuid::new_v4().to_string(),"service_id":"bb2ad4e7-6168-4be2-a214-0177cb4320f0","query":query,"limit":limit});
        assert_eq!(call(&engine,"POST","/v1/search",Some("runtime-secret"),request).await.0,StatusCode::BAD_REQUEST);
    }
    let valid = json!({"request_id":uuid::Uuid::new_v4().to_string(),"service_id":"bb2ad4e7-6168-4be2-a214-0177cb4320f0","query":"猫".repeat(128)});
    assert_eq!(call(&engine,"POST","/v1/search",Some("runtime-secret"),valid).await.0,StatusCode::NOT_FOUND);
    let oversized = json!({"name":"x".repeat(20000),"kind":"deepseek","enabled":false,"auth_mode":"api_key"});
    assert_eq!(call(&engine,"POST","/v1/admin/services",Some("admin-secret"),oversized).await.0,StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn shared_subscription_is_caller_owned_and_cannot_store_an_engine_key() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let shared = json!({"name":"Site subscription","kind":"gpt","enabled":true,"auth_mode":"shared_chatgpt","model":"gpt-5.4"});
    let (status, created) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),shared.clone()).await;
    assert_eq!(status,StatusCode::CREATED);
    assert_eq!(created["has_api_key"],false);
    assert_eq!(created["connected"],false);
    let request = json!({"request_id":uuid::Uuid::new_v4().to_string(),"service_id":created["id"],"query":"public test","limit":1});
    assert_eq!(call(&engine,"POST","/v1/search",Some("runtime-secret"),request).await.0,StatusCode::CONFLICT);
    let mut keyed = shared.clone();
    keyed["api_key"] = json!("never-store-a-shared-subscription-key");
    assert_eq!(call(&engine,"POST","/v1/admin/services",Some("admin-secret"),keyed).await.0,StatusCode::BAD_REQUEST);
    let mut wrong_provider = shared;
    wrong_provider["kind"] = json!("deepseek");
    assert_eq!(call(&engine,"POST","/v1/admin/services",Some("admin-secret"),wrong_provider).await.0,StatusCode::BAD_REQUEST);
    assert!(!std::fs::read_to_string(directory.path().join("services.json")).unwrap().contains("never-store-a-shared-subscription-key"));
}

#[tokio::test]
async fn delegated_credentials_are_ephemeral_and_rejected_outside_shared_search() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let (_, key_service) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),service(Some("saved-engine-key"),false)).await;
    let shared = json!({"name":"Site subscription","kind":"gpt","enabled":true,"auth_mode":"shared_chatgpt"});
    let (_, shared_service) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),shared).await;
    for (service_id,credential,expected) in [
        (key_service["id"].as_str().unwrap(),"ephemeral-never-persist",StatusCode::BAD_REQUEST),
        (shared_service["id"].as_str().unwrap(),"",StatusCode::BAD_REQUEST),
        (shared_service["id"].as_str().unwrap(),"invalid token with spaces",StatusCode::BAD_REQUEST),
        (shared_service["id"].as_str().unwrap(),"ephemeral-never-persist",StatusCode::CONFLICT),
    ] {
        let id = uuid::Uuid::new_v4().to_string();
        call(&engine,"POST","/v1/cancel",Some("runtime-secret"),json!({"request_id":id})).await;
        let request = Request::builder().method("POST").uri("/v1/search")
            .header("authorization","Bearer runtime-secret").header("content-type","application/json")
            .header("x-nekochat-openai-access-token",credential)
            .body(Body::from(json!({"request_id":id,"service_id":service_id,"query":"test"}).to_string())).unwrap();
        let response = engine.router().oneshot(request).await.unwrap();
        assert_eq!(response.status(),expected);
        let bytes = to_bytes(response.into_body(),65536).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("ephemeral-never-persist"));
        if expected == StatusCode::CONFLICT {
            assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["error"],"request_cancelled");
        }
    }
    let persisted = std::fs::read_to_string(directory.path().join("services.json")).unwrap();
    assert!(!persisted.contains("ephemeral-never-persist"));
    let request = json!({"request_id":uuid::Uuid::new_v4().to_string(),"service_id":shared_service["id"],"query":"test","access_token":"never-accept-json-secret"});
    let (status,error) = call(&engine,"POST","/v1/search",Some("runtime-secret"),request).await;
    assert_eq!(status,StatusCode::BAD_REQUEST);
    assert!(!error.to_string().contains("never-accept-json-secret"));
}

#[tokio::test]
async fn switching_to_shared_mode_removes_the_old_saved_key_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let (_, created) = call(&engine,"POST","/v1/admin/services",Some("admin-secret"),service(Some("obsolete-saved-key"),false)).await;
    let path = format!("/v1/admin/services/{}",created["id"].as_str().unwrap());
    let shared = json!({"name":"Site subscription","kind":"gpt","enabled":true,"auth_mode":"shared_chatgpt","api_key":""});
    let (status,updated) = call(&engine,"PUT",&path,Some("admin-secret"),shared).await;
    assert_eq!(status,StatusCode::OK);
    assert_eq!(updated["has_api_key"],false);
    assert_eq!(updated["connected"],false);
    drop(engine);
    assert!(!std::fs::read_to_string(directory.path().join("services.json")).unwrap().contains("obsolete-saved-key"));
    let engine = Engine::open(directory.path(),"runtime-secret","admin-secret").unwrap();
    let (_, services) = call(&engine,"GET","/v1/services",Some("runtime-secret"),json!(null)).await;
    assert_eq!(services[0]["has_api_key"],false);
    assert_eq!(services[0]["connected"],false);
}
