use super::*;
#[test]
fn rejected_oversized_snapshot_preserves_previous_durable_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let mut vault = Vault::open(directory.path()).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let service = Service { id:id.clone(),name:"Original".into(),kind:ServiceKind::Gpt,enabled:true,auth_mode:AuthMode::ApiKey,model:None,api_key:Some("original-key".into()) };
    vault.replace(service.clone()).unwrap();
    let mut oversized = service;
    oversized.api_key = Some("x".repeat(2 * 1024 * 1024));
    assert!(vault.replace(oversized).is_err());
    assert_eq!(vault.services[&id].api_key.as_deref(),Some("original-key"));
    drop(vault);
    let restored = Vault::open(directory.path()).unwrap();
    assert_eq!(restored.services[&id].api_key.as_deref(),Some("original-key"));
}
