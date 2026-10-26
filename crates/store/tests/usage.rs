use nekochat_core::Usage;
use nekochat_store::{Store, UsagePurpose, UsageRecord};

fn record(at_ms: i64) -> UsageRecord {
    UsageRecord {
        at_ms,
        provider: "deepseek".into(),
        upstream_id: "deepseek-chat".into(),
        model_label: "DeepSeek".into(),
        usage: Usage {
            cached_in: 1,
            uncached_in: 2,
            reasoning: 3,
            output: 4,
            upstream_cost_usd: Some(0.5),
            allowance_usd: None,
        },
        cost_usd: Some(0.25),
        purpose: UsagePurpose::Chat,
    }
}

#[test]
fn usage_query_is_half_open_by_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("nekochat.db")).unwrap();
    for at in [300, 100, 200] {
        store.insert_usage(&record(at)).unwrap();
    }
    let got = store.usage_between(100, 300).unwrap();
    assert_eq!(got.iter().map(|r| r.at_ms).collect::<Vec<_>>(), [100, 200]);
    assert_eq!(got[0], record(100));
}
