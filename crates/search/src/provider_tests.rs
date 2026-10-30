use super::*;
use serde_json::json;

fn completed(items: serde_json::Value) -> String {
    format!("data: {}\n\n", json!({"type":"response.completed","response":{"status":"completed","output":items,"usage":{"input_tokens":120,"input_tokens_details":{"cached_tokens":20},"output_tokens":30,"output_tokens_details":{"reasoning_tokens":10}}}}))
}

#[test]
fn generated_answers_and_markdown_links_are_not_search_proof() {
    let text = json!({"type":"message","content":[{"type":"output_text","text":"See [Example](https://example.com)","annotations":[{"type":"url_citation","url":"https://example.com","title":"Example"}]}]});
    assert!(normalize_gpt(&completed(json!([text])),"test",5).is_err());
    let no_structured_sources = completed(json!([{"type":"web_search_call","status":"completed","action":{"type":"search","query":"test"}},{"type":"message","content":[{"type":"output_text","text":"https://example.com"}]}]));
    assert!(normalize_gpt(&no_structured_sources,"test",5).is_err());
}

#[test]
fn native_search_sources_are_deduplicated_and_generated_summary_is_not_page_content() {
    let input = completed(json!([
        {"type":"web_search_call","status":"completed","action":{"type":"search","sources":[{"url":"https://example.com/article","title":"Article"},{"url":"http://127.0.0.1/","title":"Private"}]}},
        {"type":"message","content":[{"type":"output_text","text":"Generated synthesis.","annotations":[{"type":"url_citation","url":"https://example.com/article","title":"Article"}]}]}
    ]));
    let response = normalize_gpt(&input,"test",5).unwrap();
    assert_eq!(response.sources.len(),1);
    assert_eq!(response.sources[0].url,"https://example.com/article");
    assert_eq!(response.sources[0].content,None);
    assert_eq!(response.summary.as_deref(),Some("Generated synthesis."));
    assert_eq!(response.usage.cached_in,20);
    assert_eq!(response.usage.uncached_in,100);
    assert_eq!(response.usage.output,20);
    assert_eq!(response.usage.reasoning,10);
    assert!(response.usage.cost.is_none());
}

#[test]
fn compact_completion_keeps_streamed_native_search_evidence() {
    let search = json!({"type":"response.output_item.done","item":{"type":"web_search_call","status":"completed","action":{"type":"search","sources":[{"url":"https://sqlite.org/wal.html","title":"Write-Ahead Logging"}]}}});
    let message = json!({"type":"response.output_item.done","item":{"type":"message","content":[{"type":"output_text","text":"WAL permits one writer at a time.","annotations":[{"type":"url_citation","url":"https://sqlite.org/wal.html","title":"Write-Ahead Logging"}]}]}});
    let input = format!("data: {{\"type\":\"response.web_search_call.completed\"}}\n\ndata: {search}\n\ndata: {message}\n\n{}", completed(json!([])));
    let response = normalize_gpt(&input,"SQLite WAL",3).unwrap();
    assert_eq!(response.sources.iter().map(|source| source.url.as_str()).collect::<Vec<_>>(), ["https://sqlite.org/wal.html"]);
    assert_eq!(response.summary.as_deref(),Some("WAL permits one writer at a time."));
    assert_eq!((response.usage.cached_in,response.usage.uncached_in,response.usage.output,response.usage.reasoning),(20,100,20,10));
}

#[test]
fn incomplete_and_failed_streams_never_become_successful_searches() {
    let item = "data: {\"type\":\"response.output_item.done\",\"item\":{\"type\":\"web_search_call\",\"action\":{\"sources\":[{\"url\":\"https://example.com/\"}]}}}\n\n";
    assert!(normalize_gpt(item,"test",5).is_err());
    let failed = format!("{item}data: {{\"type\":\"response.failed\",\"response\":{{\"error\":{{\"code\":\"rate_limit_exceeded\",\"message\":\"secret-token\"}}}}}}\n\n");
    let error = normalize_gpt(&failed,"test",5).unwrap_err();
    assert_eq!(error.status(),429);
    assert!(!error.to_string().contains("secret-token"));
}

#[test]
fn deepseek_preserves_summary_content_and_official_nonempty_fee() {
    let value = json!({"results":[{"title":"Article","url":"https://example.com/","snippet":"Extract","summary":"Generated summary","content":"Page body","date":"2026-01-01"}]});
    let response = normalize_deepseek(value,"test",5).unwrap();
    assert_eq!(response.sources[0].summary.as_deref(),Some("Generated summary"));
    assert_eq!(response.sources[0].content.as_deref(),Some("Page body"));
    assert_eq!(response.usage.requests,1);
    let cost = response.usage.cost.unwrap();
    assert_eq!(cost.amount,0.005);
    assert_eq!(cost.currency,"CNY");
}

#[test]
fn empty_deepseek_search_is_known_free_but_invalid_results_are_not_successes() {
    let response = normalize_deepseek(json!({"results":[]}),"test",5).unwrap();
    assert!(response.sources.is_empty());
    let cost = response.usage.cost.unwrap();
    assert_eq!(cost.amount,0.0);
    assert_eq!(cost.currency,"CNY");
    assert!(normalize_deepseek(json!({}),"test",5).is_err());
    assert!(normalize_deepseek(json!({"results":[{"url":"http://127.0.0.1/private"}]}),"test",5).is_err());
}

#[test]
fn deepseek_fetch_charges_only_real_nonempty_page_content() {
    let response = normalize_deepseek_fetch(json!({"title":"Article","content":"Actual page content"}),"https://example.com/article").unwrap();
    assert_eq!(response.source.content.as_deref(),Some("Actual page content"));
    assert_eq!(response.usage.requests,1);
    let cost = response.usage.cost.unwrap();
    assert_eq!(cost.amount,0.005);
    assert_eq!(cost.currency,"CNY");
    for value in [json!({}),json!({"content":""}),json!({"content":" \n\t "})] {
        assert!(normalize_deepseek_fetch(value,"https://example.com/article").is_err());
    }
}
