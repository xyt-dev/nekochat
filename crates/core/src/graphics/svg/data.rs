// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Nested image data transport. The decoded XML reenters the SVG validator.

use super::super::Checked;

pub(super) fn svg_source(value: &str) -> Checked<Option<String>> {
    let Some((header, payload)) = value.trim().split_once(',') else { return Ok(None); };
    let mut parts = header.split(';');
    if !parts.next().is_some_and(|mime| mime.eq_ignore_ascii_case("data:image/svg+xml")) { return Ok(None); }
    let mut base64 = false;
    for part in parts {
        if part.eq_ignore_ascii_case("base64") && !base64 { base64 = true; }
        else if !part.eq_ignore_ascii_case("charset=utf-8") && !part.eq_ignore_ascii_case("utf8") {
            return Err("Unsupported embedded SVG data encoding".into());
        }
    }
    let decoded = super::decode_fragment(payload)?;
    if base64 {
        String::from_utf8(decode_base64(&decoded)?).map(Some).map_err(|_| "Embedded SVG must be UTF-8".into())
    } else { Ok(Some(decoded)) }
}

fn decode_base64(value: &str) -> Checked<Vec<u8>> {
    let encoded_len = value.bytes().filter(|byte| !byte.is_ascii_whitespace()).count();
    if encoded_len == 0 || encoded_len % 4 != 0 { return Err("Invalid embedded SVG base64".into()); }
    let mut output = Vec::with_capacity(encoded_len / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0;
    let mut padding = 0;
    for byte in value.bytes() {
        if byte.is_ascii_whitespace() { continue; }
        if byte == b'=' { padding += 1; continue; }
        if padding != 0 { return Err("Invalid embedded SVG base64 padding".into()); }
        let digit = match byte {
            b'A'..=b'Z' => byte - b'A', b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52, b'+' => 62, b'/' => 63,
            _ => return Err("Invalid embedded SVG base64 character".into()),
        };
        accumulator = (accumulator << 6) | u32::from(digit);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
        }
    }
    if padding > 2 || bits != padding * 2 || accumulator & ((1 << bits) - 1) != 0 {
        return Err("Invalid embedded SVG base64 padding".into());
    }
    Ok(output)
}

pub(super) fn encode_base64(value: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(value.len().div_ceil(3) * 4);
    for chunk in value.chunks(3) {
        let word = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        output.push(ALPHABET[((word >> 18) & 63) as usize] as char);
        output.push(ALPHABET[((word >> 12) & 63) as usize] as char);
        output.push(if chunk.len() > 1 { ALPHABET[((word >> 6) & 63) as usize] as char } else { '=' });
        output.push(if chunk.len() > 2 { ALPHABET[(word & 63) as usize] as char } else { '=' });
    }
    output
}
