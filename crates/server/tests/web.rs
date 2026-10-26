mod common;

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use common::{TestCfg, spawn_app};
use nekochat_server::web::WebAssets;
use serde_json::{Value, json};

struct Files(HashMap<&'static str, &'static str>);

impl WebAssets for Files {
    fn get(&self, path: &str) -> Option<Cow<'static, [u8]>> {
        self.0.get(path).map(|s| Cow::Borrowed(s.as_bytes()))
    }
}

const INDEX: &str = "<!doctype html><title>nekochat</title>";

fn dist() -> Arc<dyn WebAssets> {
    Arc::new(Files(HashMap::from([
        ("index.html", INDEX),
        ("assets/app-1234.js", "console.log(1)"),
        ("favicon.svg", "<svg/>"),
    ])))
}

async fn get(app: &common::TestApp, path: &str) -> reqwest::Response {
    app.client.get(app.url(path)).send().await.unwrap()
}

fn header<'a>(r: &'a reqwest::Response, name: &str) -> &'a str {
    r.headers().get(name).unwrap().to_str().unwrap()
}

#[tokio::test]
async fn root_and_unknown_paths_serve_index() {
    let app = spawn_app(TestCfg {
        web_assets: Some(dist()),
        access_password: Some("pw".into()),
        ..Default::default()
    })
    .await;
    for path in [
        "/",
        "/c/some-conversation",
        "/index.html",
        "/assets/missing.js",
    ] {
        let r = get(&app, path).await;
        assert_eq!(r.status(), 200, "{path}");
        assert!(
            header(&r, "content-type").starts_with("text/html"),
            "{path}"
        );
        assert_eq!(header(&r, "cache-control"), "no-cache", "{path}");
        assert_eq!(r.text().await.unwrap(), INDEX, "{path}");
    }
    let r = get(&app, "/favicon.svg").await;
    assert_eq!(r.status(), 200);
    assert_eq!(header(&r, "content-type"), "image/svg+xml");
    // The access password guards the API only.
    assert_eq!(get(&app, "/api/conversations").await.status(), 401);
    let r = get(&app, "/api/nope").await;
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn hashed_assets_are_immutable() {
    let app = spawn_app(TestCfg {
        web_assets: Some(dist()),
        ..Default::default()
    })
    .await;
    let r = get(&app, "/assets/app-1234.js").await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        header(&r, "cache-control"),
        "public, max-age=31536000, immutable"
    );
    assert!(header(&r, "content-type").contains("javascript"));
    assert_eq!(r.text().await.unwrap(), "console.log(1)");
    // Unknown API paths stay JSON 404s.
    let r = get(&app, "/api/nope").await;
    assert_eq!(r.status(), 404);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"error": "接口不存在"})
    );
}

#[tokio::test]
async fn missing_frontend_is_json_404() {
    let app = spawn_app(TestCfg {
        web_assets: Some(Arc::new(Files(HashMap::new()))),
        ..Default::default()
    })
    .await;
    let r = get(&app, "/").await;
    assert_eq!(r.status(), 404);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"error": "前端未构建：先运行 web 构建"})
    );
}

#[tokio::test]
async fn themes_are_listed_and_served() {
    let app = spawn_app(TestCfg {
        access_password: Some("pw".into()),
        ..Default::default()
    })
    .await;
    assert_eq!(get(&app, "/api/themes").await.status(), 401);
    let r = app
        .client
        .post(app.url("/api/session"))
        .json(&json!({"password": "pw"}))
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success());

    // No themes directory yet.
    let r = get(&app, "/api/themes").await;
    assert_eq!(r.json::<Value>().await.unwrap(), json!([]));

    let dir = app.dir.path().join("themes");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("zeta.css"), ":root{--x:1}").unwrap();
    std::fs::write(dir.join("Alpha_1-b.css"), ":root{--y:2}").unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    std::fs::write(dir.join("bad name.css"), "x").unwrap();
    std::fs::create_dir(dir.join("sub.css")).unwrap();
    let r = get(&app, "/api/themes").await;
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!([
            {"id": "Alpha_1-b", "name": "Alpha_1-b", "url": "/themes/Alpha_1-b.css"},
            {"id": "zeta", "name": "zeta", "url": "/themes/zeta.css"},
        ])
    );

    let r = get(&app, "/themes/zeta.css").await;
    assert_eq!(r.status(), 200);
    assert!(header(&r, "content-type").starts_with("text/css"));
    assert_eq!(r.text().await.unwrap(), ":root{--x:1}");
}

#[tokio::test]
async fn theme_paths_cannot_escape() {
    let app = spawn_app(TestCfg {
        web_assets: Some(dist()),
        ..Default::default()
    })
    .await;
    let dir = app.dir.path().join("themes");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(app.dir.path().join("secret.css"), "secret").unwrap();
    std::fs::write(dir.join("notes.txt"), "x").unwrap();
    for path in [
        "/themes/..%2Fsecret.css",
        "/themes/..%2F..%2Fetc%2Fpasswd",
        "/themes/%2E%2E%2Fsecret.css",
        "/themes/..%5Csecret.css",
        "/themes/notes.txt",
        "/themes/missing.css",
        "/themes/sub/x.css",
    ] {
        let r = get(&app, path).await;
        assert_eq!(r.status(), 404, "{path}");
        let v: Value = r.json().await.unwrap();
        assert!(v["error"].is_string(), "{path}");
    }
}
