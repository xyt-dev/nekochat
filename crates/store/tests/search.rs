use nekochat_store::{CharacterSpec, SearchUsageRecord, Store};
use serde_json::json;

#[test]
fn legacy_character_defaults_and_search_binding_survive_reopen_and_clear() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.db");
    let store = Store::open(&path).unwrap();
    let mut spec: CharacterSpec = serde_json::from_value(json!({"name":"cat","display_name":"Cat"})).unwrap();
    assert_eq!(spec.search_service_id, None);
    let character = store.create_character(&spec).unwrap();
    spec.search_service_id = Some("0196f817-1eb0-7000-8000-000000000001".into());
    store.update_character(&character.id, &spec).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.get_character(&character.id).unwrap().unwrap().spec.search_service_id.as_deref(), Some("0196f817-1eb0-7000-8000-000000000001"));
    assert!(store.search_service_in_use("0196f817-1eb0-7000-8000-000000000001").unwrap());
    assert_eq!(store.list_characters().unwrap()[0].spec.search_service_id, spec.search_service_id);
    spec.search_service_id = None;
    store.update_character(&character.id, &spec).unwrap();
    assert_eq!(store.get_character(&character.id).unwrap().unwrap().spec.search_service_id, None);
    assert!(!store.search_service_in_use("0196f817-1eb0-7000-8000-000000000001").unwrap());
}

#[test]
fn search_usage_is_separate_bounded_by_dates_and_preserves_unknown_cost() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("search.db");
    let store = Store::open(&path).unwrap();
    for (id, at_ms, usage) in [("a", 10, None), ("b", 20, Some(json!({"requests":1,"cached_in":2,"uncached_in":3,"output":4,"reasoning":5,"cost":null}))), ("c", 30, Some(json!({"requests":1,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"cost":{"amount":0.01,"currency":"CNY"}})))] {
        store.insert_search_usage(&SearchUsageRecord { id: id.into(), at_ms, service_id: "service".into(), status: "complete".into(), usage }).unwrap();
    }
    drop(store);
    let store = Store::open(&path).unwrap();
    let records = store.list_search_usage(20, 30).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].id, "b");
    assert_eq!(records[0].usage.as_ref().unwrap()["cost"], serde_json::Value::Null);
    assert_eq!(records[1].usage.as_ref().unwrap()["cost"]["currency"], "CNY");
    assert!(store.list_search_usage(31, 40).unwrap().is_empty());
}

#[test]
fn search_usage_rejects_content_in_the_metadata_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("search.db")).unwrap();
    let record = SearchUsageRecord { id: "a".into(), at_ms: 1, service_id: "service".into(), status: "error".into(), usage: Some(json!({"query":"private question","sources":[{"url":"https://example.com"}]})) };
    assert!(store.insert_search_usage(&record).is_err());
    assert!(store.list_search_usage(0, 2).unwrap().is_empty());
}
