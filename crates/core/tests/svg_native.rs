use nekochat_core::graphics::{sanitize_graphic_svg, validate_graphic_json};
use serde_json::{Value, json};

fn checked(source: &str) -> String {
    let safe = sanitize_graphic_svg(source).unwrap_or_else(|error| panic!("{error}\n{source}"));
    let embedded: Value = serde_json::from_str(&validate_graphic_json(&json!({
        "version": 1, "items": [{"type": "svg", "code": source}]
    }).to_string()).unwrap()).unwrap();
    assert_eq!(embedded["items"][0]["code"], safe);
    // Rebuilding must not change semantics or make a second validation unsafe.
    assert_eq!(sanitize_graphic_svg(&safe).unwrap(), safe);
    safe
}

#[test]
fn euler_theme_alias_survives_both_public_svg_consumers() {
    let safe = checked(include_str!("fixtures/svg-euler.svg"));
    let doc = roxmltree::Document::parse(&safe).unwrap();
    assert_eq!(doc.descendants().find(|n| n.has_tag_name("text")).unwrap().attribute("fill"), Some("var(--nk-muted)"));
    assert_eq!(doc.descendants().find(|n| n.has_tag_name("linearGradient")).unwrap().attribute("gradientUnits"), Some("userSpaceOnUse"));
    assert!(doc.descendants().any(|n| n.attribute("stroke") == Some("url(#vec)")));
}

#[test]
fn native_drawing_resources_typography_and_all_path_commands_are_preserved() {
    let safe = checked(r##"<svg viewBox="0 0 320 200" xmlns:xlink="http://www.w3.org/1999/xlink">
      <defs>
        <path id="curve" d="M0 0 l20 20 H50 v10 C50 0 80 0 80 30 s20 20 30 0 Q120 0 140 20 t30 0 A10 20 30 0 1 200 50 z"/>
        <linearGradient id="base"><stop stop-color="papayawhip"/><stop offset="100%" stop-color="oklch(70% .2 160)"/></linearGradient>
        <radialGradient id="radial" xlink:href="#base" gradientTransform="rotate(15)"/>
        <pattern id="tiles" width="10" height="10" patternUnits="userSpaceOnUse"><rect width="5" height="5" fill="color(display-p3 1 .4 0)"/></pattern>
        <mask id="fade"><rect width="100%" height="100%" fill="white"/></mask>
        <clipPath id="clip"><circle r="80"/></clipPath>
        <marker id="arrow" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0L10 5L0 10z" fill="context-stroke"/></marker>
        <filter id="effect"><feGaussianBlur stdDeviation="1 2" result="blur"/><feColorMatrix type="saturate" values=".7"/><feBlend in="SourceGraphic" in2="blur" mode="multiply"/><feComposite operator="in" in2="SourceAlpha"/><feTurbulence baseFrequency=".05" numOctaves="2"/><feDisplacementMap scale="2"/><feMerge><feMergeNode in="SourceGraphic"/></feMerge></filter>
        <symbol id="star" viewBox="0 0 20 20"><polygon points="0,0 20,10 0,20"/></symbol>
      </defs>
      <use xlink:href="#star" x="10" width="20" height="20" fill="url('#radial')"/>
      <path d="M0 0L100 100" stroke="url(#tiles)" marker-end="url(#arrow)" mask="url(#fade)" clip-path="url(#clip)" filter="url(#effect)"/>
      <text x="1em 20 30" y="2ex" rotate="0 10 20" font-family="'Noto Sans', sans-serif" font-variation-settings="'wght' 550" textLength="100" lengthAdjust="spacingAndGlyphs"><textPath href="#curve" startOffset="10%"><tspan dx="1 2" dy=".1em .2em">Native text</tspan></textPath></text>
    </svg>"##);
    let doc = roxmltree::Document::parse(&safe).unwrap();
    assert_eq!(doc.descendants().find(|n| n.has_tag_name("use")).unwrap().attribute(("http://www.w3.org/1999/xlink", "href")), Some("#star"));
    assert!(doc.descendants().any(|n| n.has_tag_name("feTurbulence")));
    assert_eq!(doc.descendants().find(|n| n.has_tag_name("textPath")).unwrap().attribute("startOffset"), Some("10%"));
}

#[test]
fn safe_css_custom_properties_selectors_keyframes_and_smil_remain_native() {
    let safe = checked(r##"<svg viewBox="0 0 100 100"><style>
      :root { --ink: var(--nk-muted, lightseagreen); }
      @media (min-width: 1px) { .label > tspan:first-child { fill: var(--ink); font: italic 12px 'Noto Sans'; } }
      @supports (color: oklch(50% .1 20)) { #dot { color: oklch(50% .1 20); } }
      @keyframes pulse { from { opacity: .3; } to { opacity: 1; } }
      #dot { animation: pulse 2s infinite; fill: url(\23 paint); }
      </style><defs><linearGradient id="paint"><stop stop-color="lightgoldenrodyellow"/></linearGradient><path id="motion" d="M0 0L50 50"/></defs>
      <circle id="dot" r="5" style="stroke: var(--nk-text); stroke-width: calc(1px + 1px)">
        <animate attributeName="r" values="5;10;5" dur="2s" repeatCount="indefinite"/>
        <animate attributeName="fill" values="var(--nk-muted);hsl(120 50% 50% / .5)" dur="2s"/>
        <animateTransform attributeName="transform" type="rotate" from="0" to="360" dur="3s"/>
        <animateMotion dur="3s"><mpath href="#motion"/></animateMotion>
        <set attributeName="opacity" to=".5" begin="1s" dur="1s"/>
      </circle><text class="label"><tspan>Styled</tspan></text></svg>"##);
    let doc = roxmltree::Document::parse(&safe).unwrap();
    assert!(doc.descendants().find(|n| n.has_tag_name("style")).unwrap().text().unwrap().contains("@keyframes pulse"));
    assert_eq!(doc.descendants().find(|n| n.has_tag_name("animateMotion")).unwrap().attribute("dur"), Some("3s"));
    assert_eq!(doc.descendants().filter(|n| n.has_tag_name("animate")).count(), 2);
}

#[test]
fn raster_data_and_inert_xhtml_keep_their_namespaces_and_presentation() {
    let png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+j2ioAAAAASUVORK5CYII=";
    let safe = checked(&format!(r##"<s:svg xmlns:s="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100"><s:image xlink:href="{png}" width="10" height="10"/><s:foreignObject width="100" height="80"><h:div xmlns:h="http://www.w3.org/1999/xhtml" style="display:flex; gap:2px; color:var(--nk-muted)"><h:p lang="zh">安全 <h:strong>排版</h:strong></h:p><h:img src="{png}" alt="pixel"/></h:div></s:foreignObject></s:svg>"##));
    let doc = roxmltree::Document::parse(&safe).unwrap();
    let html = doc.descendants().find(|n| n.tag_name().name() == "div").unwrap();
    assert_eq!(html.tag_name().namespace(), Some("http://www.w3.org/1999/xhtml"));
    assert!(safe.starts_with("<s:svg"));
    assert!(doc.descendants().any(|n| n.attribute("src") == Some(png)));
}

#[test]
fn embedded_fonts_preserve_local_css_and_reject_disguised_active_data() {
    let font = include_str!("fixtures/svg-font.woff.base64").trim();
    let safe = checked(&format!(r#"<svg><style>@font-face {{font-family:fixture;src:url(data:font/woff;base64,{font}) format('woff')}} text {{font-family:fixture}}</style><text>A</text></svg>"#));
    let doc = roxmltree::Document::parse(&safe).unwrap();
    assert!(doc.descendants().find(|n| n.has_tag_name("style")).unwrap().text().unwrap().contains(font));
    for url in ["data:font/woff;base64,PHN2Zy8+", "data:image/png;base64,PHN2Zy8+", "data:text/css;base64,Ym9keXt9"] {
        assert!(sanitize_graphic_svg(&format!(r#"<svg><style>@font-face {{font-family:x;src:url({url})}}</style></svg>"#)).is_err());
    }
}

fn nested_image(source: &str, base64: bool) -> String {
    let payload = if base64 {
        // Test encoding stays independent from the production encoder.
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut encoded = String::new();
        for chunk in source.as_bytes().chunks(3) {
            let word = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
            encoded.push(ALPHABET[((word >> 18) & 63) as usize] as char);
            encoded.push(ALPHABET[((word >> 12) & 63) as usize] as char);
            encoded.push(if chunk.len() > 1 { ALPHABET[((word >> 6) & 63) as usize] as char } else { '=' });
            encoded.push(if chunk.len() > 2 { ALPHABET[(word & 63) as usize] as char } else { '=' });
        }
        format!("data:image/svg+xml;base64,{encoded}")
    } else {
        let encoded: String = source.bytes().map(|byte| format!("%{byte:02X}")).collect();
        format!("data:image/svg+xml;charset=utf-8,{encoded}")
    };
    format!(r#"<svg><image href="{payload}" width="20" height="20"/></svg>"#)
}

#[test]
fn nested_svg_data_images_are_rebuilt_through_the_same_security_boundary() {
    for base64 in [false, true] {
        let safe = checked(&nested_image(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><title>内嵌图</title><circle cx="10" cy="10" r="8" fill="papayawhip"/></svg>"#, base64));
        let doc = roxmltree::Document::parse(&safe).unwrap();
        let expected = nested_image(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 20 20"><title>内嵌图</title><circle cx="10" cy="10" r="8" fill="papayawhip"></circle></svg>"#, true);
        let expected = roxmltree::Document::parse(&expected).unwrap();
        assert_eq!(doc.descendants().find(|n| n.has_tag_name("image")).unwrap().attribute("href"), expected.descendants().find(|n| n.has_tag_name("image")).unwrap().attribute("href"));
        for nested in [
            "<svg><script>alert(1)</script></svg>",
            "<svg><image href=\"https://example.com/x\"/></svg>",
            "<!DOCTYPE svg [<!ENTITY x 'bad'>]><svg>&x;</svg>",
            "<svg><g id=\"a\"><use href=\"#a\"/></g></svg>",
        ] { assert!(sanitize_graphic_svg(&nested_image(nested, base64)).is_err(), "{nested}"); }
    }
    let mut nested = "<svg><rect/></svg>".to_owned();
    for _ in 0..10 { nested = nested_image(&nested, true); }
    assert!(sanitize_graphic_svg(&nested).is_err());
    for href in ["data:image/svg+xml;base64,PHN2Zy8+====", "data:image/svg+xml,%GG", "data:image/svg+xml;charset=windows-1252,%3Csvg/%3E"] {
        assert!(sanitize_graphic_svg(&format!(r#"<svg><image href="{href}"/></svg>"#)).is_err());
    }
    let mut many = String::from("<svg>");
    for _ in 0..400 { many.push_str("<rect/>"); }
    many.push_str("</svg>");
    let image = nested_image(&many, true);
    // Each embedded document fits alone; aggregate element count must still hold.
    let image = image.strip_prefix("<svg>").unwrap().strip_suffix("</svg>").unwrap();
    assert!(sanitize_graphic_svg(&format!("<svg>{image}{image}{image}</svg>")).is_err());
}

#[test]
fn executable_network_and_namespace_bypasses_are_rejected() {
    for body in [
        r#"<script>alert(1)</script>"#,
        r#"<rect ONCLICK="alert(1)"/>"#,
        r#"<image href="&#x6a;avascript:alert(1)"/>"#,
        r#"<image href="data:text/html;base64,PHNjcmlwdC8+"/>"#,
        r#"<image href="//example.com/pixel"/>"#,
        r#"<use xmlns:xlink="http://www.w3.org/1999/xlink" xlink:href="https://example.com/a.svg#x"/>"#,
        r##"<g xml:base="https://example.com/"><image href="#x"/></g>"##,
        r#"<style>@\69mport "https://example.com/a.css";</style>"#,
        r#"<style>rect { fill: u\72l(https://example.com/x); }</style>"#,
        r#"<rect style="fill:URL('https://example.com/x')"/>"#,
        r#"<rect fill="var(--x, url(https://example.com/x))"/>"#,
        r#"<style>rect { background-image: image-set('https://example.com/x' 1x); }</style>"#,
        r#"<style>@font-face { font-family:x; src:local(x),url(https://example.com/font); }</style>"#,
        r#"<rect style="--x: url(https://example.com/x); fill: var(--x)"/>"#,
        r#"<style>rect{fill:red}<!-- separator -->rect{fill:url(https://example.com/x)}</style>"#,
        r#"<style>u<!-- separator -->rl(https://example.com/x){fill:red}</style>"#,
        r##"<animate attributeName="href" values="#x;javascript:alert(1)"/>"##,
        r#"<set attributeName="onload" to="alert(1)"/>"#,
        r#"<set attributeName="xml:base" to="https://example.com"/>"#,
        r#"<set attributeName="style" to="fill:url(https://example.com)"/>"#,
        r#"<feImage href="https://example.com/x.png"/>"#,
        r#"<foreignObject><div xmlns="http://www.w3.org/1999/xhtml"><iframe srcdoc="&lt;script>1&lt;/script>"/></div></foreignObject>"#,
        r#"<foreignObject><div xmlns="http://www.w3.org/1999/xhtml"><img srcset="https://example.com/x 1x"/></div></foreignObject>"#,
        r#"<foreignObject><div xmlns="http://www.w3.org/1999/xhtml" onclick="alert(1)"/></foreignObject>"#,
        r#"<foreignObject><div xmlns="urn:attacker"/></foreignObject>"#,
        r#"<x:script xmlns:x="urn:attacker"/>"#,
        r##"<g xmlns:x="urn:attacker" x:href="#x"/>"##,
        r#"<g xmlns=""/>"#,
        r#"<style><?xml-stylesheet href="https://example.com/a"?></style>"#,
    ] {
        let source = format!(r#"<svg xmlns="http://www.w3.org/2000/svg">{body}</svg>"#);
        assert!(sanitize_graphic_svg(&source).is_err(), "accepted {body}");
    }
    for source in [
        r#"<?xml-stylesheet href="https://example.com/a"?><svg/>"#,
        r#"<!DOCTYPE svg [<!ENTITY x "bad">]><svg>&x;</svg>"#,
    ] { assert!(sanitize_graphic_svg(source).is_err()); }
}

#[test]
fn missing_duplicate_and_cyclic_local_resources_are_rejected() {
    for body in [
        r##"<use href="#missing"/>"##,
        r##"<rect fill="url('#missing')"/>"##,
        r##"<rect style="fill:u\72l(\23 missing)"/>"##,
        r##"<path id="x"/><circle id="x"/>"##,
        r##"<g id="x"><use href="#x"/></g>"##,
        r##"<g id="a"><use href="#b"/></g><g id="b"><use href="#a"/></g>"##,
        r##"<linearGradient id="a" href="#b"/><linearGradient id="b" href="#a"/>"##,
        r##"<pattern id="a" href="#b"/><pattern id="b" href="#a"/>"##,
        r##"<filter id="f"><feImage href="#f"/></filter>"##,
    ] {
        let source = format!("<svg>{body}</svg>");
        assert!(sanitize_graphic_svg(&source).is_err(), "accepted {body}");
    }
    checked(r##"<svg><defs><g id="leaf"><rect width="1" height="1"/></g></defs><use href="#leaf"/><use href="#leaf"/></svg>"##);
    checked(r##"<svg><style>* { fill: url('#g'); }</style><defs><linearGradient id="g"><stop stop-color="gold"/></linearGradient></defs><rect width="10" height="10"/></svg>"##);
    checked(r##"<svg fill="url(#p)"><defs><pattern id="p"><rect width="1" height="1" fill="red"/></pattern></defs><rect width="10" height="10"/></svg>"##);
    checked(r##"<svg><style>.tile {fill:url(#p)} .inside {fill:oklch(60% .2 40)}</style><defs><pattern id="p"><rect class="inside"/></pattern></defs><rect class="tile"/></svg>"##);
}

#[test]
fn svg_limits_and_nonfinite_numbers_remain_bounded() {
    for body in [
        "<rect width=\"NaN\"/>", "<circle r=\"inf\"/>",
        "<path d=\"M0 0 L1e999 2\"/>", "<g transform=\"translate(1e999)\"/>",
        "<rect style=\"opacity:1e999\"/>", "<svg viewBox=\"0 0 0 10\"/>",
    ] { assert!(sanitize_graphic_svg(&format!("<svg>{body}</svg>")).is_err(), "{body}"); }
    assert!(sanitize_graphic_svg(&format!("<svg>{}</svg>", "<g/>".repeat(1024))).is_err());
    assert!(sanitize_graphic_svg(&format!("<svg>{}{}</svg>", "<g>".repeat(33), "</g>".repeat(33))).is_err());
    assert!(sanitize_graphic_svg(&format!("<svg><desc>{}</desc></svg>", "x".repeat(64 * 1024))).is_err());
    let mut expansion = String::from("<svg><defs><g id=\"n0\"><rect/></g>");
    for level in 1..20 {
        expansion.push_str(&format!("<g id=\"n{level}\"><use href=\"#n{}\"/><use href=\"#n{}\"/></g>", level - 1, level - 1));
    }
    expansion.push_str("</defs><use href=\"#n19\"/></svg>");
    assert!(sanitize_graphic_svg(&expansion).is_err());
}

#[test]
fn native_timing_namespaces_and_attribute_whitespace_survive_rebuilding() {
    checked(r#"<svg><circle r="5"><animate attributeName="r" values="5;10" keyTimes="0;1" keySplines=".2 .4 .6 .8" calcMode="spline" dur="2s"/><animateMotion keyPoints="0;.5;1" keyTimes="0;.5;1" path="M0 0L20 20" dur="2s"/></circle></svg>"#);
    for namespace in [r#""http://www.w3.org/2000/svg""#, r#"url("http://www.w3.org/2000/svg")"#] {
        checked(&format!(r#"<svg><style>@namespace s {namespace}; s|rect {{fill:red}}</style><rect width="10" height="10"/></svg>"#));
    }
    let safe = checked(r#"<svg aria-label="A&#10;B&#9;C&#13;D"><text style="--label:'A&#9;B'">Text</text></svg>"#);
    let doc = roxmltree::Document::parse(&safe).unwrap();
    assert_eq!(doc.root_element().attribute("aria-label"), Some("A\nB\tC\rD"));
}
