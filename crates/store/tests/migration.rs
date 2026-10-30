use nekochat_store::{NodeStatus, Store};

#[test]
fn schema_one_migrates_without_losing_tree_usage_or_private_model_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(include_str!("../src/schema.sql"))
        .unwrap();
    conn.execute_batch(r#"
        INSERT INTO characters VALUES ('role','Old role','SECRET',1,2);
        INSERT INTO models VALUES ('model','provider','upstream','Old Model',0,'chat_completions','{"style":"none","levels":[],"default":null}',0,NULL,'{"cached_in":1,"uncached_in":2,"output":3}');
        INSERT INTO settings VALUES ('default_model_id','model');
        INSERT INTO settings VALUES ('default_character_id','role');
        INSERT INTO conversations VALUES ('conv','Old title','role','assistant','session',1,2);
        INSERT INTO nodes VALUES ('user','conv',NULL,'user','hello',NULL,NULL,NULL,NULL,'complete',NULL,1);
        INSERT INTO nodes VALUES ('assistant','conv','user','assistant','old reply','private thought','model','Old Model','high','complete',NULL,2);
        INSERT INTO usage VALUES (1,2,'provider','upstream','Old Model',1,2,3,4,0.5,NULL,NULL,'chat');
        PRAGMA user_version=1;
    "#).unwrap();
    drop(conn);
    let store = Store::open(&path).unwrap();
    let role = store.get_character("role").unwrap().unwrap();
    assert_eq!(role.spec.display_name, "Old role");
    assert_eq!(role.spec.model_id.as_deref(), Some("model"));
    assert_eq!(role.spec.system_prompt, "SECRET");
    assert!(role.spec.search_service_id.is_none());
    assert!(role.spec.background.is_none());
    assert!(role.spec.resource_packs.is_empty());
    let nodes = store.nodes("conv").unwrap();
    assert_eq!(nodes.len(), 2);
    let reply = nodes.iter().find(|node| node.id == "assistant").unwrap();
    assert_eq!(reply.content, "old reply");
    assert_eq!(reply.reasoning.as_deref(), Some("private thought"));
    assert_eq!(reply.model_id.as_deref(), Some("model"));
    assert_eq!(reply.status, NodeStatus::Complete);
    assert_eq!(reply.character_name, None);
    assert_eq!(reply.character_avatar, None);
    let user = store
        .create_user("migration-user", "migration-test-password")
        .unwrap();
    assert!(store.list_user_conversations(&user.id).unwrap().is_empty());
    assert!(store.require_conversation_owner("conv", &user.id).is_err());
    assert_eq!(store.legacy_conversation_count().unwrap(), 1);
    assert_eq!(
        store
            .get_conversation("conv")
            .unwrap()
            .unwrap()
            .leaf_id
            .as_deref(),
        Some("assistant")
    );
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row("SELECT count(*) FROM usage", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        6
    );
    drop(conn);
    assert_eq!(Store::open(&path).unwrap().nodes("conv").unwrap().len(), 2);
}
