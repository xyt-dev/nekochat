use crate::{client::bounded_body,validation::{public_ip,public_url},FetchResponse,SearchError,SearchSource,SearchUsage};
use reqwest::{dns::{Addrs,Name,Resolve,Resolving},Client,Url};
use scraper::{Html,Node,Selector};
use std::{net::SocketAddr,sync::{Arc,LazyLock},time::{Duration,Instant}};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

const TEXT_LIMIT: usize = 24_000;
const MARKUP_LIMIT: usize = 8192;
const NODE_LIMIT: usize = 24_000;
const DEPTH_LIMIT: usize = 128;
const PROCESSING_TIMEOUT: Duration = Duration::from_secs(2);
static PROCESSING_SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
static TITLE_SELECTOR: LazyLock<Selector> = LazyLock::new(|| Selector::parse("title").expect("fixed selector"));

#[derive(Clone)]
pub(crate) struct Reader { http: Client }
struct PublicResolver;
impl Resolve for PublicResolver {
    fn resolve(&self,name: Name) -> Resolving {
        Box::pin(async move {
            // The exact validated addresses below are handed to the connector. There is
            // no preflight lookup followed by a second, unchecked connection lookup.
            public_url(&format!("https://{}/",name.as_str())).map_err(|_| std::io::Error::other("public destination required"))?;
            let addresses = tokio::time::timeout(Duration::from_secs(5),tokio::net::lookup_host((name.as_str(),0))).await.map_err(|_| std::io::Error::other("DNS timeout"))??;
            let addresses = checked_addresses(addresses.collect()).map_err(|_| std::io::Error::other("public destination required"))?;
            Ok(addresses)
        })
    }
}
fn checked_addresses(addresses: Vec<SocketAddr>) -> Result<Addrs,SearchError> {
    if addresses.is_empty() || addresses.len() > 64 || addresses.iter().any(|address| !public_ip(address.ip())) { return Err(SearchError::new(400,"page_public_destination_required")); }
    Ok(Box::new(addresses.into_iter()))
}
fn page_url(raw: &str) -> Result<Url,SearchError> {
    let mut url = public_url(raw)?;
    let default = if url.scheme() == "https" { 443 } else { 80 };
    if url.port_or_known_default() != Some(default) { return Err(SearchError::new(400,"page_default_port_required")); }
    url.set_fragment(None);
    Ok(url)
}
impl Reader {
    pub fn new() -> Result<Self,SearchError> {
        // A dedicated pool with no provider headers, cookies, proxies or redirect following.
        let http = Client::builder().no_proxy().retry(reqwest::retry::never()).redirect(reqwest::redirect::Policy::none()).dns_resolver(Arc::new(PublicResolver))
            .user_agent("nekochat-search-reader/0.1.0").connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(20)).build().map_err(|_| SearchError::new(502,"page_reader_configuration_failed"))?;
        Ok(Self { http })
    }
    pub async fn fetch(&self,raw: &str) -> Result<FetchResponse,SearchError> {
        let url = page_url(raw)?;
        let response = self.http.get(url.clone()).header("Accept","text/html, text/plain, text/markdown, application/xhtml+xml").send().await.map_err(|error| if error.is_timeout() { SearchError::new(504,"page_read_timeout") } else { SearchError::new(502,"page_read_failed") })?;
        let (title,content) = read_response(response).await?;
        Ok(FetchResponse { source:SearchSource { id:"s1".into(),title:if title.is_empty() { url.host_str().unwrap_or("Page").to_owned() } else { title },url:url.to_string(),snippet:String::new(),summary:None,content:Some(content),published_at:None },retrieved_at_ms:crate::now_ms(),usage:SearchUsage { requests:1,..Default::default() } })
    }
}
async fn read_response(response: reqwest::Response) -> Result<(String,String),SearchError> {
    if response.status().is_redirection() { return Err(SearchError::new(502,"page_redirect_not_followed")); }
    if !response.status().is_success() {
        return Err(match response.status().as_u16() { 401|402|403 => SearchError::new(502,"page_access_denied"),404 => SearchError::new(502,"page_not_found"),429 => SearchError::new(429,"page_rate_limited"),_ => SearchError::new(502,"page_read_failed") });
    }
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if !matches!(content_type.as_str(),"text/html"|"application/xhtml+xml"|"text/plain"|"text/markdown") { return Err(SearchError::new(502,"page_content_type_unsupported")); }
    let bytes = bounded_body(response,1024 * 1024).await?;
    let (title,content) = if matches!(content_type.as_str(),"text/html"|"application/xhtml+xml") {
        process_html(bytes).await?
    } else {
        let text = std::str::from_utf8(&bytes).map_err(|_| SearchError::new(502,"page_encoding_unsupported"))?;
        let mut characters = text.chars();
        let mut content: String = characters.by_ref().take(TEXT_LIMIT).collect();
        if characters.next().is_some() { content.push_str("\n[正文达到读取上限，未完整返回]"); }
        (String::new(), content)
    };
    if content.trim().is_empty() { return Err(SearchError::new(502,"page_static_text_unavailable")); }
    Ok((title,content))
}
async fn process_html(bytes: Vec<u8>) -> Result<(String,String),SearchError> {
    let cancelled = CancellationToken::new();
    // Dropping the enclosing fetch (explicit cancel/deadline) also stops queued/running work.
    let _cancel_on_drop = cancelled.clone().drop_guard();
    let deadline = Instant::now() + PROCESSING_TIMEOUT;
    tokio::time::timeout(PROCESSING_TIMEOUT, async move {
        let permit = PROCESSING_SLOTS.clone().acquire_owned().await
            .map_err(|_| SearchError::new(503,"page_processing_unavailable"))?;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| SearchError::new(502,"page_encoding_unsupported"))?;
            html_text(text,&cancelled,deadline)
        }).await.map_err(|_| SearchError::new(502,"page_processing_failed"))?
    }).await.map_err(|_| SearchError::new(504,"page_processing_timeout"))?
}

fn processing_check(cancelled: &CancellationToken,deadline: Instant) -> Result<(),SearchError> {
    if cancelled.is_cancelled() { return Err(SearchError::new(409,"request_cancelled")); }
    if Instant::now() >= deadline { return Err(SearchError::new(504,"page_processing_timeout")); }
    Ok(())
}

fn html_text(text: &str,cancelled: &CancellationToken,deadline: Instant) -> Result<(String,String),SearchError> {
    // Bound parser work before entering the non-cooperative third-party HTML parser.
    // Counting '<' is deliberately conservative, including markers in scripts/attributes.
    let mut markers = 0usize;
    for chunk in text.as_bytes().chunks(4096) {
        processing_check(cancelled,deadline)?;
        markers += chunk.iter().filter(|byte| **byte == b'<').count();
        if markers > MARKUP_LIMIT { return Err(SearchError::new(502,"page_markup_limit")); }
    }
    let document = Html::parse_document(text);
    processing_check(cancelled,deadline)?;
    let title = document.select(&TITLE_SELECTOR).next()
        .map(|node| node.text().flat_map(str::chars).take(300).collect::<String>())
        .unwrap_or_default();
    let mut output = String::new();
    let mut remaining = TEXT_LIMIT;
    let mut visited = 0usize;
    let mut depth = 0usize;
    let mut cursor = Some(document.tree.root());
    while let Some(mut node) = cursor {
        visited += 1;
        if visited > NODE_LIMIT || depth > DEPTH_LIMIT {
            return Err(SearchError::new(502,"page_structure_limit"));
        }
        if visited & 63 == 0 { processing_check(cancelled,deadline)?; }
        let excluded = match node.value() {
            Node::Element(element) => {
                matches!(element.name(),"head"|"script"|"style"|"nav"|"header"|"footer"|"aside"|"form"|"iframe"|"noscript"|"svg"|"template"|"button"|"select")
                    || element.attr("hidden").is_some() || element.attr("aria-hidden") == Some("true")
                    || matches!(element.attr("role"),Some("navigation"|"banner"|"contentinfo"))
            },
            _ => false,
        };
        if !excluded {
            if let Node::Text(text) = node.value() {
                let mut first_word = true;
                for word in text.split_whitespace() {
                    if !output.is_empty() && remaining > 0 {
                        output.push(if first_word { '\n' } else { ' ' });
                        remaining -= 1;
                    }
                    first_word = false;
                    for character in word.chars().take(remaining) { output.push(character); remaining -= 1; }
                    if remaining == 0 { break; }
                }
            }
            if remaining == 0 {
                output.push_str("\n[正文达到读取上限，可能未完整返回]");
                break;
            }
            if let Some(child) = node.first_child() {
                depth += 1;
                cursor = Some(child);
                continue;
            }
        }
        // Prune excluded subtrees; every visited edge is traversed at most twice.
        loop {
            if let Some(sibling) = node.next_sibling() {
                cursor = Some(sibling);
                break;
            }
            if let Some(parent) = node.parent() {
                depth = depth.saturating_sub(1);
                node = parent;
            } else {
                cursor = None;
                break;
            }
        }
    }
    processing_check(cancelled,deadline)?;
    Ok((title,output))
}

#[cfg(test)]
#[path = "reader_tests.rs"]
mod tests;
