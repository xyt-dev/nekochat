// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Bounded SVG image documents. Native drawing grammar is preserved; executable
//! content, network resources and recursive href subtree expansion are rejected.

use std::collections::HashMap;
use roxmltree::{Document, Node, ParsingOptions};
use super::Checked;

mod css;
mod data;
use data::encode_base64;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XHTML_NS: &str = "http://www.w3.org/1999/xhtml";
const XLINK_NS: &str = "http://www.w3.org/1999/xlink";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

const ELEMENTS: &[&str] = &[
    "svg", "g", "defs", "symbol", "use", "a", "switch", "view", "title", "desc", "metadata", "style",
    "path", "rect", "circle", "ellipse", "line", "polyline", "polygon",
    "text", "tspan", "textPath", "linearGradient", "radialGradient", "stop",
    "clipPath", "mask", "pattern", "marker", "image", "foreignObject", "filter",
    "feBlend", "feColorMatrix", "feComponentTransfer", "feComposite", "feConvolveMatrix",
    "feDiffuseLighting", "feDisplacementMap", "feDistantLight", "feDropShadow", "feFlood",
    "feFuncA", "feFuncB", "feFuncG", "feFuncR", "feGaussianBlur", "feImage", "feMerge",
    "feMergeNode", "feMorphology", "feOffset", "fePointLight", "feSpecularLighting",
    "feSpotLight", "feTile", "feTurbulence", "animate", "animateMotion", "animateTransform", "mpath", "set",
];
const HTML_ELEMENTS: &[&str] = &[
    "div", "span", "p", "br", "hr", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "code",
    "blockquote", "q", "em", "strong", "b", "i", "u", "s", "small", "sub", "sup", "mark",
    "ul", "ol", "li", "dl", "dt", "dd", "table", "thead", "tbody", "tfoot", "tr", "td", "th",
    "caption", "colgroup", "col", "figure", "figcaption", "ruby", "rt", "rp", "bdi", "bdo", "wbr", "img", "a",
];

#[derive(Default)]
struct Budget { bytes: usize, elements: usize }

struct Sanitized { xml: String, expansion: usize }


/// Validates and rebuilds an SVG for the shared Rust/WASM image renderer.
/// This must stay an isolated image document, never application-DOM markup.
pub fn sanitize_graphic_svg(source: &str) -> Checked<String> {
    sanitize_document(source, &mut Budget::default(), 0, 0).map(|result| result.xml)
}

fn sanitize_document(source: &str, budget: &mut Budget, depth: usize, image_depth: usize) -> Checked<Sanitized> {
    budget.bytes += source.len();
    if budget.bytes > 64 * 1024 { return Err("SVG documents exceed aggregate 64 KiB".into()); }
    if image_depth > 8 { return Err("SVG embedded image nesting exceeds 8 levels".into()); }
    let doc = Document::parse_with_options(source, ParsingOptions {
        allow_dtd: false, nodes_limit: 8192, ..ParsingOptions::default()
    }).map_err(|e| format!("Invalid SVG XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" || !matches!(root.tag_name().namespace(), None | Some(SVG_NS)) {
        return Err("Expected an SVG root element".into());
    }
    if doc.descendants().any(|n| n.is_pi()) { return Err("SVG processing instructions are forbidden".into()); }
    // roxmltree rejects entity declarations with allow_dtd=false; reject the
    // declaration itself as well, including an empty external/internal DTD.
    if source.contains("<!DOCTYPE") || source.contains("<!ENTITY") {
        return Err("SVG DTDs and entity declarations are forbidden".into());
    }
    let nodes: Vec<_> = doc.descendants().filter(Node::is_element).collect();
    budget.elements += nodes.len();
    if budget.elements > 1024 { return Err("SVG documents exceed aggregate 1024 elements".into()); }
    let positions: HashMap<_, _> = nodes.iter().enumerate().map(|(i, n)| (n.id(), i)).collect();
    let mut ids = HashMap::new();
    for (index, node) in nodes.iter().enumerate() {
        if let Some(id) = node.attribute("id") {
            if !local_id(id) || ids.insert(id.to_owned(), index).is_some() {
                return Err("SVG ids must be valid and unique".into());
            }
        }
    }
    let mut references = Vec::new();
    let mut structural = Vec::new();
    let mut graph = vec![Vec::new(); nodes.len()];
    let mut weights = vec![1usize; nodes.len()];
    let mut rewrites = HashMap::new();
    for (index, &node) in nodes.iter().enumerate() {
        if depth + node.ancestors().filter(Node::is_element).count() > 32 {
            return Err("SVG exceeds 32 nesting levels".into());
        }
        validate_element(node, root)?;
        if let Some(parent) = node.parent_element() { graph[positions[&parent.id()]].push(index); }
        if node.attributes().len() > 64 { return Err("Too many SVG attributes".into()); }
        let animation = is_animation(node.tag_name().name());
        for attr in node.attributes() {
            let key = attr.name();
            let value = attr.value().trim();
            validate_attribute_namespace(attr.namespace(), key, value)?;
            if attr.namespace() == Some(XML_NS) { continue; }
            if key == "id" { continue; }
            if forbidden_attribute(key) { return Err(format!("Forbidden SVG attribute: {key}")); }
            if ["aria-labelledby", "aria-describedby"].contains(&key) {
                if value.is_empty() || !value.split_ascii_whitespace().all(local_id) {
                    return Err("Invalid SVG accessibility references".into());
                }
                references.extend(value.split_ascii_whitespace().map(str::to_owned));
                continue;
            }
            if key == "href" || key == "src" {
                let embedded = matches!(node.tag_name().name(), "image" | "feImage" | "img");
                if embedded {
                    if let Some(nested) = nested_svg(value, budget, depth + node.ancestors().filter(Node::is_element).count(), image_depth + 1)? {
                        weights[index] += nested.expansion;
                        rewrites.insert(attr.range_qname().start, format!("data:image/svg+xml;base64,{}", encode_base64(nested.xml.as_bytes())));
                        continue;
                    }
                }
                let reference = resource_url(value, embedded)?;
                if let Some(id) = reference {
                    if ["use", "linearGradient", "radialGradient", "pattern", "image", "feImage"].contains(&node.tag_name().name()) {
                        structural.push((index, id.clone()));
                    }
                    references.push(id);
                }
                continue;
            }
            if node.tag_name().namespace() == Some(XHTML_NS) { validate_html_attribute(key)?; }
            if key == "style" {
                references.extend(css::declarations(value)?.references);
            } else if animation && ["from", "to", "by", "values"].contains(&key) {
                for value in value.split(';') {
                    if let Some(attribute) = node.attribute("attributeName") {
                        if !value.trim().starts_with("var(") { validate_numeric_attribute(attribute, value.trim())?; }
                    }
                    references.extend(css::value(value.trim())?.references);
                }
            } else if animation && ["keyTimes", "keySplines", "keyPoints"].contains(&key) {
                for value in value.split(';') {
                    references.extend(css::value(value.trim())?.references);
                }
            } else if !["class", "role", "lang", "aria-label", "aria-description", "title", "alt", "begin", "end"].contains(&key)
                && !key.starts_with("aria-") && !key.starts_with("data-") {
                // Attributes ignored by native SVG are inert; known native drawing
                // attributes need no parallel whitelist or private color grammar.
                validate_numeric_attribute(key, value)?;
                references.extend(css::value(value)?.references);
            }
        }
        if animation { validate_animation(node)?; }
        if node.tag_name().name() == "style" {
            if node.children().any(|n| n.is_element()) { return Err("SVG style must contain CSS text".into()); }
            let stylesheet: String = node.children().filter(Node::is_text).filter_map(|n| n.text()).collect();
            references.extend(css::stylesheet(&stylesheet)?.references);
        }
    }
    check_references(&references, &ids)?;
    // Paint/CSS recursion is defined by the native SVG engine's invalid-resource
    // handling. Only explicit subtree-reusing href edges belong in this graph;
    // inferring cascade/inheritance here would reject valid ordinary drawings.
    for (owner, id) in structural { graph[owner].push(ids[&id]); }
    for edges in &mut graph { edges.sort_unstable(); edges.dedup(); }
    let mut state = vec![0u8; nodes.len()];
    let mut sizes = vec![0usize; nodes.len()];
    for index in 0..nodes.len() { check_resources(index, &graph, &weights, &mut state, &mut sizes)?; }
    let mut output = String::with_capacity(source.len());
    write_element(root, source, &rewrites, &mut output, true);
    budget.bytes = budget.bytes - source.len() + output.len();
    if budget.bytes > 64 * 1024 { return Err("Normalized SVG documents exceed aggregate 64 KiB".into()); }
    Ok(Sanitized { xml: output, expansion: sizes[0] })
}

fn nested_svg(value: &str, budget: &mut Budget, depth: usize, image_depth: usize) -> Checked<Option<Sanitized>> {
    let Some(source) = data::svg_source(value)? else { return Ok(None); };
    sanitize_document(&source, budget, depth, image_depth).map(Some)
}


fn validate_element(node: Node<'_, '_>, root: Node<'_, '_>) -> Checked<()> {
    for ns in node.namespaces() {
        if ![SVG_NS, XHTML_NS, XLINK_NS, XML_NS].contains(&ns.uri()) {
            return Err("Unknown SVG namespace declaration".into());
        }
    }
    let tag = node.tag_name();
    match tag.namespace() {
        Some(SVG_NS) => {
            if !ELEMENTS.contains(&tag.name()) { return Err(format!("Forbidden SVG element: {}", tag.name())); }
        }
        None if root.tag_name().namespace().is_none() && !node.ancestors().any(|n| n.tag_name().namespace() == Some(XHTML_NS)) => {
            if !ELEMENTS.contains(&tag.name()) { return Err(format!("Forbidden SVG element: {}", tag.name())); }
        }
        Some(XHTML_NS) if node.ancestors().any(|n| n.tag_name().name() == "foreignObject" && matches!(n.tag_name().namespace(), None | Some(SVG_NS))) => {
            if !HTML_ELEMENTS.contains(&tag.name()) { return Err(format!("Forbidden XHTML presentation element: {}", tag.name())); }
        }
        _ => return Err("Foreign SVG element namespace".into()),
    }
    Ok(())
}

fn validate_attribute_namespace(namespace: Option<&str>, key: &str, value: &str) -> Checked<()> {
    match namespace {
        None => Ok(()),
        Some(XLINK_NS) if key == "href" => Ok(()),
        Some(XML_NS) if key == "lang" || key == "space" && ["default", "preserve"].contains(&value) => Ok(()),
        _ => Err(format!("Forbidden SVG attribute namespace: {key}")),
    }
}

fn forbidden_attribute(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.starts_with("on") || key.contains(':') || [
        "srcdoc", "srcset", "imagesrcset", "ping", "action", "formaction", "poster", "background",
        "data", "codebase", "archive", "manifest", "profile", "http-equiv", "content", "autofocus",
        "is", "nonce", "integrity", "crossorigin", "download", "target", "xml:base",
    ].contains(&key.as_str())
}

fn validate_html_attribute(key: &str) -> Checked<()> {
    if ["id", "class", "style", "lang", "dir", "title", "role", "alt", "width", "height", "colspan", "rowspan", "span", "start", "reversed", "type", "value", "scope", "headers", "abbr", "align", "valign", "cellpadding", "cellspacing", "border"].contains(&key)
        || key.starts_with("aria-") || key.starts_with("data-") {
        Ok(())
    } else { Err(format!("Forbidden XHTML presentation attribute: {key}")) }
}

fn is_animation(tag: &str) -> bool { ["animate", "animateTransform", "animateMotion", "set"].contains(&tag) }

fn validate_animation(node: Node<'_, '_>) -> Checked<()> {
    if node.tag_name().name() == "animateMotion" { return Ok(()); }
    let attribute = node.attribute("attributeName").ok_or("SVG animation requires attributeName")?;
    if forbidden_attribute(attribute) || ["href", "src", "style", "id", "class", "attributeName", "xmlns"].contains(&attribute)
        || attribute.starts_with("data-") || attribute.starts_with("aria-") {
        return Err("SVG animation may only change drawing attributes".into());
    }
    Ok(())
}


fn local_id(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|c| c.is_alphanumeric() || ['_', '-', '.', ':'].contains(&c))
}

/// URL tokens have already been unescaped by the XML/CSS parsers. Only same
/// document fragments and self-contained raster/font payloads may reach CSS.
fn resource_url(value: &str, embedded: bool) -> Checked<Option<String>> {
    let value = value.trim();
    if let Some(id) = value.strip_prefix('#') {
        let id = decode_fragment(id)?;
        if local_id(&id) { return Ok(Some(id)); }
        return Err("Invalid SVG local fragment".into());
    }
    if embedded && embedded_data(value) { return Ok(None); }
    Err("SVG resources must be local fragments or validated embedded images/fonts".into())
}

fn decode_fragment(value: &str) -> Checked<String> {
    let mut result = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let high = bytes.next().and_then(|b| (b as char).to_digit(16)).ok_or("Invalid SVG fragment escape")?;
            let low = bytes.next().and_then(|b| (b as char).to_digit(16)).ok_or("Invalid SVG fragment escape")?;
            result.push((high * 16 + low) as u8);
        } else { result.push(byte); }
    }
    String::from_utf8(result).map_err(|_| "Invalid SVG fragment UTF-8".into())
}

fn embedded_data(value: &str) -> bool {
    let Some((header, payload)) = value.split_once(',') else { return false; };
    let header = header.to_ascii_lowercase();
    let Some(mime) = header.strip_prefix("data:").and_then(|h| h.strip_suffix(";base64")) else { return false; };
    let encoded_len = payload.bytes().filter(|byte| !byte.is_ascii_whitespace()).count();
    if !["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif", "image/bmp", "font/woff", "font/woff2", "font/ttf", "font/otf", "application/font-woff", "application/x-font-ttf", "application/x-font-opentype"].contains(&mime) || encoded_len == 0 || encoded_len % 4 != 0 { return false; }
    let mut prefix = [0u8; 32];
    let mut written = 0;
    let mut accumulator = 0u32;
    let mut bits = 0;
    let mut padding = 0;
    for byte in payload.bytes() {
        if byte.is_ascii_whitespace() { continue; }
        if byte == b'=' { padding += 1; continue; }
        if padding != 0 { return false; }
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A', b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52, b'+' => 62, b'/' => 63,
            _ => return false,
        };
        accumulator = (accumulator << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            if written < prefix.len() { prefix[written] = (accumulator >> bits) as u8; }
            written += 1;
        }
    }
    if padding > 2 || bits != padding * 2 || accumulator & ((1 << bits) - 1) != 0 { return false; }
    let prefix = &prefix[..written.min(prefix.len())];
    match mime {
        "image/png" => prefix.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => prefix.starts_with(b"\xff\xd8\xff"),
        "image/gif" => prefix.starts_with(b"GIF87a") || prefix.starts_with(b"GIF89a"),
        "image/webp" => prefix.starts_with(b"RIFF") && prefix.get(8..12) == Some(b"WEBP"),
        "image/avif" => prefix.get(4..8) == Some(b"ftyp") && (prefix.get(8..12) == Some(b"avif") || prefix.get(8..12) == Some(b"avis")),
        "image/bmp" => prefix.starts_with(b"BM"),
        "font/woff" | "application/font-woff" => prefix.starts_with(b"wOFF"),
        "font/woff2" => prefix.starts_with(b"wOF2"),
        "font/ttf" | "application/x-font-ttf" => prefix.starts_with(b"\0\x01\0\0"),
        "font/otf" | "application/x-font-opentype" => prefix.starts_with(b"OTTO"),
        _ => false,
    }
}

fn check_references(references: &[String], ids: &HashMap<String, usize>) -> Checked<()> {
    for id in references {
        if !ids.contains_key(id) { return Err(format!("Unknown SVG fragment target: {id}")); }
    }
    Ok(())
}


fn check_resources(index: usize, graph: &[Vec<usize>], weights: &[usize], state: &mut [u8], sizes: &mut [usize]) -> Checked<usize> {
    match state[index] {
        1 => return Err("Cyclic SVG resource references are forbidden".into()),
        2 => return Ok(sizes[index]),
        _ => {},
    }
    state[index] = 1;
    let mut size = weights[index];
    if size > 64 * 1024 { return Err("SVG resource expansion exceeds 65536 elements".into()); }
    for &target in &graph[index] {
        size += check_resources(target, graph, weights, state, sizes)?;
        if size > 64 * 1024 { return Err("SVG resource expansion exceeds 65536 elements".into()); }
    }
    sizes[index] = size;
    state[index] = 2;
    Ok(size)
}

fn finite(value: &str) -> bool { value.parse::<f64>().is_ok_and(f64::is_finite) }

// Numeric guardrails for consumer-facing geometry. CSS/layout expression grammar
// otherwise remains native, including modern units, math and text coordinate lists.
fn validate_numeric_attribute(key: &str, value: &str) -> Checked<()> {
    if key == "viewBox" {
        let parts: Vec<_> = value.split(|c: char| c.is_ascii_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
        if parts.len() != 4 || !parts.iter().all(|v| finite(v)) || !parts[2..].iter().all(|v| v.parse::<f64>().is_ok_and(|n| n > 0.0)) {
            return Err("Invalid SVG viewBox".into());
        }
    }
    if key == "d" && !path_numbers(value) { return Err("Invalid or nonfinite SVG path numbers".into()); }
    if key == "font-size" {
        use cssparser::{Parser, ParserInput, Token};
        let mut input = ParserInput::new(value);
        let mut parser = Parser::new(&mut input);
        let token = parser.next().map_err(|_| "Invalid SVG font size")?.clone();
        let valid = match token {
            Token::Number { value, .. } | Token::Percentage { unit_value: value, .. } => value.is_finite() && value >= 0.0,
            Token::Dimension { value, unit, .. } => value.is_finite() && value >= 0.0 && [
                "px", "em", "rem", "ex", "rex", "cap", "rcap", "ch", "rch", "ic", "ric", "lh", "rlh",
                "in", "cm", "mm", "q", "pt", "pc", "vw", "vh", "vi", "vb", "vmin", "vmax",
                "svw", "svh", "svi", "svb", "svmin", "svmax", "lvw", "lvh", "lvi", "lvb", "lvmin", "lvmax",
                "dvw", "dvh", "dvi", "dvb", "dvmin", "dvmax", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax",
            ].contains(&unit.to_ascii_lowercase().as_str()),
            Token::Ident(name) => ["inherit", "initial", "unset", "revert", "revert-layer", "xx-small", "x-small", "small", "medium", "large", "x-large", "xx-large", "xxx-large", "larger", "smaller", "math"].contains(&name.as_ref()),
            Token::Function(_) => return Ok(()),
            _ => false,
        };
        if !valid || !parser.is_exhausted() { return Err("Invalid or nonfinite SVG font size".into()); }
    }
    if ["x", "y", "dx", "dy", "rotate", "cx", "cy", "r", "rx", "ry", "width", "height", "x1", "y1", "x2", "y2", "fx", "fy", "fr", "transform", "gradientTransform", "patternTransform", "points", "pathLength", "textLength", "stroke-width", "opacity", "fill-opacity", "stroke-opacity", "stop-opacity"].contains(&key) {
        // CSS treats these spellings as identifiers; reject them explicitly on
        // numeric surfaces instead of accidentally forwarding f64 special values.
        let lower = value.to_ascii_lowercase();
        if lower.split(|c: char| !c.is_ascii_alphanumeric() && c != '-').any(|part| part == "nan" || part == "inf" || part == "infinity" || part.starts_with("nan") || part.starts_with("inf")) {
            return Err("Nonfinite SVG number".into());
        }
    }
    Ok(())
}

fn path_numbers(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut pos = 0;
    while pos < bytes.len() {
        if bytes[pos].is_ascii_whitespace() || bytes[pos] == b',' || b"MmZzLlHhVvCcSsQqTtAa".contains(&bytes[pos]) { pos += 1; continue; }
        let start = pos;
        if matches!(bytes[pos], b'+' | b'-') { pos += 1; }
        let mut digits = 0;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() { pos += 1; digits += 1; }
        if pos < bytes.len() && bytes[pos] == b'.' {
            pos += 1;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() { pos += 1; digits += 1; }
        }
        if digits == 0 { return false; }
        if pos < bytes.len() && matches!(bytes[pos], b'e' | b'E') {
            pos += 1;
            if pos < bytes.len() && matches!(bytes[pos], b'+' | b'-') { pos += 1; }
            let exponent = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() { pos += 1; }
            if pos == exponent { return false; }
        }
        if !finite(&value[start..pos]) { return false; }
    }
    true
}

fn escape(value: &str, output: &mut String) {
    for c in value.chars() {
        match c {
            '&' => output.push_str("&amp;"), '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"), '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"), '\t' => output.push_str("&#9;"),
            '\n' => output.push_str("&#10;"), '\r' => output.push_str("&#13;"),
            _ => output.push(c),
        }
    }
}

fn write_element(node: Node<'_, '_>, source: &str, rewrites: &HashMap<usize, String>, output: &mut String, root: bool) {
    // Keep the original qualified names; local-name-only rebuilding changes xlink,
    // prefixed SVG/XHTML, and namespace-sensitive CSS selector semantics.
    let name = source[node.range().start + 1..].split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or("");
    output.push('<'); output.push_str(name);
    if root && node.tag_name().namespace().is_none() { output.push_str(" xmlns=\"http://www.w3.org/2000/svg\""); }
    for ns in node.namespaces() {
        if node.parent_element().is_some_and(|parent| parent.lookup_namespace_uri(ns.name()) == Some(ns.uri())) { continue; }
        output.push_str(" xmlns");
        if let Some(prefix) = ns.name() { output.push(':'); output.push_str(prefix); }
        output.push_str("=\""); escape(ns.uri(), output); output.push('"');
    }
    for attr in node.attributes() {
        output.push(' '); output.push_str(&source[attr.range_qname()]);
        output.push_str("=\""); escape(rewrites.get(&attr.range_qname().start).map_or(attr.value(), String::as_str), output); output.push('"');
    }
    output.push('>');
    for child in node.children() {
        if child.is_element() { write_element(child, source, rewrites, output, false); }
        else if child.is_text() { escape(child.text().unwrap_or(""), output); }
    }
    output.push_str("</"); output.push_str(name); output.push('>');
}
