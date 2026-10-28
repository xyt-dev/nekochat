use nekochat_store::{PasswordStorage, Store, StoreError};

#[test]
fn account_password_modes_sessions_and_legacy_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("accounts.db")).unwrap();
    let legacy = store.create_conversation(None).unwrap();
    let a = store.create_user("alpha", "test-password-alpha").unwrap();
    let b = store.create_user("beta", "test-password-beta").unwrap();
    assert!(matches!(
        store.create_user("ALPHA", "duplicate-test-password"),
        Err(StoreError::Conflict(_))
    ));
    assert_eq!(store.password_storage().unwrap(), PasswordStorage::Plain);
    assert_eq!(
        store.user_password(&a.id).unwrap().0.as_deref(),
        Some("test-password-alpha")
    );
    assert!(matches!(
        store.require_conversation_owner(&legacy.id, &a.id),
        Err(StoreError::NotFound)
    ));
    assert_eq!(
        store
            .assign_legacy_conversations(&a.id, Some(&[legacy.id.clone()]))
            .unwrap(),
        1
    );
    store.require_conversation_owner(&legacy.id, &a.id).unwrap();
    assert!(store.require_conversation_owner(&legacy.id, &b.id).is_err());
    assert_eq!(store.list_user_conversations(&b.id).unwrap().len(), 0);
    store
        .create_user_session(&a.id, a.credential_revision, "opaque-fingerprint", i64::MAX)
        .unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("accounts.db")).unwrap();
    assert_eq!(
        store
            .user_for_session("opaque-fingerprint")
            .unwrap()
            .unwrap()
            .id,
        a.id
    );
    store.set_password_storage(PasswordStorage::Hash).unwrap();
    assert_eq!(
        store.user_password(&a.id).unwrap(),
        (None, PasswordStorage::Hash)
    );
    store.set_password_storage(PasswordStorage::Plain).unwrap();
    assert_eq!(
        store.user_password(&a.id).unwrap(),
        (None, PasswordStorage::Hash)
    );
    assert!(store.authenticate_user("alpha", "wrong").unwrap().is_none());
    assert!(
        store
            .authenticate_user("alpha", "test-password-alpha")
            .unwrap()
            .is_some()
    );
    assert_eq!(
        store.user_password(&a.id).unwrap().1,
        PasswordStorage::Plain
    );
    store
        .update_user(&a.id, None, None, None, None, Some(true))
        .unwrap();
    assert!(
        store
            .user_for_session("opaque-fingerprint")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("alpha", "test-password-alpha")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("beta", "test-password-beta")
            .unwrap()
            .is_some()
    );
}

#[test]
fn preferences_reject_cross_user_conversations_and_clear_deleted_references() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("accounts.db")).unwrap();
    let a = store.create_user("alpha", "test-password-alpha").unwrap();
    let b = store.create_user("beta", "test-password-beta").unwrap();
    let conv = store.create_owned_conversation(&a.id, None).unwrap();
    assert!(
        store
            .set_user_preferences(&b.id, None, Some(&conv.id))
            .is_err()
    );
    store
        .set_user_preferences(&a.id, None, Some(&conv.id))
        .unwrap();
    let avatar = store
        .insert_avatar("image/png", b"test-local-avatar")
        .unwrap();
    let role = store
        .create_character(
            &serde_json::from_value(serde_json::json!({
                "name":"role", "display_name":"Role", "avatar":avatar
            }))
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        store.get_character(&role.id).unwrap().unwrap().spec.avatar,
        Some(avatar)
    );
    store
        .set_user_preferences(&a.id, Some(&role.id), Some(&conv.id))
        .unwrap();
    store.delete_character(&role.id).unwrap();
    assert!(
        store
            .get_user(&a.id)
            .unwrap()
            .unwrap()
            .last_character_id
            .is_none()
    );
    store.delete_conversation(&conv.id).unwrap();
    assert!(
        store
            .get_user(&a.id)
            .unwrap()
            .unwrap()
            .last_conversation_id
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn credentials_database_sidecars_are_private_and_hash_conversion_scrubs_plaintext() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.db");
    let store = Store::open(&path).unwrap();
    let password = "unique-fixture-password-migration-marker";
    store.create_user("private-user", password).unwrap();
    store.set_password_storage(PasswordStorage::Hash).unwrap();
    for name in ["accounts.db", "accounts.db-wal", "accounts.db-shm"] {
        let file = dir.path().join(name);
        if file.exists() {
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert!(
                !std::fs::read(file)
                    .unwrap()
                    .windows(password.len())
                    .any(|bytes| bytes == password.as_bytes())
            );
        }
    }
}

#[test]
fn password_reset_rejects_a_session_from_prior_authentication() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("accounts.db")).unwrap();
    store
        .create_user("race-user", "previous-test-password")
        .unwrap();
    let authenticated = store
        .authenticate_user("race-user", "previous-test-password")
        .unwrap()
        .unwrap();
    store
        .update_user(
            &authenticated.id,
            None,
            None,
            Some("replacement-test-password"),
            None,
            None,
        )
        .unwrap();
    assert!(
        store
            .create_user_session(
                &authenticated.id,
                authenticated.credential_revision,
                "stale-authentication",
                i64::MAX
            )
            .is_err()
    );
    assert!(
        store
            .user_for_session("stale-authentication")
            .unwrap()
            .is_none()
    );
    let current = store
        .authenticate_user("race-user", "replacement-test-password")
        .unwrap()
        .unwrap();
    store
        .create_user_session(
            &current.id,
            current.credential_revision,
            "current-authentication",
            i64::MAX,
        )
        .unwrap();
    assert_eq!(
        store
            .user_for_session("current-authentication")
            .unwrap()
            .unwrap()
            .id,
        current.id
    );
    store
        .update_user(&current.id, None, None, None, None, Some(true))
        .unwrap();
    store
        .update_user(&current.id, None, None, None, None, Some(false))
        .unwrap();
    assert!(
        store
            .create_user_session(
                &current.id,
                current.credential_revision,
                "pre-disable-authentication",
                i64::MAX
            )
            .is_err()
    );
}

#[test]
fn password_reset_rejects_a_profile_update_verified_against_the_old_password() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("accounts.db")).unwrap();
    let user = store
        .create_user("profile-race", "previous-test-password")
        .unwrap();
    let revision = store
        .verified_password_revision(&user.id, "previous-test-password")
        .unwrap()
        .unwrap();
    store
        .update_user(
            &user.id,
            None,
            None,
            Some("admin-replacement-password"),
            None,
            None,
        )
        .unwrap();
    assert!(
        store
            .update_user(
                &user.id,
                Some(revision),
                None,
                Some("stale-profile-password"),
                None,
                None
            )
            .is_err()
    );
    assert!(
        store
            .authenticate_user("profile-race", "stale-profile-password")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("profile-race", "admin-replacement-password")
            .unwrap()
            .is_some()
    );
}
