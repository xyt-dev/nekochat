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

#[test]
fn stats_preserve_billing_identity_unknown_prices_and_epoch_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("stats.db")).unwrap();
    let mut old = record(59_999);
    old.cost_usd = None;
    old.usage.upstream_cost_usd = None;
    old.model_label = "old label".into();
    store.insert_usage(&old).unwrap();
    let mut zero = record(60_000);
    zero.cost_usd = Some(0.0);
    zero.usage.upstream_cost_usd = None;
    zero.model_label = "latest label".into();
    store.insert_usage(&zero).unwrap();
    let mut other = record(60_001);
    other.provider = "another provider".into();
    other.cost_usd = None;
    other.usage.upstream_cost_usd = None;
    store.insert_usage(&other).unwrap();
    let mut boundary = zero.clone();
    boundary.at_ms = 120_000;
    store.insert_usage(&boundary).unwrap();
    let models = store.usage_stats(59_999, 120_000, 60_000).unwrap();
    assert_eq!(models.len(), 2);
    let model = models.iter().find(|m| m.provider == "deepseek").unwrap();
    assert_eq!(model.model_label, "latest label");
    assert_eq!(
        model.points.iter().map(|p| p.at_ms).collect::<Vec<_>>(),
        [0, 60_000]
    );
    assert_eq!(model.totals.requests, 2);
    assert_eq!(model.totals.cost_usd, Some(0.0));
    assert_eq!(model.totals.unpriced_requests, 1);
    assert_eq!((model.totals.reasoning, model.totals.output), (6, 8));
    assert_eq!(model.points[0].totals.cost_usd, None);
    assert_eq!(
        models
            .iter()
            .find(|m| m.provider == other.provider)
            .unwrap()
            .totals
            .cost_usd,
        None
    );
    assert!(
        store
            .usage_stats(120_001, 180_000, 60_000)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn stats_prefer_historical_upstream_charge_including_free_over_estimated_price() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("charges.db")).unwrap();
    let mut paid = record(1);
    paid.cost_usd = Some(99.0);
    paid.usage.upstream_cost_usd = Some(0.25);
    store.insert_usage(&paid).unwrap();
    let mut free = paid.clone();
    free.at_ms = 2;
    free.usage.upstream_cost_usd = Some(0.0);
    store.insert_usage(&free).unwrap();
    let models = store.usage_stats(0, 3, 60_000).unwrap();
    assert_eq!(models[0].totals.cost_usd, Some(0.25));
    assert_eq!(models[0].totals.unpriced_requests, 0);
}

#[test]
fn rolling_allowance_uses_half_open_windows_and_retains_missing_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("allowance.db")).unwrap();
    let now = 40 * 86_400_000;
    for (at, amount) in [
        (now - 30 * 86_400_000 - 1, Some(100.0)),
        (now - 30 * 86_400_000, Some(1.0)),
        (now - 7 * 86_400_000, Some(2.0)),
        (now - 5 * 3_600_000, Some(0.0)),
        (now - 1, None),
        (now, Some(100.0)),
    ] {
        let mut rec = record(at);
        rec.usage.allowance_usd = amount;
        store.insert_usage(&rec).unwrap();
    }
    let models = store.allowance_windows(now).unwrap();
    assert_eq!(models.len(), 1);
    let windows = &models[0].windows;
    assert_eq!(windows[0].used_usd, Some(0.0));
    assert_eq!(windows[1].used_usd, Some(2.0));
    assert_eq!(windows[2].used_usd, Some(3.0));
    assert!(windows.iter().all(|w| w.unknown_requests == 1));
    let mut missing = record(now - 1);
    missing.upstream_id = "missing".into();
    store.insert_usage(&missing).unwrap();
    let models = store.allowance_windows(now).unwrap();
    let missing = models.iter().find(|m| m.upstream_id == "missing").unwrap();
    assert!(
        missing
            .windows
            .iter()
            .all(|w| w.used_usd.is_none() && w.unknown_requests == 1)
    );
}
