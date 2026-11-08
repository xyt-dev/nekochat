use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use axum::{Router, body::Body, http::{HeaderMap, StatusCode}, response::{IntoResponse, Redirect}, routing::get};
use futures_util::stream;
use nekochat_core::ReasoningStyle;
use nekochat_gateway::{GatewayError, ModelCatalogEntry, Provider, list_models};
use serde_json::{Value, json};

fn provider(base: &str) -> Provider {
    Provider {
        key: "custom".into(), label: "Custom".into(), base_url: format!("{base}/prefix/v1/"),
        models_url: None, api_key: "private-key".into(), session_header: None,
    }
}

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

async fn catalog(value: Value) -> Vec<ModelCatalogEntry> {
    let base = serve(Router::new().route("/prefix/v1/models", get(move || async move { axum::Json(value) }))).await;
    list_models(&reqwest::Client::new(), &provider(&base)).await.unwrap()
}

#[tokio::test]
async fn map_aliases_win_and_standard_data_has_priority() {
    let rows = catalog(json!({"models": {
        "relay-alias": {"id":"canonical", "display_name":"Label"},
        "": {"id":"fallback"}, "metadata": 9, "array": [], "null": null
    }})).await;
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().any(|row| row.id == "relay-alias" && row.name.as_deref() == Some("Label")));
    assert!(rows.iter().any(|row| row.id == "fallback"));
    let rows = catalog(json!({"data":[{"id":"served"}], "models":{"ignored":{}}})).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "served");
    assert!(catalog(json!({"data":[],"models":{"ignored":{}}})).await.is_empty());
}

#[tokio::test]
async fn malformed_rows_are_skipped_and_first_duplicate_wins() {
    let rows = catalog(json!({"data":[null, 1, [], {}, {"id":""}, {"id":"   "}, {"id":3},
        {"id":"valid","name":"first"}, {"id":"valid","name":"second"}, {"id":"other"}]})).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name.as_deref(), Some("first"));
    assert_eq!(rows[1].id, "other");
    assert!(rows[1].name.is_none());
}

#[tokio::test]
async fn capacities_follow_disclosed_precedence_without_name_guesses() {
    let rows = catalog(json!({"data":[
        {"id":"first","contextWindow":1234,"context_window":5678,"limit":{"context":9000}},
        {"id":"fallback","contextWindow":-1,"context_window":1.5,"context_length":0,"max_input_tokens":4096.0},
        {"id":"nested","limit":{"context":8192}},
        {"id":"gpt-vision-reasoner-1m","context_window":"1000000"},
        {"id":"too-large","context_window":9223372036854775808u64}
    ]})).await;
    assert_eq!(rows[0].context_window, Some(1234));
    assert_eq!(rows[1].context_window, Some(4096));
    assert_eq!(rows[2].context_window, Some(8192));
    for row in &rows[3..] {
        assert!(row.context_window.is_none());
        assert!(row.vision.is_none());
        assert!(row.reasoning.is_none());
    }
}

#[tokio::test]
async fn modalities_preserve_explicit_false_and_unknown() {
    let rows = catalog(json!({"data":[
        {"id":"explicit","vision":false,"architecture":{"input_modalities":["text","image"]}},
        {"id":"image","architecture":{"input_modalities":["text","image"]}},
        {"id":"text","modalities":{"input":["text"]}},
        {"id":"empty","input_modalities":[]},
        {"id":"malformed","input_modalities":[1,null]},
        {"id":"output-only","modalities":{"output":["image"]}}
    ]})).await;
    assert_eq!(rows[0].vision, Some(false));
    assert_eq!(rows[1].vision, Some(true));
    assert_eq!(rows[2].vision, Some(false));
    for row in &rows[3..] { assert!(row.vision.is_none()); }
}

#[tokio::test]
async fn reasoning_uses_disclosed_wire_style_and_validates_default() {
    let rows = catalog(json!({"data":[
        {"id":"effort","reasoning":{"style":"effort","levels":["low","high","high",1,""],"default":"high"}},
        {"id":"invalid-default","reasoning":{"style":"deepseek","levels":["low"],"default":"max"}},
        {"id":"disabled","reasoning":false},
        {"id":"unsupported","reasoning":{"style":"budget","levels":["low"]}},
        {"id":"boolean-only","reasoning":true},
        {"id":"parameter","supported_parameters":["reasoning_effort"]},
        {"id":"none","reasoning":{"style":"none","levels":["high"],"default":"high"}}
    ]})).await;
    let effort = rows[0].reasoning.as_ref().unwrap();
    assert_eq!(effort.style, ReasoningStyle::Effort);
    assert_eq!(effort.levels, ["low", "high"]);
    assert_eq!(effort.default.as_deref(), Some("high"));
    assert_eq!(rows[1].reasoning.as_ref().unwrap().style, ReasoningStyle::DeepSeek);
    assert!(rows[1].reasoning.as_ref().unwrap().default.is_none());
    for index in [2, 6] {
        let reasoning = rows[index].reasoning.as_ref().unwrap();
        assert_eq!(reasoning.style, ReasoningStyle::None);
        assert!(reasoning.levels.is_empty());
        assert!(reasoning.default.is_none());
    }
    assert!(rows[3].reasoning.is_none());
    assert!(rows[4].reasoning.is_none());
    assert!(rows[5].reasoning.is_none());
}

#[tokio::test]
async fn recognized_usd_prices_convert_units_and_preserve_zero_and_missing() {
    let rows = catalog(json!({"data":[
        {"id":"router","pricing":{"prompt":"0.000002","completion":"0.000008","input_cache_read":"0"}},
        {"id":"litellm","input_cost_per_token":0.000003,"output_cost_per_token":0.000009,"cache_read_input_token_cost":0.0000005},
        {"id":"directory","limit":{"context":8192},"cost":{"input":2.5,"output":10,"cache_read":0}},
        {"id":"partial","pricing":{"prompt":"0"}},
        {"id":"info","model_info":{"input_cost_per_token":0.000001}}
    ]})).await;
    assert_eq!(rows[0].price.uncached_in, Some(2.0));
    assert_eq!(rows[0].price.output, Some(8.0));
    assert_eq!(rows[0].price.cached_in, Some(0.0));
    assert_eq!(rows[1].price.uncached_in, Some(3.0));
    assert_eq!(rows[1].price.output, Some(9.0));
    assert_eq!(rows[1].price.cached_in, Some(0.5));
    assert_eq!(rows[2].price.uncached_in, Some(2.5));
    assert_eq!(rows[2].price.output, Some(10.0));
    assert_eq!(rows[2].price.cached_in, Some(0.0));
    assert_eq!(rows[3].price.uncached_in, Some(0.0));
    assert!(rows[3].price.output.is_none());
    assert!(rows[3].price.cached_in.is_none());
    assert_eq!(rows[4].price.uncached_in, Some(1.0));
    for row in rows { assert!(row.price_source.unwrap().ends_with("/prefix/v1/models")); }
}

#[tokio::test]
async fn ambiguous_non_usd_negative_nonfinite_and_unknown_unit_prices_stay_unknown() {
    let rows = catalog(json!({"data":[
        {"id":"ambiguous","cost":{"input":2,"output":4}},
        {"id":"currency","pricing":{"currency":"EUR","prompt":"0.1"}},
        {"id":"root-currency","currency":"CNY","input_cost_per_token":0.1},
        {"id":"unit","pricing":{"unit":"credits","prompt":"0.1"}},
        {"id":"bad","pricing":{"prompt":"-1","completion":"NaN","input_cache_read":"Infinity"}},
        {"id":"overflow","pricing":{"prompt":"1e308"}},
        {"id":"directory-currency","limit":{"context":1024},"cost":{"currency":"JPY","input":1}},
        {"id":"directory-unit","limit":{"context":1024},"cost":{"unit":"token","input":1}}
    ]})).await;
    for row in rows {
        assert!(row.price.uncached_in.is_none(), "{}", row.id);
        assert!(row.price.output.is_none(), "{}", row.id);
        assert!(row.price.cached_in.is_none(), "{}", row.id);
        assert!(row.price_source.is_none(), "{}", row.id);
    }
}

#[tokio::test]
async fn explicit_models_url_preserves_query_and_auth_but_sanitizes_price_source() {
    let base = serve(Router::new().route("/custom", get(|headers: HeaderMap, uri: axum::http::Uri| async move {
        assert!(headers["user-agent"].to_str().unwrap().starts_with("nekochat/"));
        assert_eq!(headers["authorization"], "Bearer private-key");
        assert_eq!(uri.query(), Some("api_key=secret"));
        axum::Json(json!({"data":[{"id":"m","pricing":{"prompt":"0"}}]}))
    }))).await;
    let mut p = provider("http://127.0.0.1:1");
    p.models_url = Some(format!("{}/custom?api_key=secret#fragment", base.replace("http://", "http://user:password@")));
    let rows = list_models(&reqwest::Client::new(), &p).await.unwrap();
    assert_eq!(rows[0].price_source.as_deref(), Some(format!("{base}/custom").as_str()));
}

#[tokio::test]
async fn redirects_do_not_forward_credentials_even_on_same_origin() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let base = serve(Router::new()
        .route("/prefix/v1/models", get(|| async { Redirect::temporary("/target?secret=private-key") }))
        .route("/target", get(move || { let observed = observed.clone(); async move {
            observed.fetch_add(1, Ordering::SeqCst);
            axum::Json(json!({"data":[]}))
        }}))).await;
    let error = list_models(&reqwest::Client::new(), &provider(&base)).await.unwrap_err();
    assert!(matches!(error, GatewayError::Http { status: 307, .. }));
    assert_eq!(hits.load(Ordering::SeqCst), 0);
    assert!(!error.to_string().contains("private-key"));
}

#[tokio::test]
async fn errors_never_echo_upstream_body_or_endpoint_secrets() {
    let base = serve(Router::new()
        .route("/failure", get(|| async { (StatusCode::UNAUTHORIZED, "private-key raw secret body") }))
        .route("/invalid", get(|| async { "private-key invalid JSON" }))
        .route("/shape", get(|| async { axum::Json(json!({"error":"private-key"})) }))).await;
    for path in ["failure", "invalid", "shape"] {
        let mut p = provider(&base);
        p.models_url = Some(format!("{base}/{path}?api_key=private-key"));
        let error = list_models(&reqwest::Client::new(), &p).await.unwrap_err().to_string();
        assert!(!error.contains("private-key"));
        assert!(!error.contains("raw secret body"));
    }
    let mut p = provider("http://127.0.0.1:1");
    p.models_url = Some("http://user:password@127.0.0.1:1/models?secret=private-key".into());
    let error = list_models(&reqwest::Client::new(), &p).await.unwrap_err().to_string();
    assert!(!error.contains("private-key"));
    assert!(!error.contains("password"));
}

#[tokio::test]
async fn declared_and_streamed_oversized_catalogs_are_rejected() {
    let base = serve(Router::new()
        .route("/declared", get(|| async { ([("content-length", "2097153")], Body::from(vec![b' '; 2097153])).into_response() }))
        .route("/streamed", get(|| async {
            let chunks = stream::iter((0..33).map(|_| Ok::<_, std::io::Error>(vec![b' '; 65536])));
            Body::from_stream(chunks).into_response()
        }))).await;
    for path in ["declared", "streamed"] {
        let mut p = provider(&base);
        p.models_url = Some(format!("{base}/{path}"));
        let error = list_models(&reqwest::Client::new(), &p).await.unwrap_err();
        assert!(matches!(error, GatewayError::Protocol(message) if message.contains("too large")));
    }
}

#[tokio::test]
async fn nested_provider_rates_capabilities_and_variable_prices_are_preserved() {
    let rows = catalog(json!({"data":[
        {"id":"fixed","limit":{"context":1000000},"modalities":{"input":["text","image/png"]},
         "capabilities":{"reasoning":{"supported":true,"effort":["low","high","max"]}},
         "cost":{"text":{"input":{"price":1.4,"cache":{"read":0.26}},"output":{"price":4.4}}}},
        {"id":"tiered","limit":{"context":1000000},"cost":{"text":{"input":{"price":0.1,"overrides":[{"when":{"context":{"gt":272000}},"price":0.2}]},"output":{"price":0.5}}}},
        {"id":"cache-write","limit":{"context":1000000},"cost":{"text":{"input":{"price":2,"cache":{"read":0.25,"write":2.5}},"output":{"price":6}}}},
        {"id":"deepseek","effort":{"supported_levels":["low","high","max"],"default_level":"high"}}
    ]})).await;
    assert_eq!(rows[0].price.uncached_in, Some(1.4));
    assert_eq!(rows[0].price.cached_in, Some(0.26));
    assert_eq!(rows[0].price.output, Some(4.4));
    assert_eq!(rows[0].vision, Some(true));
    assert_eq!(rows[0].reasoning.as_ref().unwrap().levels, ["low","high","max"]);
    for row in &rows[1..3] {
        assert!(row.price.uncached_in.is_none());
        assert!(row.price.output.is_none());
        assert!(row.price_source.is_none());
    }
    assert_eq!(rows[3].reasoning.as_ref().unwrap().levels, ["low","high","max"]);
    assert_eq!(rows[3].reasoning.as_ref().unwrap().default.as_deref(), Some("high"));
}
