mod common;
use common::{TestApp, TestCfg, spawn_app};
use reqwest::Client;
use serde_json::{Value, json};

fn client() -> Client {
    Client::builder().cookie_store(true).build().unwrap()
}
async fn register(app: &TestApp, client: &Client, name: &str) -> Value {
    let response = client
        .post(app.url("/api/account/register"))
        .json(&json!({"username":name,"password":"account-test-password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response.headers()["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    let user: Value = response.json().await.unwrap();
    assert!(user.get("password").is_none());
    user
}
async fn login(app: &TestApp, client: &Client, name: &str, password: &str) -> reqwest::Response {
    client
        .post(app.url("/api/account/login"))
        .json(&json!({"username":name,"password":password}))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn every_conversation_and_attachment_id_boundary_is_user_scoped() {
    let app = spawn_app(TestCfg {
        anonymous: true,
        admin_password: Some("admin-test-only".into()),
        ..Default::default()
    })
    .await;
    let alice = client();
    let bob = client();
    let anonymous = client();
    let a = register(&app, &alice, "alice").await;
    let b = register(&app, &bob, "bob").await;
    let conv = app
        .store
        .create_owned_conversation(a["id"].as_str().unwrap(), None)
        .unwrap();
    let url = format!("/api/conversations/{}", conv.id);
    let upload = alice
        .post(app.url(&format!("{url}/attachments")))
        .header("content-type", "text/plain")
        .header("x-file-name", "notes.txt")
        .body("private document context")
        .send()
        .await
        .unwrap();
    assert_eq!(upload.status(), 201);
    let attachment: Value = upload.json().await.unwrap();
    let attachment_url = format!("{url}/attachments/{}", attachment["id"].as_str().unwrap());
    for client in [&bob, &anonymous] {
        let expected = if std::ptr::eq(client, &bob) { 404 } else { 401 };
        for path in [&url, &attachment_url, &format!("{attachment_url}/metadata")] {
            assert_eq!(
                client.get(app.url(path)).send().await.unwrap().status(),
                expected
            );
        }
        assert_eq!(
            client.delete(app.url(&url)).send().await.unwrap().status(),
            expected
        );
        assert_eq!(
            client
                .post(app.url(&format!("{url}/clear")))
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
        assert_eq!(
            client
                .put(app.url(&format!("{url}/leaf")))
                .json(&json!({"node_id":"not-a-node","descend":false}))
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
        assert_eq!(
            client
                .post(app.url(&format!("{url}/attachments")))
                .header("x-file-name", "private.txt")
                .body("steal")
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
        assert_eq!(
            client
                .post(app.url(&format!("{url}/turns")))
                .json(&json!({"action":"send","character_id":"unavailable-role","content":"steal"}))
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    let bob_conv = app
        .store
        .create_owned_conversation(b["id"].as_str().unwrap(), None)
        .unwrap();
    let swapped = format!(
        "/api/conversations/{}/attachments/{}",
        bob_conv.id,
        attachment["id"].as_str().unwrap()
    );
    assert_eq!(
        bob.get(app.url(&swapped)).send().await.unwrap().status(),
        404
    );
    assert_eq!(
        bob.get(app.url(&format!("{swapped}/metadata")))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    app.store.delete_conversation(&bob_conv.id).unwrap();
    let listed: Value = bob
        .get(app.url("/api/conversations"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed, json!([]));
    assert_eq!(
        bob.put(app.url("/api/account/preferences"))
            .json(&json!({"last_character_id":null,"last_conversation_id":conv.id}))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(
        alice
            .get(app.url(&attachment_url))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .as_ref(),
        b"private document context"
    );
    assert_eq!(
        alice
            .get(app.url("/api/admin/users"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn plain_hash_plain_conversion_is_explicit_and_relogin_restores_preferences() {
    let app = spawn_app(TestCfg {
        anonymous: true,
        admin_password: Some("admin-test-only".into()),
        upstream: Some("http://127.0.0.1:9".into()),
        ..Default::default()
    })
    .await;
    app.login_admin("admin-test-only").await;
    let user_client = client();
    let user = register(&app, &user_client, "mode-user").await;
    let id = user["id"].as_str().unwrap();
    let model=app.store.insert_model(&serde_json::from_value(json!({"provider":"mock","upstream_id":"test","display_name":"test","sort":0,"protocol":"chat_completions","reasoning":{"style":"none","levels":[],"default":null},"vision":false,"context_window":null,"price":{"cached_in":null,"uncached_in":null,"output":null}})).unwrap()).unwrap();
    let role = app
        .store
        .create_character(
            &serde_json::from_value(
                json!({"name":"role","display_name":"role","model_id":model.id}),
            )
            .unwrap(),
        )
        .unwrap();
    let conv = app
        .store
        .create_owned_conversation(id, Some(&role.id))
        .unwrap();
    assert_eq!(
        user_client
            .put(app.url("/api/account/preferences"))
            .json(&json!({"last_character_id":"missing","last_conversation_id":null}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        user_client
            .put(app.url("/api/account/preferences"))
            .json(&json!({"last_character_id":role.id,"last_conversation_id":conv.id}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    let password_url = app.url(&format!("/api/admin/users/{id}/password"));
    let revealed: Value = app
        .client
        .post(&password_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(revealed["password"], "account-test-password");
    for mode in ["hash", "plain"] {
        assert_eq!(
            app.client
                .put(app.url("/api/admin/user-settings"))
                .json(&json!({"password_storage":mode}))
                .send()
                .await
                .unwrap()
                .status(),
            200
        );
    }
    let hidden: Value = app
        .client
        .post(&password_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(hidden["password"].is_null());
    assert_eq!(hidden["password_storage"], "hash");
    let (logout_guard, logout_cancel) = app.state.begin_turn(&conv.id).unwrap();
    assert_eq!(
        user_client
            .delete(app.url("/api/account/session"))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert!(logout_cancel.is_cancelled());
    drop(logout_guard);
    assert_eq!(
        user_client
            .get(app.url("/api/conversations"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        login(&app, &user_client, "mode-user", "wrong-password")
            .await
            .status(),
        401
    );
    let restored: Value = login(&app, &user_client, "mode-user", "account-test-password")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(restored["last_character_id"], role.id);
    assert_eq!(restored["last_conversation_id"], conv.id);
    let revealed: Value = app
        .client
        .post(&password_url)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(revealed["password_storage"], "plain");
    assert_eq!(revealed["password"], "account-test-password");
    let (_guard, cancellation) = app.state.begin_turn(&conv.id).unwrap();
    assert_eq!(
        app.client
            .patch(app.url(&format!("/api/admin/users/{id}")))
            .json(&json!({"disabled":true}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(cancellation.is_cancelled());
    let session: Value = user_client
        .get(app.url("/api/account/session"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(session.is_null());
    assert_eq!(
        login(&app, &user_client, "mode-user", "account-test-password")
            .await
            .status(),
        401
    );
    assert_eq!(
        app.client
            .patch(app.url(&format!("/api/admin/users/{id}")))
            .json(&json!({"disabled":false,"password":"reset-test-password"}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        login(&app, &user_client, "mode-user", "reset-test-password")
            .await
            .status(),
        200
    );
    assert_eq!(
        app.client
            .delete(app.url(&format!("/api/admin/users/{id}")))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert!(app.store.get_conversation(&conv.id).unwrap().is_none());
    assert_eq!(
        user_client
            .get(app.url("/api/conversations"))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn avatars_profiles_legacy_assignment_and_origin_wall_are_real() {
    let app = spawn_app(TestCfg {
        anonymous: true,
        admin_password: Some("admin-test-only".into()),
        ..Default::default()
    })
    .await;
    app.login_admin("admin-test-only").await;
    let legacy = app.store.create_conversation(None).unwrap();
    let created = app
        .client
        .post(app.url("/api/admin/users"))
        .json(&json!({"username":"managed-user","password":"managed-test-password"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), 201);
    let managed: Value = created.json().await.unwrap();
    assert!(managed.get("password").is_none());
    assert_eq!(managed["password_storage"], "plain");
    let users: Value = app
        .client
        .get(app.url("/api/admin/users"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        users["users"]
            .as_array()
            .unwrap()
            .iter()
            .all(|u| u.get("password").is_none())
    );
    let user_client = client();
    let user = register(&app, &user_client, "avatar-user").await;
    let id = user["id"].as_str().unwrap();
    assert_eq!(
        user_client
            .get(app.url(&format!("/api/conversations/{}", legacy.id)))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    let allocated: Value = app
        .client
        .post(app.url(&format!("/api/admin/users/{id}/legacy-conversations")))
        .json(&json!({"conversation_ids":[legacy.id]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(allocated["assigned"], 1);
    assert_eq!(
        user_client
            .get(app.url(&format!("/api/conversations/{}", legacy.id)))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    use base64::Engine;
    let png=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a/YQAAAAASUVORK5CYII=").unwrap();
    let uploaded = user_client
        .post(app.url("/api/account/avatar"))
        .header("content-type", "image/png")
        .body(png.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(uploaded.status(), 200);
    let avatar: Value = uploaded.json().await.unwrap();
    let url = avatar["url"].as_str().unwrap();
    let updated: Value = user_client
        .patch(app.url("/api/account"))
        .json(&json!({"username":"new-name","avatar":url}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["avatar"], url);
    assert_eq!(updated["username"], "new-name");
    assert_eq!(
        app.client.get(app.url(url)).send().await.unwrap().status(),
        200
    );
    assert_eq!(
        client().get(app.url(url)).send().await.unwrap().status(),
        401
    );
    let admin_avatar: Value = app
        .client
        .post(app.url("/api/admin/avatars"))
        .body(png.clone())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let role:Value=app.client.post(app.url("/api/admin/characters")).json(&json!({"name":"avatar-role","display_name":"Avatar role","avatar":admin_avatar["url"]})).send().await.unwrap().json().await.unwrap();
    assert_eq!(role["avatar"], admin_avatar["url"]);
    assert_eq!(app.client.post(app.url("/api/admin/characters")).json(&json!({"name":"external-avatar","display_name":"External","avatar":"https://evil.example/a.png"})).send().await.unwrap().status(),400);
    assert_eq!(
        user_client
            .patch(app.url("/api/account"))
            .json(&json!({"avatar":"https://evil.example/avatar.png"}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        user_client
            .post(app.url("/api/account/avatar"))
            .body("not an image")
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert_eq!(
        user_client
            .post(app.url("/api/account/avatar"))
            .body(vec![0; 2 * 1024 * 1024 + 1])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    let mut foreign = reqwest::Url::parse(&app.base).unwrap();
    foreign
        .set_port(Some(if foreign.port() == Some(12345) {
            12346
        } else {
            12345
        }))
        .unwrap();
    assert_eq!(
        user_client
            .patch(app.url("/api/account"))
            .header("origin", foreign.as_str().trim_end_matches('/'))
            .json(&json!({"username":"csrf"}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        user_client
            .patch(app.url("/api/account"))
            .header("origin", &app.base)
            .json(&json!({"avatar":null}))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        user_client
            .patch(app.url("/api/account"))
            .json(&json!({"password":"new-test-password","current_password":"wrong-password"}))
            .send()
            .await
            .unwrap()
            .status(),
        400
    );
    assert!(
        user_client
            .get(app.url("/api/account/session"))
            .send()
            .await
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["id"]
            .is_string()
    );
    assert_eq!(
        user_client
            .patch(app.url("/api/account"))
            .json(
                &json!({"password":"new-test-password","current_password":"account-test-password"})
            )
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        login(&app, &client(), "new-name", "new-test-password")
            .await
            .status(),
        200
    );
}
