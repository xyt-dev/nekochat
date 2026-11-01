// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Static SVG allowlist. Accepted XML is rebuilt; original markup is never returned.

use std::collections::HashMap;
use roxmltree::{Document, Node, ParsingOptions};
use super::Checked;

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const THEME: &[&str] = &[
    "bg", "bg-side", "bg-deep", "bg-raised", "surface", "surface-hover", "border",
    "text", "text-muted", "text-faint", "accent", "on-accent", "red", "orange",
    "yellow", "green", "teal", "blue", "purple", "pink",
];

/// Validates and rebuilds a bounded static SVG with no executable or external content.
pub fn sanitize_graphic_svg(source: &str) -> Checked<String> {
    if source.len() > 64 * 1024 { return Err("SVG exceeds 64 KiB".into()); }
    if source.contains("<!DOCTYPE") || source.contains("<!ENTITY") {
        return Err("SVG DTDs and entity declarations are forbidden".into());
    }
    let doc = Document::parse_with_options(source, ParsingOptions {
        allow_dtd: false, nodes_limit: 8192, ..ParsingOptions::default()
    }).map_err(|e| format!("Invalid SVG XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "svg" { return Err("Expected an SVG root element".into()); }
    let mut count = 0usize;
    let mut ids = HashMap::new();
    let mut references = Vec::new();
    let mut accessible_references = Vec::new();
    for node in doc.descendants() {
        if node.is_pi() { return Err("SVG processing instructions are forbidden".into()); }
        if node.is_text() {
            if node.text().unwrap_or("").chars().count() > 4000 { return Err("SVG text exceeds 4000 characters".into()); }
            continue;
        }
        if !node.is_element() { continue; }
        count += 1;
        if count > 1024 || node.ancestors().count() > 33 { return Err("SVG exceeds 1024 elements or 32 nesting levels".into()); }
        if node.tag_name().namespace().is_some_and(|ns| ns != SVG_NS) { return Err("Foreign SVG element namespace".into()); }
        let tag = node.tag_name().name();
        if ![
            "svg", "g", "defs", "path", "rect", "circle", "ellipse", "line", "polyline",
            "polygon", "text", "tspan", "title", "desc", "linearGradient", "radialGradient", "stop", "clipPath",
        ].contains(&tag) { return Err(format!("Forbidden SVG element: {tag}")); }
        if node.attributes().len() > 32 { return Err("Too many SVG attributes".into()); }
        for attr in node.attributes() {
            let key = attr.name();
            let value = attr.value().trim();
            if let Some(ns) = attr.namespace() {
                if ns == XML_NS && key == "space" && ["default", "preserve"].contains(&value) { continue; }
                return Err(format!("Forbidden SVG attribute namespace: {key}"));
            }
            if key == "id" {
                if !local_id(value) || ids.insert(value, tag).is_some() { return Err("SVG ids must be valid and unique".into()); }
                continue;
            }
            validate_attribute(tag, key, value)?;
            if ["aria-labelledby", "aria-describedby"].contains(&key) {
                accessible_references.extend(value.split_ascii_whitespace());
            }
            if ["fill", "stroke"].contains(&key) {
                if let Some(id) = fragment(value) { references.push((id, false)); }
            }
            if key == "clip-path" && value != "none" {
                if node.ancestors().any(|n| n.is_element() && n.tag_name().name() == "clipPath") {
                    return Err("Nested SVG clip references are forbidden".into());
                }
                references.push((fragment(value).ok_or("Invalid SVG clip reference")?, true));
            }
        }
    }
    for (id, clip) in references {
        match ids.get(id) {
            Some(&"clipPath") if clip => {},
            Some(&"linearGradient") | Some(&"radialGradient") if !clip => {},
            _ => return Err(format!("Invalid SVG fragment target: {id}")),
        }
    }
    for id in accessible_references {
        if !ids.contains_key(id) { return Err(format!("Unknown SVG accessibility target: {id}")); }
    }
    let mut output = String::with_capacity(source.len());
    write_element(root, &mut output, true);
    if output.len() > 64 * 1024 { return Err("Normalized SVG exceeds 64 KiB".into()); }
    Ok(output)
}

fn local_id(s: &str) -> bool {
    let mut bytes = s.bytes();
    bytes.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && bytes.all(|c| c.is_ascii_alphanumeric() || [b'_', b'-', b'.', b':'].contains(&c))
}

fn fragment(value: &str) -> Option<&str> {
    let id = value.strip_prefix("url(#")?.strip_suffix(')')?;
    local_id(id).then_some(id)
}

fn paint(value: &str, references: bool) -> bool {
    if ["none", "currentColor", "transparent"].contains(&value) { return true; }
    if let Some(hex) = value.strip_prefix('#') {
        return [3, 4, 6, 8].contains(&hex.len()) && hex.bytes().all(|c| c.is_ascii_hexdigit());
    }
    if let Some(inner) = value.strip_prefix("var(--nk-").and_then(|s| s.strip_suffix(')')) {
        let (token, fallback) = inner.split_once(',').map_or((inner, None), |(token, fallback)| (token, Some(fallback.trim())));
        return THEME.contains(&token.trim()) && fallback.is_none_or(|fallback| !fallback.starts_with("var(") && paint(fallback, false));
    }
    if references && fragment(value).is_some() { return true; }
    if [
        "black", "white", "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink",
        "gray", "grey", "silver", "maroon", "navy", "lime", "aqua", "fuchsia", "olive", "cyan", "magenta",
        "rebeccapurple", "gold", "coral", "tomato", "indigo", "violet", "brown", "beige", "lightgray", "darkgray",
    ].contains(&value) { return true; }
    for (name, count) in [("rgb(", 3), ("rgba(", 4), ("hsl(", 3), ("hsla(", 4)] {
        if let Some(inner) = value.strip_prefix(name).and_then(|s| s.strip_suffix(')')) {
            let mut parts = inner.split(',');
            for _ in 0..count {
                let Some(part) = parts.next() else { return false; };
                if !finite(part.trim().strip_suffix('%').unwrap_or(part.trim())) { return false; }
            }
            return parts.next().is_none();
        }
    }
    false
}

fn finite(value: &str) -> bool { value.parse::<f64>().is_ok_and(f64::is_finite) }

fn length(value: &str, negative: bool) -> bool {
    let value = ["rem", "px", "em", "ex", "%"].iter().find_map(|unit| value.strip_suffix(unit)).unwrap_or(value);
    value.parse::<f64>().is_ok_and(|n| n.is_finite() && (negative || n >= 0.0))
}

// Scans numeric SVG lists/path data without accepting CSS, URLs or nonfinite numbers.
fn numbers(value: &str, commands: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut pos = 0;
    let mut count = 0;
    while pos < bytes.len() {
        if bytes[pos].is_ascii_whitespace() || bytes[pos] == b',' { pos += 1; continue; }
        if commands.as_bytes().contains(&bytes[pos]) { pos += 1; continue; }
        let start = pos;
        if matches!(bytes[pos], b'+' | b'-') { pos += 1; }
        let mut digits = 0;
        while pos < bytes.len() && bytes[pos].is_ascii_digit() { pos += 1; digits += 1; }
        if pos < bytes.len() && bytes[pos] == b'.' {
            pos += 1;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() { pos += 1; digits += 1; }
        }
        if digits == 0 { return None; }
        if pos < bytes.len() && matches!(bytes[pos], b'e' | b'E') {
            pos += 1;
            if pos < bytes.len() && matches!(bytes[pos], b'+' | b'-') { pos += 1; }
            let exponent = pos;
            while pos < bytes.len() && bytes[pos].is_ascii_digit() { pos += 1; }
            if pos == exponent { return None; }
        }
        if !finite(&value[start..pos]) { return None; }
        count += 1;
    }
    Some(count)
}

fn transform(value: &str) -> bool {
    let mut remaining = value.trim();
    let mut operations = 0;
    while !remaining.is_empty() {
        operations += 1;
        if operations > 32 { return false; }
        let Some((name, after)) = remaining.split_once('(') else { return false; };
        let Some((args, rest)) = after.split_once(')') else { return false; };
        let Some(count) = numbers(args, "") else { return false; };
        if !match name.trim() {
            "matrix" => count == 6, "translate" | "scale" => [1, 2].contains(&count),
            "rotate" => [1, 3].contains(&count), "skewX" | "skewY" => count == 1,
            _ => false,
        } { return false; }
        remaining = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
    }
    operations > 0
}

fn validate_attribute(tag: &str, key: &str, value: &str) -> Checked<()> {
    let ok = match key {
        "fill" | "stroke" => paint(value, true),
        "color" => paint(value, false) && !value.starts_with("url("),
        "opacity" | "fill-opacity" | "stroke-opacity" => value.parse::<f64>().is_ok_and(|n| n.is_finite() && (0.0..=1.0).contains(&n)),
        "stroke-width" | "stroke-miterlimit" => length(value, false),
        "stroke-dashoffset" => length(value, true),
        "stroke-dasharray" => value == "none" || numbers(value, "").is_some_and(|n| n > 0) && value.split(|c: char| c.is_ascii_whitespace() || c == ',').filter(|s| !s.is_empty()).all(|s| s.parse::<f64>().is_ok_and(|v| v >= 0.0)),
        "stroke-linecap" => ["butt", "round", "square"].contains(&value),
        "stroke-linejoin" => ["miter", "round", "bevel"].contains(&value),
        "fill-rule" | "clip-rule" => ["nonzero", "evenodd"].contains(&value),
        "clip-path" => value == "none" || fragment(value).is_some(),
        "transform" => transform(value),
        "vector-effect" => ["none", "non-scaling-stroke"].contains(&value),
        "role" => ["img", "presentation"].contains(&value),
        "aria-label" | "aria-description" => value.chars().count() <= 4000,
        "aria-labelledby" | "aria-describedby" => !value.is_empty() && value.len() <= 4000 && value.split_ascii_whitespace().all(local_id),
        "viewBox" if tag == "svg" => {
            let parts: Vec<_> = value.split(|c: char| c.is_ascii_whitespace() || c == ',').filter(|s| !s.is_empty()).collect();
            parts.len() == 4 && parts.iter().all(|s| finite(s)) && parts[2].parse::<f64>().is_ok_and(|n| n > 0.0) && parts[3].parse::<f64>().is_ok_and(|n| n > 0.0)
        }
        "preserveAspectRatio" if tag == "svg" => {
            let mut parts = value.split_ascii_whitespace();
            let first = parts.next().unwrap_or("");
            ["none", "xMinYMin", "xMidYMin", "xMaxYMin", "xMinYMid", "xMidYMid", "xMaxYMid", "xMinYMax", "xMidYMax", "xMaxYMax"].contains(&first)
                && parts.next().is_none_or(|p| ["meet", "slice"].contains(&p)) && parts.next().is_none()
        }
        "width" | "height" if ["svg", "rect"].contains(&tag) => length(value, false),
        "x" | "y" if ["svg", "rect", "text", "tspan"].contains(&tag) => length(value, true),
        "rx" | "ry" if ["rect", "ellipse"].contains(&tag) => length(value, false),
        "cx" | "cy" if ["circle", "ellipse", "radialGradient"].contains(&tag) => length(value, true),
        "r" if ["circle", "radialGradient"].contains(&tag) => length(value, false),
        "x1" | "y1" | "x2" | "y2" if ["line", "linearGradient"].contains(&tag) => length(value, true),
        "fx" | "fy" | "fr" if tag == "radialGradient" => length(value, key != "fr"),
        "d" if tag == "path" => numbers(value, "MmZzLlHhVvCcSsQqTtAa").is_some(),
        "pathLength" if ["path", "rect", "circle", "ellipse", "line", "polyline", "polygon"].contains(&tag) => value.parse::<f64>().is_ok_and(|n| n.is_finite() && n > 0.0),
        "points" if ["polyline", "polygon"].contains(&tag) => numbers(value, "").is_some_and(|n| n >= 2 && n % 2 == 0),
        "dx" | "dy" if ["text", "tspan"].contains(&tag) => length(value, true),
        "font-size" | "letter-spacing" | "word-spacing" if ["text", "tspan", "g", "svg"].contains(&tag) => length(value, key != "font-size"),
        "font-family" if ["text", "tspan", "g", "svg"].contains(&tag) => value.len() <= 256 && value.bytes().all(|c| c.is_ascii_alphanumeric() || [b' ', b'-', b'_', b','].contains(&c)),
        "font-weight" => ["normal", "bold", "100", "200", "300", "400", "500", "600", "700", "800", "900"].contains(&value),
        "font-style" => ["normal", "italic", "oblique"].contains(&value),
        "text-anchor" => ["start", "middle", "end"].contains(&value),
        "dominant-baseline" => ["auto", "middle", "central", "hanging", "alphabetic", "text-before-edge", "text-after-edge"].contains(&value),
        "gradientUnits" if ["linearGradient", "radialGradient"].contains(&tag) => ["userSpaceOnUse", "objectBoundingBox"].contains(&value),
        "gradientTransform" if ["linearGradient", "radialGradient"].contains(&tag) => transform(value),
        "spreadMethod" if ["linearGradient", "radialGradient"].contains(&tag) => ["pad", "reflect", "repeat"].contains(&value),
        "offset" if tag == "stop" => value.strip_suffix('%').map_or_else(
            || value.parse::<f64>().is_ok_and(|n| n.is_finite() && (0.0..=1.0).contains(&n)),
            |v| v.parse::<f64>().is_ok_and(|n| n.is_finite() && (0.0..=100.0).contains(&n))),
        "stop-color" if tag == "stop" => paint(value, false),
        "stop-opacity" if tag == "stop" => value.parse::<f64>().is_ok_and(|n| n.is_finite() && (0.0..=1.0).contains(&n)),
        "clipPathUnits" if tag == "clipPath" => ["userSpaceOnUse", "objectBoundingBox"].contains(&value),
        _ => false,
    };
    if ok { Ok(()) } else { Err(format!("Forbidden or invalid SVG attribute: {key}")) }
}

fn escape(value: &str, output: &mut String) {
    for c in value.chars() {
        match c {
            '&' => output.push_str("&amp;"), '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"), '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"), _ => output.push(c),
        }
    }
}

fn write_element(node: Node<'_, '_>, output: &mut String, root: bool) {
    output.push('<'); output.push_str(node.tag_name().name());
    if root { output.push_str(" xmlns=\"http://www.w3.org/2000/svg\""); }
    for attr in node.attributes() {
        output.push(' ');
        if attr.namespace() == Some(XML_NS) { output.push_str("xml:"); }
        output.push_str(attr.name()); output.push_str("=\""); escape(attr.value().trim(), output); output.push('"');
    }
    output.push('>');
    for child in node.children() {
        if child.is_element() { write_element(child, output, false); }
        else if child.is_text() { escape(child.text().unwrap_or(""), output); }
    }
    output.push_str("</"); output.push_str(node.tag_name().name()); output.push('>');
}
