use super::*;
use std::net::{IpAddr,Ipv4Addr,SocketAddr};

#[test]
fn destination_guard_rejects_mixed_public_private_dns_without_returning_an_address() {
    assert!(checked_addresses(vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::new(93,184,216,34)),0),SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST),0)]).is_err());
    let result = checked_addresses(vec!["93.184.216.34:0".parse().unwrap()]).unwrap();
    assert_eq!(result.collect::<Vec<_>>(),vec!["93.184.216.34:0".parse::<SocketAddr>().unwrap()]);
    assert!(checked_addresses(Vec::new()).is_err());
}
#[test]
fn page_reader_requires_public_http_default_ports() {
    for url in ["http://127.0.0.1/","https://[::1]/","http://169.254.169.254/","http://[::ffff:127.0.0.1]/","http://localhost/","http://example.com:8080/","https://example.com:80/","https://user:secret@example.com/","file:///etc/passwd","gopher://example.com/"] { assert!(page_url(url).is_err(),"{url}"); }
    assert!(page_url("https://example.com/article").is_ok());
}
#[test]
fn html_reader_ignores_executable_and_navigation_content() {
    let (title,text) = html_text("<html><head><title>Article title</title><style>secret style</style></head><body><nav>Navigation junk</nav><script>alert('secret')</script><main><h1>Heading</h1><p>Actual evidence &amp; text.</p><p hidden>Hidden junk</p></main></body></html>",&CancellationToken::new(),Instant::now()+PROCESSING_TIMEOUT).unwrap();
    assert_eq!(title,"Article title");
    assert!(text.contains("Actual evidence & text."));
    assert!(!text.contains("secret"));
    assert!(!text.contains("Navigation junk"));
    assert!(!text.contains("Hidden junk"));
}
#[tokio::test]
async fn redirect_and_pdf_responses_are_explicitly_rejected() {
    use axum::{routing::get,Router};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/redirect",get(move || async move { (axum::http::StatusCode::FOUND,[("location",format!("http://{address}/target"))],"") })).route("/target",get(|| async { "This target must not be read" })).route("/pdf",get(|| async { ([("content-type","application/pdf")],"not extracted") }));
    let server = tokio::spawn(async move { axum::serve(listener,router).await.unwrap(); });
    let client = Reader::new().unwrap().http;
    let redirect = client.get(format!("http://{address}/redirect")).send().await.unwrap();
    assert_eq!(read_response(redirect).await.unwrap_err().code(),"page_redirect_not_followed");
    let pdf = client.get(format!("http://{address}/pdf")).send().await.unwrap();
    assert_eq!(read_response(pdf).await.unwrap_err().code(),"page_content_type_unsupported");
    server.abort();
}

#[tokio::test]
async fn deeply_nested_document_is_rejected_before_unbounded_extraction() {
    use axum::{routing::get, Router};
    let mut html = "<div>".repeat(2048);
    html.push_str(&"<span> </span>".repeat(512));
    html.push_str("<p>Visible evidence</p>");
    html.push_str(&"</div>".repeat(2048));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new().route("/", get(move || {
        let html = html.clone();
        async move { ([("content-type", "text/html")], html) }
    }));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap(); });
    let response = reqwest::Client::new().get(format!("http://{address}/")).send().await.unwrap();
    let result = read_response(response).await;
    server.abort();
    assert_eq!(result.unwrap_err().code(), "page_structure_limit");
}

#[test]
fn parser_work_rejects_excessive_markup_and_cancelled_or_expired_jobs() {
    let token = CancellationToken::new();
    let excessive = "<div>".repeat(MARKUP_LIMIT + 1);
    assert_eq!(html_text(&excessive,&token,Instant::now()+PROCESSING_TIMEOUT).unwrap_err().code(),"page_markup_limit");
    token.cancel();
    assert_eq!(html_text("<p>Evidence</p>",&token,Instant::now()+PROCESSING_TIMEOUT).unwrap_err().code(),"request_cancelled");
    assert_eq!(html_text("<p>Evidence</p>",&CancellationToken::new(),Instant::now()-Duration::from_millis(1)).unwrap_err().code(),"page_processing_timeout");
}
