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
    let unknown = records.iter().find(|record| record.id == "b").unwrap();
    let known = records.iter().find(|record| record.id == "c").unwrap();
    assert_eq!(unknown.usage.as_ref().unwrap()["cost"], serde_json::Value::Null);
    assert_eq!(known.usage.as_ref().unwrap()["cost"]["currency"], "CNY");
    assert!(store.list_search_usage(31, 40).unwrap().is_empty());
}

#[test]
fn search_usage_rejects_content_in_the_metadata_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("search.db")).unwrap();
    for usage in [
        json!({"query":"private question","sources":[{"url":"https://example.com"}]}),
        json!({"requests":1,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"cost":null,"query":"private question"}),
        json!({"requests":1,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"cost":{"amount":0.1,"currency":"USD","evidence":"private evidence"}}),
    ] {
        let record = SearchUsageRecord { id: "a".into(), at_ms: 1, service_id: "service".into(), status: "error".into(), usage: Some(usage) };
        assert!(store.insert_search_usage(&record).is_err());
    }
    assert!(store.list_search_usage(0, 2).unwrap().is_empty());
}

#[test]
fn search_usage_returns_latest_one_hundred_attempts_with_stable_timestamp_ties() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("search.db")).unwrap();
    for index in (0..105).rev() {
        store.insert_search_usage(&SearchUsageRecord {
            id: format!("{index:03}"),
            at_ms: 1000 + index / 2,
            service_id: "service".into(),
            status: "complete".into(),
            usage: None,
        }).unwrap();
    }
    for (id, at_ms) in [("before", 999), ("after", 1053)] {
        store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "service".into(),
            status: "error".into(), usage: None,
        }).unwrap();
    }
    let records = store.list_search_usage(1000, 1052).unwrap();
    assert_eq!(records.len(), 100);
    assert_eq!(records.iter().map(|record| record.id.as_str()).collect::<Vec<_>>(),
        (5..105).rev().map(|index| format!("{index:03}")).collect::<Vec<_>>());
}

#[test]
fn search_dashboard_aggregates_every_attempt_but_only_returns_latest_hundred() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("search.db")).unwrap();
    for index in 0..101 {
        store.insert_search_usage(&SearchUsageRecord {
            id: format!("a-{index:03}"), at_ms: 1000 + index, service_id: "a".into(),
            status: "complete".into(),
            usage: Some(json!({"requests":2,"cached_in":1,"uncached_in":2,"output":3,"reasoning":4,"cost":{"amount":0.5,"currency":"CNY"}})),
        }).unwrap();
    }
    for (id, at_ms, status, usage) in [
        ("b-zero", 2000, "complete", Some(json!({"requests":3,"cached_in":2,"uncached_in":3,"output":4,"reasoning":5,"cost":{"amount":0.0,"currency":"USD"}}))),
        ("b-paid", 2001, "complete", Some(json!({"requests":4,"cached_in":3,"uncached_in":4,"output":5,"reasoning":6,"cost":{"amount":2.0,"currency":"USD"}}))),
        ("b-unknown", 2002, "complete", Some(json!({"requests":5,"cached_in":4,"uncached_in":5,"output":6,"reasoning":7,"cost":null}))),
        ("b-error", 2003, "error", None),
        ("b-cancel", 2004, "cancelled", None),
    ] {
        store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "b".into(), status: status.into(), usage,
        }).unwrap();
    }
    for (id, at_ms) in [("before", 999), ("after", 2005)] {
        store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "excluded".into(), status: "error".into(), usage: None,
        }).unwrap();
    }
    let dashboard = store.search_usage_dashboard(1000, 2004, 1000).unwrap();
    let wire = serde_json::to_value(&dashboard).unwrap();
    assert_eq!(wire["from_ms"], 1000);
    assert_eq!(wire["to_ms"], 2004);
    assert_eq!(wire["bucket_ms"], 1000);
    assert_eq!(wire["totals"], json!({
        "attempts":106,"requests":214,"cached_in":110,"uncached_in":214,"output":318,"reasoning":422,
        "known_costs":[{"currency":"CNY","amount":50.5},{"currency":"USD","amount":2.0}],"unknown_costs":3
    }));
    assert_eq!(wire["services"], json!([
        {"service_id":"a","totals":{"attempts":101,"requests":202,"cached_in":101,"uncached_in":202,"output":303,"reasoning":404,"known_costs":[{"currency":"CNY","amount":50.5}],"unknown_costs":0},
         "points":[{"at_ms":1000,"attempts":101,"requests":202,"cached_in":101,"uncached_in":202,"output":303,"reasoning":404}]},
        {"service_id":"b","totals":{"attempts":5,"requests":12,"cached_in":9,"uncached_in":12,"output":15,"reasoning":18,"known_costs":[{"currency":"USD","amount":2.0}],"unknown_costs":3},
         "points":[{"at_ms":2000,"attempts":5,"requests":12,"cached_in":9,"uncached_in":12,"output":15,"reasoning":18}]}
    ]));
    assert_eq!(dashboard.records.len(), 100);
    assert_eq!(dashboard.records.first().unwrap().id, "b-cancel");
    assert_eq!(dashboard.records.last().unwrap().id, "a-006");
    assert_eq!(dashboard.records, store.list_search_usage(1000, 2004).unwrap());
    assert_eq!(dashboard.records[0].usage, None);
    assert_eq!(dashboard.records[2].usage.as_ref().unwrap()["cost"], serde_json::Value::Null);
    let zero = store.search_usage_dashboard(2000, 2000, 1000).unwrap();
    assert_eq!(serde_json::to_value(zero.totals).unwrap()["known_costs"], json!([{"currency":"USD","amount":0.0}]));
    assert_eq!(zero.records.len(), 1);
}

#[test]
fn search_dashboard_has_sparse_ascending_epoch_buckets_and_inclusive_empty_ranges() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("search.db")).unwrap();
    for (id, at_ms) in [("negative", -1), ("start", 10), ("tie-a", 35), ("tie-b", 35), ("end", 40)] {
        store.insert_search_usage(&SearchUsageRecord {
            id: id.into(), at_ms, service_id: "deleted-service".into(), status: "error".into(), usage: None,
        }).unwrap();
    }
    let dashboard = store.search_usage_dashboard(-1, 40, 10).unwrap();
    assert_eq!(dashboard.services[0].service_id, "deleted-service");
    assert_eq!(dashboard.services[0].points.iter().map(|point| (point.at_ms, point.attempts)).collect::<Vec<_>>(),
        vec![(-10, 1), (10, 1), (30, 2), (40, 1)]);
    assert_eq!(dashboard.records.iter().map(|record| record.id.as_str()).collect::<Vec<_>>(),
        vec!["end", "tie-b", "tie-a", "start", "negative"]);
    let single = store.search_usage_dashboard(35, 35, 10).unwrap();
    assert_eq!(single.totals.attempts, 2);
    assert_eq!(single.services[0].points[0].at_ms, 30);
    let empty = store.search_usage_dashboard(11, 34, 10).unwrap();
    assert_eq!(serde_json::to_value(empty).unwrap(), json!({
        "from_ms":11,"to_ms":34,"bucket_ms":10,
        "totals":{"attempts":0,"requests":0,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"known_costs":[],"unknown_costs":0},
        "services":[],"records":[]
    }));
    for (from, to, bucket) in [(0, 1, 0), (0, 1, -1), (2, 1, 1)] {
        assert!(store.search_usage_dashboard(from, to, bucket).is_err());
    }
    store.insert_search_usage(&SearchUsageRecord {
        id: "minimum".into(), at_ms: i64::MIN, service_id: "service".into(), status: "error".into(), usage: None,
    }).unwrap();
    assert!(store.search_usage_dashboard(i64::MIN, i64::MIN, 3).is_err());
}

#[test]
fn search_dashboard_returns_errors_instead_of_wrapping_counters_or_costs() {
    for usage in [
        json!({"requests":1,"cached_in":u64::MAX,"uncached_in":0,"output":0,"reasoning":0,"cost":null}),
        json!({"requests":1,"cached_in":0,"uncached_in":0,"output":0,"reasoning":0,"cost":{"amount":f64::MAX,"currency":"USD"}}),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("search.db")).unwrap();
        for id in ["a", "b"] {
            store.insert_search_usage(&SearchUsageRecord {
                id: id.into(), at_ms: 0, service_id: "service".into(), status: "complete".into(), usage: Some(usage.clone()),
            }).unwrap();
        }
        assert!(store.search_usage_dashboard(0, 0, 1).is_err());
    }
}
