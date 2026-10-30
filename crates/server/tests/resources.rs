mod common;
use common::{TestCfg, spawn_app};
use serde_json::{Value, json};

#[tokio::test]
async fn asset_routes_are_access_gated_and_background_upload_is_admin_only() {
    let app = spawn_app(TestCfg {
        access_password: Some("access".into()),
        admin_password: Some("admin".into()),
        ..Default::default()
    })
    .await;
    for path in [
        "/api/resources/nekowhale/unknown.gif",
        "/api/backgrounds/00000000-0000-4000-8000-000000000000.gif",
    ] {
        assert_eq!(
            app.client.get(app.url(path)).send().await.unwrap().status(),
            401
        );
    }
    assert_eq!(
        app.client
            .post(app.url("/api/admin/backgrounds"))
            .header("content-type", "image/gif")
            .body("GIF89a")
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    app.login_admin("admin").await;
    for (mime, bytes) in [
        ("image/svg+xml", b"<svg/>".as_slice()),
        ("image/png", b"GIF89a".as_slice()),
        ("text/html", b"<!doctype html>".as_slice()),
    ] {
        assert_eq!(
            app.client
                .post(app.url("/api/admin/backgrounds"))
                .header("content-type", mime)
                .body(bytes.to_vec())
                .send()
                .await
                .unwrap()
                .status(),
            400
        );
    }
    assert_eq!(
        app.client
            .post(app.url("/api/admin/backgrounds"))
            .header("content-type", "image/gif")
            .body(vec![0; 5 * 1024 * 1024 + 1])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );
    let response = app
        .client
        .post(app.url("/api/admin/backgrounds"))
        .header("content-type", "image/avif")
        .body(b"\0\0\0\x18ftypavif\0\0\0\0mif1miaf".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let uploaded: Value = response.json().await.unwrap();
    let url = uploaded["url"].as_str().unwrap();
    assert!(url.starts_with("/api/backgrounds/"));
    assert_eq!(
        app.client.get(app.url(url)).send().await.unwrap().status(),
        401
    );
    assert_eq!(
        app.client
            .post(app.url("/api/session"))
            .json(&json!({"password":"access"}))
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    let response = app.client.get(app.url(url)).send().await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "image/avif");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    for invalid in [
        "https://evil/image.png",
        "//evil/image.png",
        "/api/resources/nekowhale/../asset.gif",
        "/api/backgrounds/missing.gif",
    ] {
        let response = app.client.post(app.url("/api/admin/characters")).json(&json!({"name":"role","display_name":"Role","background":{"url":invalid,"opacity":0.15}})).send().await.unwrap();
        assert_eq!(response.status(), 400, "{invalid}");
    }
    let response = app
        .client
        .post(app.url("/api/admin/characters"))
        .json(&json!({"name":"role","display_name":"Role","background":{"url":url}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let role: Value = response.json().await.unwrap();
    assert_eq!(role["background"]["opacity"], 0.15);
    assert_eq!(role["background"]["position"], "center");
    let id = role["id"].as_str().unwrap();
    let response = app.client.put(app.url(&format!("/api/admin/characters/{id}")))
        .json(&json!({"name":"role","display_name":"Role","background":{"url":url,"opacity":0.25,"position":"right"}}))
        .send().await.unwrap();
    assert_eq!(response.status(), 200);
    let roles: Vec<Value> = app.client.get(app.url("/api/admin/characters")).send().await.unwrap().json().await.unwrap();
    let saved = roles.iter().find(|item| item["id"] == id).unwrap();
    assert_eq!(saved["background"]["position"], "right");
    assert_eq!(saved["background"]["opacity"], 0.25);
    assert_eq!(saved["background"]["url"], url);
    let rejected = app.client.put(app.url(&format!("/api/admin/characters/{id}")))
        .json(&json!({"name":"role","display_name":"Role","background":{"url":url,"position":"left"}}))
        .send().await.unwrap();
    assert_eq!(rejected.status(), 400);
    let roles: Vec<Value> = app.client.get(app.url("/api/admin/characters")).send().await.unwrap().json().await.unwrap();
    assert_eq!(roles.iter().find(|item| item["id"] == id).unwrap()["background"]["position"], "right");
}

#[cfg(unix)]
#[tokio::test]
async fn background_file_and_directory_symlinks_cannot_escape() {
    use std::os::unix::fs::symlink;
    let app = spawn_app(TestCfg::default()).await;
    let outside = tempfile::tempdir().unwrap();
    let file = "00000000-0000-4000-8000-000000000000.gif";
    std::fs::write(outside.path().join(file), b"GIF89a").unwrap();
    std::fs::create_dir(app.dir.path().join("backgrounds")).unwrap();
    symlink(
        outside.path().join(file),
        app.dir.path().join("backgrounds").join(file),
    )
    .unwrap();
    assert_eq!(
        app.client
            .get(app.url(&format!("/api/backgrounds/{file}")))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    std::fs::remove_file(app.dir.path().join("backgrounds").join(file)).unwrap();
    std::fs::remove_dir(app.dir.path().join("backgrounds")).unwrap();
    symlink(outside.path(), app.dir.path().join("backgrounds")).unwrap();
    assert_eq!(
        app.client
            .get(app.url(&format!("/api/backgrounds/{file}")))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
}
