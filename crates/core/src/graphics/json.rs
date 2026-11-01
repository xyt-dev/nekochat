// SPDX-License-Identifier: Apache-2.0 OR MIT
//! JSON decoding that also rejects duplicate object keys instead of silently overwriting them.

use std::fmt;
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

pub(super) struct UniqueValue(pub Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct JsonVisitor;
        impl<'de> Visitor<'de> for JsonVisitor {
            type Value = UniqueValue;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("JSON without duplicate object keys")
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Null)) }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Bool(v))) }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Number(v.into()))) }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> { Ok(UniqueValue(Value::Number(v.into()))) }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Number::from_f64(v).map(|n| UniqueValue(Value::Number(n))).ok_or_else(|| E::custom("Nonfinite JSON number"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> { Ok(UniqueValue(Value::String(v.into()))) }
            fn visit_string<E: de::Error>(self, v: String) -> Result<Self::Value, E> { Ok(UniqueValue(Value::String(v))) }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(UniqueValue(v)) = seq.next_element()? { values.push(v); }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut values = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) { return Err(de::Error::custom(format!("Duplicate JSON field: {key}"))); }
                    let UniqueValue(v) = map.next_value()?;
                    values.insert(key, v);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(JsonVisitor)
    }
}

pub(super) fn complete_containers(source: &str) -> Option<String> {
    let mut closers = [0u8; 128];
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for byte in source.bytes() {
        if quoted {
            if escaped { escaped = false; }
            else if byte == b'\\' { escaped = true; }
            else if byte == b'"' { quoted = false; }
        } else {
            match byte {
                b'"' => quoted = true,
                b'[' | b'{' => {
                    if depth == closers.len() { return None; }
                    closers[depth] = if byte == b'[' { b']' } else { b'}' };
                    depth += 1;
                }
                b']' | b'}' => {
                    if depth == 0 || closers[depth - 1] != byte { return None; }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }
    if quoted || depth == 0 { return None; }
    let mut completed = String::with_capacity(source.len() + depth);
    completed.push_str(source);
    for &closer in closers[..depth].iter().rev() { completed.push(char::from(closer)); }
    Some(completed)
}
