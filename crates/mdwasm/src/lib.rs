//! Markdown → safe HTML for the web frontend, compiled to WASM and run in a Web Worker.
//!
//! `split_blocks` cuts a document into top-level blocks so the frontend can freeze finished
//! blocks while streaming; `render_html` renders one block with raw HTML escaped, unsafe link
//! schemes dropped and math/code left as placeholders for KaTeX and highlight.js.

use std::ops::Range;

use pulldown_cmark::{CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use pulldown_cmark_escape::{escape_href, escape_html};
use wasm_bindgen::prelude::wasm_bindgen;

/// Validates a v1 graphic, completing missing container ends only for finished blocks.
#[wasm_bindgen]
pub fn validate_graphic(source: &str, complete: bool) -> Result<String, wasm_bindgen::JsValue> {
    let checked = if complete {
        nekochat_core::graphics::validate_complete_graphic_json(source)
    } else {
        nekochat_core::graphics::validate_graphic_json(source)
    };
    checked.map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

/// Rebuilds static SVG through the core allowlist.
#[wasm_bindgen]
pub fn sanitize_graphic_svg(source: &str) -> Result<String, wasm_bindgen::JsValue> {
    nekochat_core::graphics::sanitize_graphic_svg(source).map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

/// Samples a plot locally, using null points for nonfinite function values.
#[wasm_bindgen]
pub fn sample_graphic_plot(source: &str, values: &str) -> Result<String, wasm_bindgen::JsValue> {
    nekochat_core::graphics::sample_graphic_plot(source, values).map_err(|error| wasm_bindgen::JsValue::from_str(&error))
}

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_MATH
}

/// Splits `md` into the source text of its top-level blocks, in order.
#[wasm_bindgen]
pub fn split_blocks(md: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut depth = 0usize;
    for (event, range) in Parser::new_ext(md, options()).into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    blocks.push(md[range].to_string());
                }
                depth += 1;
            }
            Event::End(_) => depth -= 1,
            _ if depth == 0 => blocks.push(md[range].to_string()),
            _ => {}
        }
    }
    blocks
}

/// Renders `md` to sanitized HTML (see the crate docs and the plan-2 contract).
#[wasm_bindgen]
pub fn render_html(md: &str) -> String {
    let source = convert_tex_delimiters(md);
    let events = sanitize(Parser::new_ext(&source, options()));
    let events = unwrap_display_math_paragraphs(events);
    let mut out = String::with_capacity(source.len() * 3 / 2);
    pulldown_cmark::html::push_html(&mut out, events.into_iter());
    out
}

/// Rewrites `\(…\)` to `$…$` and `\[…\]` to `$$…$$` outside code, links, images and
/// existing math. Only paired delimiters are rewritten.
fn convert_tex_delimiters(md: &str) -> String {
    if !md.contains("\\(") && !md.contains("\\[") {
        return md.to_string();
    }
    let mut protected: Vec<Range<usize>> = Vec::new();
    let mut code_block_start = None;
    for (event, range) in Parser::new_ext(md, options()).into_offset_iter() {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_start = Some(range.start),
            Event::End(TagEnd::CodeBlock) => {
                if let Some(start) = code_block_start.take() {
                    protected.push(start..range.end);
                }
            }
            Event::Start(Tag::Link { .. } | Tag::Image { .. }) => protected.push(range),
            Event::Code(_) | Event::InlineMath(_) | Event::DisplayMath(_) => protected.push(range),
            _ => {}
        }
    }
    // An unclosed fence still yields its End event, but be defensive anyway.
    if let Some(start) = code_block_start {
        protected.push(start..md.len());
    }
    protected.sort_by_key(|r| r.start);

    // Candidate delimiters: (byte offset, kind) where kind is one of `(`, `)`, `[`, `]`.
    let bytes = md.as_bytes();
    let mut delims = Vec::new();
    let mut p = 0;
    let mut i = 0;
    while i + 1 < bytes.len() {
        while p < protected.len() && protected[p].end <= i {
            p += 1;
        }
        if p < protected.len() && protected[p].start <= i {
            i = protected[p].end;
            continue;
        }
        if bytes[i] == b'\\' {
            match bytes[i + 1] {
                b'\\' => {
                    i += 2;
                    continue;
                }
                c @ (b'(' | b')' | b'[' | b']') => {
                    delims.push((i, c));
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        i += 1;
    }

    let mut replacements: Vec<(usize, &str)> = Vec::new();
    let mut open: Option<(usize, u8)> = None;
    for (pos, c) in delims {
        match (open, c) {
            (_, b'(' | b'[') => open = Some((pos, c)),
            (Some((start, b'(')), b')') => {
                replacements.push((start, "$"));
                replacements.push((pos, "$"));
                open = None;
            }
            (Some((start, b'[')), b']') => {
                replacements.push((start, "$$"));
                replacements.push((pos, "$$"));
                open = None;
            }
            _ => {}
        }
    }

    let mut out = String::with_capacity(md.len());
    let mut last = 0;
    for (pos, with) in replacements {
        out.push_str(&md[last..pos]);
        out.push_str(with);
        last = pos + 2;
    }
    out.push_str(&md[last..]);
    out
}

fn link_allowed(url: &str) -> bool {
    let url = url.trim_start_matches(|c: char| c.is_whitespace() || c.is_control());
    if url.starts_with('#') || url.starts_with('/') {
        return true;
    }
    has_scheme(url, &["http:", "https:", "mailto:"])
}

fn image_allowed(url: &str) -> bool {
    fn component(s: &str) -> bool {
        !s.is_empty()
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    }
    if let Some(path) = url.strip_prefix("/api/resources/") {
        return path
            .split_once('/')
            .is_some_and(|(pack, asset)| component(pack) && component(asset));
    }
    if let Some(file) = url.strip_prefix("/api/backgrounds/") {
        return file.rsplit_once('.').is_some_and(|(id, ext)| {
            id.len() == 36
                && id.bytes().enumerate().all(|(i, b)| {
                    if [8, 13, 18, 23].contains(&i) {
                        b == b'-'
                    } else {
                        b.is_ascii_hexdigit()
                    }
                })
                && ["png", "jpg", "webp", "gif", "avif"].contains(&ext)
        });
    }
    false
}

fn has_scheme(url: &str, schemes: &[&str]) -> bool {
    schemes.iter().any(|s| {
        url.get(..s.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(s))
    })
}

fn escaped(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_html(&mut out, s).expect("writing to a String cannot fail");
    out
}

fn escaped_href(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    escape_href(&mut out, s).expect("writing to a String cannot fail");
    out
}

/// Rewrites parser events so the stock HTML writer only ever emits safe markup.
fn sanitize<'a>(events: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut out = Vec::new();
    // For each open link: whether its <a> was emitted.
    let mut links: Vec<bool> = Vec::new();
    // Open image: (allowed src, title, collected alt text, nesting depth inside the image).
    let mut image: Option<(Option<String>, String, String, usize)> = None;

    for event in events {
        if let Some((src, title, alt, depth)) = image.as_mut() {
            match event {
                Event::Start(_) => *depth += 1,
                Event::End(TagEnd::Image) if *depth == 0 => {
                    let html = match src {
                        Some(src) => {
                            let mut tag = format!(
                                "<img src=\"{}\" alt=\"{}\"",
                                escaped_href(src),
                                escaped(alt)
                            );
                            if !title.is_empty() {
                                tag.push_str(&format!(" title=\"{}\"", escaped(title)));
                            }
                            tag.push_str(" loading=\"lazy\" decoding=\"async\" style=\"max-width:100%;max-height:24rem\">");
                            Event::InlineHtml(tag.into())
                        }
                        None => Event::Text(std::mem::take(alt).into()),
                    };
                    out.push(html);
                    image = None;
                }
                Event::End(_) => *depth -= 1,
                Event::Text(t)
                | Event::Code(t)
                | Event::InlineMath(t)
                | Event::DisplayMath(t)
                | Event::Html(t)
                | Event::InlineHtml(t) => alt.push_str(&t),
                Event::SoftBreak | Event::HardBreak => alt.push(' '),
                _ => {}
            }
            continue;
        }

        match event {
            Event::Html(t) | Event::InlineHtml(t) => out.push(Event::Text(t)),
            Event::Start(Tag::HtmlBlock) => out.push(Event::Start(Tag::Paragraph)),
            Event::End(TagEnd::HtmlBlock) => out.push(Event::End(TagEnd::Paragraph)),
            Event::InlineMath(t) => out.push(Event::InlineHtml(
                format!(
                    "<span class=\"nk-math\" data-display=\"0\">{}</span>",
                    escaped(&t)
                )
                .into(),
            )),
            Event::DisplayMath(t) => out.push(Event::InlineHtml(
                format!(
                    "<div class=\"nk-math\" data-display=\"1\">{}</div>",
                    escaped(&t)
                )
                .into(),
            )),
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                title,
                ..
            }) => {
                let href: CowStr = if link_type == LinkType::Email {
                    format!("mailto:{dest_url}").into()
                } else {
                    dest_url
                };
                let allowed = link_allowed(&href);
                if allowed {
                    let mut tag = format!("<a href=\"{}\"", escaped_href(&href));
                    if !title.is_empty() {
                        tag.push_str(&format!(" title=\"{}\"", escaped(&title)));
                    }
                    tag.push_str(" target=\"_blank\" rel=\"noopener noreferrer nofollow\">");
                    out.push(Event::InlineHtml(tag.into()));
                }
                links.push(allowed);
            }
            Event::End(TagEnd::Link) => {
                if links.pop().unwrap_or(false) {
                    out.push(Event::InlineHtml("</a>".into()));
                }
            }
            Event::Start(Tag::Image {
                dest_url, title, ..
            }) => {
                let src = image_allowed(&dest_url).then(|| dest_url.to_string());
                image = Some((src, title.to_string(), String::new(), 0));
            }
            other => out.push(other),
        }
    }
    out
}

/// A paragraph holding nothing but display math would put a `<div>` inside a `<p>`; drop the
/// paragraph wrapper in that case.
fn unwrap_display_math_paragraphs(events: Vec<Event<'_>>) -> Vec<Event<'_>> {
    let is_display =
        |e: &Event| matches!(e, Event::InlineHtml(h) if h.starts_with("<div class=\"nk-math\""));
    let is_filler = |e: &Event| match e {
        Event::SoftBreak | Event::HardBreak => true,
        Event::Text(t) => t.trim().is_empty(),
        _ => false,
    };
    let mut out = Vec::with_capacity(events.len());
    let mut i = 0;
    while i < events.len() {
        if matches!(events[i], Event::Start(Tag::Paragraph)) {
            let end = events[i..]
                .iter()
                .position(|e| matches!(e, Event::End(TagEnd::Paragraph)))
                .map(|p| i + p);
            if let Some(end) = end {
                let inner = &events[i + 1..end];
                if inner.iter().any(is_display)
                    && inner.iter().all(|e| is_display(e) || is_filler(e))
                {
                    out.extend(inner.iter().filter(|e| is_display(e)).cloned());
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(events[i].clone());
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_injection_is_escaped() {
        let html = render_html(
            "<script>alert(1)</script>\n\nhi <script>x</script> <img src=x onerror=alert(1)>",
        );
        assert!(!html.contains("<script"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn javascript_links_are_rejected() {
        for md in [
            "[x](javascript:alert(1))",
            "[x](JavaScript:alert(1))",
            "[x]( data:text/html,hi)",
            "<javascript:alert(1)>",
            "[x](vbscript:msgbox)",
        ] {
            let html = render_html(md);
            assert!(!html.contains("<a"), "{md} -> {html}");
            assert!(!html.contains("href"), "{md} -> {html}");
        }
        let html = render_html("![a](javascript:alert(1))");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains('a'), "{html}");
    }

    #[test]
    fn safe_links_and_images() {
        let html = render_html("[x](https://e.com/?a=1&b=2) [y](#top) [z](/p) <m@e.com>");
        assert!(html.contains("<a href=\"https://e.com/?a=1&amp;b=2\" target=\"_blank\" rel=\"noopener noreferrer nofollow\">x</a>"), "{html}");
        assert!(html.contains("href=\"#top\""), "{html}");
        assert!(html.contains("href=\"/p\""), "{html}");
        assert!(html.contains("href=\"mailto:m@e.com\""), "{html}");
        let html = render_html("![a *b*](/api/resources/nekowhale/abc.gif)");
        assert!(
            html.contains("<img src=\"/api/resources/nekowhale/abc.gif\" alt=\"a b\""),
            "{html}"
        );
        assert!(
            html.contains("loading=\"lazy\" decoding=\"async\""),
            "{html}"
        );
        let html = render_html("![AVIF](/api/backgrounds/00000000-0000-4000-8000-000000000000.avif)");
        assert!(html.contains("<img "), "{html}");
        assert!(html.contains("src=\"/api/backgrounds/00000000-0000-4000-8000-000000000000.avif\""), "{html}");
        for url in [
            "https://e.com/i.png",
            "//e.com/i.png",
            "/api/resources/a/../b",
            "/api/resources/a/%2e%2e",
            "/other/a.png",
            "data:image/png,hi",
        ] {
            let html = render_html(&format!("![alt]({url})"));
            assert!(!html.contains("<img"), "{url}: {html}");
            assert!(html.contains("alt"), "{html}");
        }
    }

    #[test]
    fn math_placeholders() {
        let html = render_html("a $x<y$ b");
        assert!(
            html.contains("<span class=\"nk-math\" data-display=\"0\">x&lt;y</span>"),
            "{html}"
        );
        let html = render_html("$$\n\\sum_i a_i\n$$");
        assert_eq!(
            html.trim(),
            "<div class=\"nk-math\" data-display=\"1\">\n\\sum_i a_i\n</div>"
        );
    }

    #[test]
    fn tex_delimiters_are_converted_outside_code() {
        let html = render_html("\\[x^2\\] and \\(y\\)");
        assert!(
            html.contains("<div class=\"nk-math\" data-display=\"1\">x^2</div>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"nk-math\" data-display=\"0\">y</span>"),
            "{html}"
        );

        let html = render_html("```\n\\[x\\]\n```\n\n`\\(y\\)`");
        assert!(!html.contains("nk-math"), "{html}");
        assert!(html.contains("\\[x\\]"), "{html}");
        assert!(html.contains("\\(y\\)"), "{html}");

        assert_eq!(convert_tex_delimiters("\\[unpaired"), "\\[unpaired");
        assert_eq!(convert_tex_delimiters("\\\\[x\\\\]"), "\\\\[x\\\\]");
    }

    #[test]
    fn tex_delimiters_preserve_escaped_link_labels() {
        for (md, href, label) in [
            (
                r"[5] [\[情报站\]HLE对阵BLG交手记录](<https://bbs.hupu.com/643477528.html>)",
                "https://bbs.hupu.com/643477528.html",
                "[情报站]HLE对阵BLG交手记录",
            ),
            (
                r"[7] [\[国际赛事\]BLG最近10次对LCK战绩](<https://ngabbs.com/read.php?tid=44494056>)",
                "https://ngabbs.com/read.php?tid=44494056",
                "[国际赛事]BLG最近10次对LCK战绩",
            ),
            (
                r"[\(赛程\)与\[记录\]](https://example.com/matches)",
                "https://example.com/matches",
                "(赛程)与[记录]",
            ),
        ] {
            let html = render_html(md);
            assert!(html.contains(&format!("<a href=\"{href}\"")), "{html}");
            assert!(html.contains(&format!(">{label}</a>")), "{html}");
            assert!(!html.contains("nk-math"), "{html}");
        }
    }

    #[test]
    fn tex_delimiters_preserve_reference_link_labels() {
        let html = render_html(
            r"前缀 [\[资料\]\(赛程\)][赛事来源]

[赛事来源]: https://example.com/matches",
        );
        assert!(html.contains("<a href=\"https://example.com/matches\""), "{html}");
        assert!(html.contains(">[资料](赛程)</a>"), "{html}");
        assert!(!html.contains("nk-math"), "{html}");
    }

    #[test]
    fn tex_delimiters_preserve_image_alt_text() {
        for md in [
            r"![\[赛程\]\(图示\)](/api/resources/nekowhale/abc.gif)",
            r"![\[赛程\]\(图示\)][图片]

[图片]: /api/resources/nekowhale/abc.gif",
            r"[![\[赛程\]\(图示\)](/api/resources/nekowhale/abc.gif)](https://example.com/matches)",
        ] {
            let html = render_html(md);
            assert!(html.contains(
                "<img src=\"/api/resources/nekowhale/abc.gif\" alt=\"[赛程](图示)\""
            ), "{html}");
            assert!(!html.contains("nk-math"), "{html}");
        }
    }

    #[test]
    fn tex_delimiters_convert_math_adjacent_to_links() {
        let html = render_html(
            r"\(before\)[\[来源\]\(记录\)](https://example.com/matches)\(after\)

\[z^2\]",
        );
        assert!(html.contains("<a href=\"https://example.com/matches\""), "{html}");
        assert!(html.contains(">[来源](记录)</a>"), "{html}");
        assert!(html.contains(
            "<span class=\"nk-math\" data-display=\"0\">before</span><a "
        ), "{html}");
        assert!(html.contains(
            "</a><span class=\"nk-math\" data-display=\"0\">after</span>"
        ), "{html}");
        assert!(html.contains(
            "<div class=\"nk-math\" data-display=\"1\">z^2</div>"
        ), "{html}");
        assert_eq!(html.matches("class=\"nk-math\"").count(), 3, "{html}");
    }

    #[test]
    fn code_blocks() {
        let html = render_html("```rust\nfn a() -> i32 { 1 < 2 }\n```");
        assert_eq!(
            html,
            "<pre><code class=\"language-rust\">fn a() -&gt; i32 { 1 &lt; 2 }\n</code></pre>\n"
        );
        let html = render_html("```\nx\n```");
        assert_eq!(html, "<pre><code>x\n</code></pre>\n");
    }

    #[test]
    fn extensions() {
        let html = render_html("| a |\n|---|\n| b |\n\n~~s~~\n\n- [x] done");
        assert!(html.contains("<table>"), "{html}");
        assert!(html.contains("<del>s</del>"), "{html}");
        assert!(html.contains("checkbox"), "{html}");
    }

    #[test]
    fn split_top_level_blocks() {
        let md = "# T\n\npara\nline\n\n- a\n- b\n\n  c\n\n> q\n\n---\n\n$$\nx\n$$\n";
        assert_eq!(
            split_blocks(md),
            [
                "# T\n",
                "para\nline\n",
                "- a\n- b\n\n  c\n\n",
                "> q\n",
                "---\n",
                "$$\nx\n$$\n"
            ]
        );
    }

    #[test]
    fn unclosed_fence_is_one_trailing_block() {
        let md = "intro\n\n```py\nprint(1)\n\nprint(2)\n";
        let blocks = split_blocks(md);
        assert_eq!(blocks, ["intro\n", "```py\nprint(1)\n\nprint(2)\n"]);
        let html = render_html(&blocks[1]);
        assert!(
            html.starts_with("<pre><code class=\"language-py\">"),
            "{html}"
        );
    }
}
