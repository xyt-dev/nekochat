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
        (Some("test-password-alpha".into()), PasswordStorage::Plain)
    );
    store.set_password_storage(PasswordStorage::Plain).unwrap();
    assert_eq!(
        store.user_password(&a.id).unwrap(),
        (Some("test-password-alpha".into()), PasswordStorage::Plain)
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
fn credentials_database_sidecars_are_private_and_new_hash_accounts_store_no_plaintext() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("accounts.db");
    let store = Store::open(&path).unwrap();
    let password = "unique-fixture-password-hash-marker";
    store.set_password_storage(PasswordStorage::Hash).unwrap();
    let user = store.create_user("private-user", password).unwrap();
    assert_eq!(
        store.user_password(&user.id).unwrap(),
        (None, PasswordStorage::Hash)
    );
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

#[test]
fn new_account_password_policy_preserves_existing_credentials_and_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("accounts.db")).unwrap();
    let plain = store
        .create_user("existing-plain", "existing-test-password")
        .unwrap();
    store
        .create_user_session(
            &plain.id,
            plain.credential_revision,
            "existing-policy-session",
            i64::MAX,
        )
        .unwrap();
    store.set_password_storage(PasswordStorage::Hash).unwrap();
    assert_eq!(
        store.user_password(&plain.id).unwrap(),
        (
            Some("existing-test-password".into()),
            PasswordStorage::Plain
        )
    );
    let authenticated = store
        .authenticate_user("existing-plain", "existing-test-password")
        .unwrap()
        .unwrap();
    assert_eq!(authenticated.password_storage, PasswordStorage::Plain);
    assert_eq!(authenticated.credential_revision, plain.credential_revision);
    assert!(
        store
            .user_for_session("existing-policy-session")
            .unwrap()
            .is_some()
    );
    let reset_plain = store
        .update_user(
            &plain.id,
            Some(plain.credential_revision),
            None,
            Some("changed-plain-password"),
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        store.user_password(&plain.id).unwrap(),
        (
            Some("changed-plain-password".into()),
            PasswordStorage::Plain
        )
    );
    assert_eq!(
        reset_plain.credential_revision,
        plain.credential_revision + 1
    );
    assert!(
        store
            .user_for_session("existing-policy-session")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("existing-plain", "existing-test-password")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("existing-plain", "changed-plain-password")
            .unwrap()
            .is_some()
    );
    let hashed = store
        .create_user("new-hashed", "hashed-test-password")
        .unwrap();
    assert_eq!(
        store.user_password(&hashed.id).unwrap(),
        (None, PasswordStorage::Hash)
    );
    store
        .create_user_session(
            &hashed.id,
            hashed.credential_revision,
            "hashed-policy-session",
            i64::MAX,
        )
        .unwrap();
    store.set_password_storage(PasswordStorage::Plain).unwrap();
    let authenticated = store
        .authenticate_user("new-hashed", "hashed-test-password")
        .unwrap()
        .unwrap();
    assert_eq!(authenticated.password_storage, PasswordStorage::Hash);
    assert_eq!(
        authenticated.credential_revision,
        hashed.credential_revision
    );
    assert!(
        store
            .user_for_session("hashed-policy-session")
            .unwrap()
            .is_some()
    );
    let reset_hashed = store
        .update_user(
            &hashed.id,
            Some(hashed.credential_revision),
            None,
            Some("changed-hashed-password"),
            None,
            None,
        )
        .unwrap();
    assert_eq!(
        store.user_password(&hashed.id).unwrap(),
        (None, PasswordStorage::Hash)
    );
    assert_eq!(
        reset_hashed.credential_revision,
        hashed.credential_revision + 1
    );
    assert!(
        store
            .user_for_session("hashed-policy-session")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("new-hashed", "hashed-test-password")
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .authenticate_user("new-hashed", "changed-hashed-password")
            .unwrap()
            .is_some()
    );
    let later = store
        .create_user("later-plain", "later-test-password")
        .unwrap();
    assert_eq!(
        store.user_password(&later.id).unwrap(),
        (Some("later-test-password".into()), PasswordStorage::Plain)
    );
}
