use nekochat_store::{Node, NodeRole, NodeStatus, Store, StoreError};

fn fresh() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("nekochat.db")).unwrap();
    (dir, store)
}

fn node(conv: &str, id: &str, parent: Option<&str>, role: NodeRole, at: i64) -> Node {
    Node {
        id: id.into(),
        conversation_id: conv.into(),
        parent_id: parent.map(Into::into),
        role,
        content: format!("content {id}"),
        reasoning: None,
        model_id: None,
        model_label: None,
        effort: None,
        status: NodeStatus::Complete,
        metrics: None,
        created_at: at,
    }
}

/// Inserts u1 → a1 → u2 → a2 and returns the conversation id.
fn chain(store: &Store) -> String {
    let c = store.create_conversation(None).unwrap().id;
    store
        .insert_node(&node(&c, "u1", None, NodeRole::User, 1))
        .unwrap();
    store
        .insert_node(&node(&c, "a1", Some("u1"), NodeRole::Assistant, 2))
        .unwrap();
    store
        .insert_node(&node(&c, "u2", Some("a1"), NodeRole::User, 3))
        .unwrap();
    store
        .insert_node(&node(&c, "a2", Some("u2"), NodeRole::Assistant, 4))
        .unwrap();
    c
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn new_conversation_defaults() {
    let (_d, store) = fresh();
    let c = store.create_conversation(None).unwrap();
    assert_eq!(c.title, "新对话");
    assert_eq!(c.leaf_id, None);
    assert_eq!(
        uuid::Uuid::parse_str(&c.session_key)
            .unwrap()
            .get_version_num(),
        4
    );
    assert_eq!(store.get_conversation(&c.id).unwrap(), Some(c.clone()));
    store.set_title(&c.id, "猫").unwrap();
    assert_eq!(store.list_conversations().unwrap()[0].title, "猫");
}

#[test]
fn path_runs_root_to_leaf() {
    let (_d, store) = fresh();
    let c = chain(&store);
    let ids: Vec<_> = store
        .path_to(&c, Some("a2"))
        .unwrap()
        .into_iter()
        .map(|n| n.id)
        .collect();
    assert_eq!(ids, ["u1", "a1", "u2", "a2"]);
    assert!(store.path_to(&c, None).unwrap().is_empty());
    let leaf = store.get_conversation(&c).unwrap().unwrap().leaf_id;
    assert_eq!(leaf.as_deref(), Some("a2"));
}

#[test]
fn regenerate_creates_sibling_and_moves_leaf() {
    let (_d, store) = fresh();
    let c = store.create_conversation(None).unwrap().id;
    store
        .insert_node(&node(&c, "u1", None, NodeRole::User, 1))
        .unwrap();
    store
        .insert_node(&node(&c, "a1", Some("u1"), NodeRole::Assistant, 2))
        .unwrap();
    store
        .insert_node(&node(&c, "a1b", Some("u1"), NodeRole::Assistant, 3))
        .unwrap();
    let conv = store.get_conversation(&c).unwrap().unwrap();
    assert_eq!(conv.leaf_id.as_deref(), Some("a1b"));
    let nodes = store.nodes(&c).unwrap();
    assert_eq!(
        nodes
            .iter()
            .filter(|n| n.parent_id.as_deref() == Some("u1"))
            .count(),
        2
    );
    assert_eq!(
        nodes.iter().map(|n| n.created_at).collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn newest_leaf_under_follows_latest_children() {
    let (_d, store) = fresh();
    let c = chain(&store);
    store
        .insert_node(&node(&c, "a1b", Some("u1"), NodeRole::Assistant, 5))
        .unwrap();
    assert_eq!(store.newest_leaf_under(&c, "u1").unwrap(), "a1b");
    assert_eq!(store.newest_leaf_under(&c, "a1").unwrap(), "a2");
}

#[test]
fn set_leaf_rejects_foreign_node() {
    let (_d, store) = fresh();
    let a = store.create_conversation(None).unwrap().id;
    let b = store.create_conversation(None).unwrap().id;
    store
        .insert_node(&node(&b, "nb", None, NodeRole::User, 1))
        .unwrap();
    assert!(matches!(
        store.set_leaf(&a, Some("nb")),
        Err(StoreError::NotFound)
    ));
    store.set_leaf(&b, None).unwrap();
    assert_eq!(store.get_conversation(&b).unwrap().unwrap().leaf_id, None);
    store.set_leaf(&b, Some("nb")).unwrap();
    let leaf = store.get_conversation(&b).unwrap().unwrap().leaf_id;
    assert_eq!(leaf.as_deref(), Some("nb"));
}

#[test]
fn insert_with_parent_from_other_conversation_conflicts() {
    let (_d, store) = fresh();
    let a = store.create_conversation(None).unwrap().id;
    let b = store.create_conversation(None).unwrap().id;
    store
        .insert_node(&node(&b, "nb", None, NodeRole::User, 1))
        .unwrap();
    let r = store.insert_node(&node(&a, "na", Some("nb"), NodeRole::Assistant, 2));
    assert!(matches!(r, Err(StoreError::Conflict(_))));
    let r = store.insert_node(&node("missing", "nx", None, NodeRole::User, 2));
    assert!(matches!(r, Err(StoreError::NotFound)));
}

#[test]
fn clear_keeps_conversation_and_removes_nodes() {
    let (_d, store) = fresh();
    let c = chain(&store);
    store.set_title(&c, "保留").unwrap();
    store.clear_conversation(&c).unwrap();
    let conv = store.get_conversation(&c).unwrap().unwrap();
    assert_eq!(conv.leaf_id, None);
    assert_eq!(conv.title, nekochat_store::DEFAULT_TITLE);
    assert!(store.nodes(&c).unwrap().is_empty());
}

#[test]
fn delete_leaves_no_content_bytes() {
    let (dir, store) = fresh();
    let marker = format!("MARKER-{}", uuid::Uuid::new_v4());
    let c = store.create_conversation(None).unwrap().id;
    let mut u = node(&c, "u1", None, NodeRole::User, 1);
    u.content = marker.clone();
    store.insert_node(&u).unwrap();
    let mut a = node(&c, "a1", Some("u1"), NodeRole::Assistant, 2);
    a.reasoning = Some(marker.clone());
    store.insert_node(&a).unwrap();
    store.delete_conversation(&c).unwrap();
    assert_eq!(store.get_conversation(&c).unwrap(), None);
    for f in ["nekochat.db", "nekochat.db-wal"] {
        let p = dir.path().join(f);
        if p.exists() {
            assert!(
                !contains(&std::fs::read(&p).unwrap(), marker.as_bytes()),
                "{f}"
            );
        }
    }
}

/// Inserts a single chain of `len` nodes and returns the conversation id.
fn deep_chain(store: &Store, len: usize) -> String {
    let c = store.create_conversation(None).unwrap().id;
    let mut parent: Option<String> = None;
    for i in 0..len {
        let id = format!("n{i}");
        let role = if i % 2 == 0 {
            NodeRole::User
        } else {
            NodeRole::Assistant
        };
        store
            .insert_node(&node(&c, &id, parent.as_deref(), role, i as i64))
            .unwrap();
        parent = Some(id);
    }
    c
}

fn foreign_key_violations(dir: &tempfile::TempDir) -> usize {
    let conn = rusqlite::Connection::open(dir.path().join("nekochat.db")).unwrap();
    let mut stmt = conn.prepare("PRAGMA foreign_key_check").unwrap();
    stmt.query_map([], |_| Ok(())).unwrap().count()
}

#[test]
fn delete_handles_deep_chain() {
    let (dir, store) = fresh();
    let c = deep_chain(&store, 1100);
    store.delete_conversation(&c).unwrap();
    assert_eq!(store.get_conversation(&c).unwrap(), None);
    assert!(store.nodes(&c).unwrap().is_empty());
    assert_eq!(foreign_key_violations(&dir), 0);
}

#[test]
fn clear_handles_deep_chain() {
    let (dir, store) = fresh();
    let c = deep_chain(&store, 1100);
    store.clear_conversation(&c).unwrap();
    assert!(store.get_conversation(&c).unwrap().is_some());
    assert!(store.nodes(&c).unwrap().is_empty());
    assert_eq!(foreign_key_violations(&dir), 0);
}

/// Two chained conversations whose padded nodes are inserted alternately (a pattern that left
/// deleted text in a live page before VACUUM); returns (kept, doomed).
fn interleaved(store: &Store, keep_tag: &str, mul: usize, modulo: usize) -> (String, String) {
    let keep = store.create_conversation(None).unwrap().id;
    let doomed = store.create_conversation(None).unwrap().id;
    let mut parents: [Option<String>; 2] = [None, None];
    for i in 0..200usize {
        let pad = "x".repeat((i * mul) % modulo);
        for (k, (conv, tag)) in [(&keep, keep_tag), (&doomed, "DELME")]
            .into_iter()
            .enumerate()
        {
            let id = nekochat_store::new_id();
            let mut n = node(conv, &id, parents[k].as_deref(), NodeRole::User, 0);
            n.content = format!("{tag}{i:04}{pad}");
            n.created_at = nekochat_store::now_ms();
            store.insert_node(&n).unwrap();
            parents[k] = Some(id);
        }
    }
    (keep, doomed)
}

fn assert_no_doomed_bytes(dir: &tempfile::TempDir) {
    for f in ["nekochat.db", "nekochat.db-wal"] {
        let p = dir.path().join(f);
        if p.exists() {
            assert!(!contains(&std::fs::read(&p).unwrap(), b"DELME"), "{f}");
        }
    }
}

#[test]
fn delete_leaves_no_bytes_in_live_pages() {
    for (keep_tag, mul, modulo) in [("KEEPME", 1700, 6000), ("KEEPS", 997, 4500)] {
        let (dir, store) = fresh();
        let (keep, doomed) = interleaved(&store, keep_tag, mul, modulo);
        store.delete_conversation(&doomed).unwrap();
        assert_eq!(store.nodes(&keep).unwrap().len(), 200);
        drop(store);
        assert_no_doomed_bytes(&dir);
    }
}

#[test]
fn clear_leaves_no_content_bytes() {
    for (keep_tag, mul, modulo) in [("KEEPME", 1700, 6000), ("KEEPS", 997, 4500)] {
        let (dir, store) = fresh();
        let (keep, doomed) = interleaved(&store, keep_tag, mul, modulo);
        store.clear_conversation(&doomed).unwrap();
        assert_eq!(store.nodes(&keep).unwrap().len(), 200);
        assert!(store.nodes(&doomed).unwrap().is_empty());
        drop(store);
        assert_no_doomed_bytes(&dir);
    }
}

#[test]
fn newer_schema_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nekochat.db");
    drop(Store::open(&path).unwrap());
    rusqlite::Connection::open(&path)
        .unwrap()
        .pragma_update(None, "user_version", 99)
        .unwrap();
    let err = Store::open(&path).err().expect("must refuse");
    let msg = err.to_string();
    assert!(msg.contains("99") && msg.contains('1'), "{msg}");
    let version: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn fresh_database_gets_schema_version() {
    let (dir, store) = fresh();
    drop(store);
    let version: i64 = rusqlite::Connection::open(dir.path().join("nekochat.db"))
        .unwrap()
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 1);
}

#[test]
fn deleting_character_detaches_conversations() {
    let (_d, store) = fresh();
    let ch = store.create_character("Neko", "You are a cat.").unwrap();
    let c = store.create_conversation(Some(&ch.id)).unwrap();
    assert_eq!(c.character_id.as_deref(), Some(ch.id.as_str()));
    store.delete_character(&ch.id).unwrap();
    let conv = store.get_conversation(&c.id).unwrap().unwrap();
    assert_eq!(conv.character_id, None);
}
