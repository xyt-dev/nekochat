// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Token-based CSS safety checks. Drawing grammar and cascade stay native.

use cssparser::{Delimiter, ParseError, Parser, ParserInput, Token};
use super::super::Checked;

#[derive(Default)]
pub(super) struct Usage {
    pub references: Vec<String>,
}

pub(super) fn value(source: &str) -> Checked<Usage> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut usage = Usage::default();
    scan(&mut parser, &mut usage, false, 0).map_err(|e| format!("Unsafe SVG CSS: {e:?}"))?;
    Ok(usage)
}

pub(super) fn declarations(source: &str) -> Checked<Usage> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut usage = Usage::default();
    while !parser.is_exhausted() {
        if parser.try_parse(|p| p.expect_semicolon()).is_ok() { continue; }
        parser.parse_until_before(Delimiter::Semicolon, |inner| {
            let name = inner.expect_ident()?.clone();
            if name.eq_ignore_ascii_case("behavior") || name.eq_ignore_ascii_case("-moz-binding") {
                return Err(inner.new_custom_error("Executable legacy CSS property".to_owned()));
            }
            inner.expect_colon()?;
            scan(inner, &mut usage, false, 0)
        }).map_err(|e: ParseError<'_, String>| format!("Invalid SVG style: {e:?}"))?;
        let _ = parser.try_parse(|p| p.expect_semicolon());
    }
    Ok(usage)
}

pub(super) fn stylesheet(source: &str) -> Checked<Usage> {
    let mut input = ParserInput::new(source);
    let mut usage = Usage::default();
    scan(&mut Parser::new(&mut input), &mut usage, true, 0)
        .map_err(|e| format!("Unsafe SVG stylesheet: {e:?}"))?;
    Ok(usage)
}

fn scan<'i>(parser: &mut Parser<'i, '_>, usage: &mut Usage, stylesheet: bool, depth: usize) -> Result<(), ParseError<'i, String>> {
    if depth > 32 { return Err(parser.new_custom_error("CSS nesting exceeds 32 levels".to_owned())); }
    while !parser.is_exhausted() {
        let token = parser.next()?.clone();
        if token.is_parse_error() { return Err(parser.new_custom_error("Malformed CSS token".to_owned())); }
        match token {
            Token::UnquotedUrl(url) => {
                if let Some(id) = super::resource_url(&url, true).map_err(|e| parser.new_custom_error(e))? {
                    usage.references.push(id);
                }
            }
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                parser.parse_nested_block(|inner| {
                    let url = inner.expect_string()?.clone();
                    inner.expect_exhausted()?;
                    if let Some(id) = super::resource_url(&url, true).map_err(|e| inner.new_custom_error(e))? {
                        usage.references.push(id);
                    }
                    Ok(())
                })?;
            }
            Token::Function(name) => {
                // These functions can interpret a bare string as a resource or defer
                // URL construction until computed-value time, beyond URL tokens.
                if ["image", "image-set", "-webkit-image-set", "src", "attr", "expression", "paint"].iter().any(|v| name.eq_ignore_ascii_case(v)) {
                    return Err(parser.new_custom_error(format!("Forbidden CSS function: {name}")));
                }
                let variable = name.eq_ignore_ascii_case("var");
                parser.parse_nested_block(|inner| {
                    if variable {
                        let name = inner.expect_ident()?;
                        if !name.starts_with("--") { return Err(inner.new_custom_error("Invalid CSS custom property".to_owned())); }
                    }
                    scan(inner, usage, false, depth + 1)
                })?;
            }
            Token::AtKeyword(name) if stylesheet && name.eq_ignore_ascii_case("namespace") => {
                // Namespace URIs identify selector namespaces; they never fetch resources.
                parser.parse_until_before(Delimiter::Semicolon, |inner| {
                    let first = inner.next()?.clone();
                    let uri = if matches!(first, Token::Ident(_)) { inner.next()?.clone() } else { first };
                    match uri {
                        Token::QuotedString(_) | Token::UnquotedUrl(_) => {}
                        Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                            inner.parse_nested_block(|url| {
                                url.expect_string()?;
                                url.expect_exhausted()?;
                                Ok(())
                            })?;
                        }
                        _ => return Err(inner.new_custom_error("Invalid CSS namespace declaration".to_owned())),
                    }
                    inner.expect_exhausted()?;
                    Ok(())
                })?;
                let _ = parser.try_parse(|p| p.expect_semicolon());
            }
            Token::AtKeyword(name) => {
                if !stylesheet || !["media", "supports", "layer", "container", "scope", "keyframes", "-webkit-keyframes", "starting-style", "font-face"].iter().any(|v| name.eq_ignore_ascii_case(v)) {
                    return Err(parser.new_custom_error(format!("Forbidden CSS at-rule: {name}")));
                }
            }
            Token::Ident(name) if name.eq_ignore_ascii_case("-moz-binding") || name.eq_ignore_ascii_case("behavior") => {
                return Err(parser.new_custom_error("Executable legacy CSS property".to_owned()));
            }
            Token::Number { value, .. } | Token::Dimension { value, .. } if !value.is_finite() => {
                return Err(parser.new_custom_error("Nonfinite CSS number".to_owned()));
            }
            Token::Percentage { unit_value, .. } if !unit_value.is_finite() => {
                return Err(parser.new_custom_error("Nonfinite CSS percentage".to_owned()));
            }
            Token::CurlyBracketBlock | Token::SquareBracketBlock | Token::ParenthesisBlock => {
                if !stylesheet && matches!(token, Token::CurlyBracketBlock) {
                    return Err(parser.new_custom_error("Unexpected CSS rule in a value".to_owned()));
                }
                parser.parse_nested_block(|inner| scan(inner, usage, stylesheet, depth + 1))?;
            }
            Token::Semicolon if !stylesheet => return Err(parser.new_custom_error("Unexpected CSS declaration in a value".to_owned())),
            _ => {}
        }
    }
    Ok(())
}
