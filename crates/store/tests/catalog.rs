use nekochat_core::{Price, Protocol, ReasoningSpec, ReasoningStyle};
use nekochat_store::{ModelSpec, SettingKey, Store, StoreError};

fn spec(provider: &str, upstream_id: &str) -> ModelSpec {
    ModelSpec {
        provider: provider.into(),
        upstream_id: upstream_id.into(),
        display_name: upstream_id.into(),
        sort: 0,
        protocol: Protocol::ChatCompletions,
        reasoning: ReasoningSpec {
            style: ReasoningStyle::DeepSeek,
            levels: vec!["none".into(), "low".into(), "high".into(), "max".into()],
            default: Some("high".into()),
        },
        vision: true,
        context_window: Some(1_000_000),
        price: Price {
            cached_in: Some(0.028),
            uncached_in: Some(0.28),
            output: Some(0.42),
        },
    }
}

fn fresh() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("sub/nekochat.db")).unwrap();
    (dir, store)
}

#[test]
fn reopen_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nekochat.db");
    let store = Store::open(&path).unwrap();
    store.create_character(&serde_json::from_value(serde_json::json!({"name":"Neko","display_name":"Neko","system_prompt":"You are a cat."})).unwrap()).unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    assert_eq!(store.list_characters().unwrap().len(), 1);
}

#[test]
fn models_listed_in_sort_order() {
    let (_d, store) = fresh();
    for (sort, name) in [(2, "B"), (1, "Z"), (1, "A")] {
        let mut s = spec("p", name);
        s.sort = sort;
        store.insert_model(&s).unwrap();
    }
    let names: Vec<String> = store
        .list_models()
        .unwrap()
        .into_iter()
        .map(|m| m.spec.display_name)
        .collect();
    assert_eq!(names, ["A", "Z", "B"]);
}

#[test]
fn duplicate_upstream_model_conflicts() {
    let (_d, store) = fresh();
    store
        .insert_model(&spec("deepseek", "deepseek-v4-pro"))
        .unwrap();
    assert!(matches!(
        store.insert_model(&spec("deepseek", "deepseek-v4-pro")),
        Err(StoreError::Conflict(_))
    ));
}

#[test]
fn model_spec_round_trips() {
    let (_d, store) = fresh();
    let s = spec("deepseek", "deepseek-v4-pro");
    let m = store.insert_model(&s).unwrap();
    let got = store.get_model(&m.id).unwrap().unwrap();
    assert_eq!(got.spec.reasoning.levels, ["none", "low", "high", "max"]);
    assert_eq!(got.spec.reasoning.default.as_deref(), Some("high"));
    assert_eq!(got.spec.price, s.price);
    assert_eq!(got.spec, s);
}

#[test]
fn deleting_default_model_clears_setting() {
    let (_d, store) = fresh();
    let m = store.insert_model(&spec("deepseek", "x")).unwrap();
    store
        .set_setting(SettingKey::DefaultModel, Some(&m.id))
        .unwrap();
    store.delete_model(&m.id).unwrap();
    assert_eq!(store.get_setting(SettingKey::DefaultModel).unwrap(), None);
}

#[test]
fn update_missing_character_is_not_found() {
    let (_d, store) = fresh();
    assert!(matches!(
        store.update_character(
            "nope",
            &serde_json::from_value(
                serde_json::json!({"name":"n","display_name":"n","system_prompt":"p"})
            )
            .unwrap()
        ),
        Err(StoreError::NotFound)
    ));
}
