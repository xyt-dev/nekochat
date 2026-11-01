//! Bounded, channel-independent inline graphics validation and plot sampling.
// SPDX-License-Identifier: Apache-2.0 OR MIT

use std::collections::HashSet;
use serde::Serialize;
use serde_json::{Map, Value};

mod expression;
mod json;
mod svg;

pub use svg::sanitize_graphic_svg;

const JSON_BYTES: usize = 128 * 1024;
type Object = Map<String, Value>;
type Checked<T> = Result<T, String>;

fn object(value: &Value) -> Checked<&Object> {
    value.as_object().ok_or_else(|| "Expected an object".into())
}

fn fields(obj: &Object, allowed: &[&str]) -> Checked<()> {
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("Unknown field: {key}"));
        }
    }
    Ok(())
}

fn required<'a>(obj: &'a Object, key: &str) -> Checked<&'a Value> {
    obj.get(key).ok_or_else(|| format!("Missing field: {key}"))
}

fn text(value: &Value) -> Checked<&str> {
    let s = value.as_str().ok_or("Expected a string")?;
    if s.chars().count() > 4000 { return Err("Text exceeds 4000 characters".into()); }
    Ok(s)
}

fn string<'a>(obj: &'a Object, key: &str) -> Checked<&'a str> {
    text(required(obj, key)?)
}

fn optional_text(obj: &Object, key: &str) -> Checked<()> {
    if let Some(v) = obj.get(key) { text(v)?; }
    Ok(())
}

fn number(value: &Value) -> Checked<f64> {
    value.as_f64().filter(|v| v.is_finite()).ok_or_else(|| "Expected a finite number".into())
}

fn num(obj: &Object, key: &str) -> Checked<f64> { number(required(obj, key)?) }

fn optional_number(obj: &Object, key: &str) -> Checked<Option<f64>> {
    obj.get(key).map(number).transpose()
}

fn range(obj: &Object, key: &str, min: f64, max: f64, integer: bool) -> Checked<()> {
    if let Some(value) = optional_number(obj, key)? {
        if value < min || value > max || (integer && value.fract() != 0.0) {
            return Err(format!("{key} must be between {min} and {max}"));
        }
    }
    Ok(())
}

fn array<'a>(obj: &'a Object, key: &str, max: usize, nonempty: bool) -> Checked<&'a [Value]> {
    let values = required(obj, key)?.as_array().ok_or_else(|| format!("{key} must be an array"))?;
    if values.len() > max || (nonempty && values.is_empty()) {
        return Err(format!("{key} has an invalid item count (maximum {max})"));
    }
    Ok(values)
}

fn color(obj: &Object) -> Checked<()> {
    if let Some(value) = obj.get("color") {
        if !safe_color(text(value)?) { return Err("Invalid graphic color".into()); }
    }
    Ok(())
}

fn safe_color(s: &str) -> bool {
    ["accent", "text", "muted", "surface", "border", "red", "orange", "yellow", "green", "teal", "blue", "purple", "pink"].contains(&s)
        || s.strip_prefix('#').is_some_and(|h| [3, 6, 8].contains(&h.len()) && h.bytes().all(|c| c.is_ascii_hexdigit()))
}

fn parse(source: &str) -> Checked<Value> {
    if source.len() > JSON_BYTES { return Err("Graphic JSON exceeds 128 KiB".into()); }
    serde_json::from_str::<json::UniqueValue>(source).map(|value| value.0).map_err(|e| format!("Invalid graphic JSON: {e}"))
}

/// Strictly validates a v1 graphic and returns its normalized JSON representation.
pub fn validate_graphic_json(source: &str) -> Checked<String> {
    let mut value = parse(source)?;
    let obj = object(&value)?;
    fields(obj, &["version", "title", "gap", "items"])?;
    if required(obj, "version")?.as_u64() != Some(1) { return Err("Graphic version must be 1".into()); }
    optional_text(obj, "title")?;
    range(obj, "gap", 0.0, 32.0, false)?;
    array(obj, "items", 128, false)?;
    let mut budget = Budget { nodes: 0, ids: HashSet::new() };
    for node in value["items"].as_array_mut().expect("validated items") {
        validate_node(node, 1, &mut budget)?;
    }
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

struct Budget { nodes: usize, ids: HashSet<String> }

fn validate_node(value: &mut Value, depth: usize, budget: &mut Budget) -> Checked<()> {
    budget.nodes += 1;
    if budget.nodes > 128 || depth > 8 { return Err("Graphic exceeds 128 nodes or 8 nesting levels".into()); }
    let obj = object(value)?;
    let kind = string(obj, "type")?;
    let extra: &[&str] = match kind {
        "text" => &["content", "size", "color"],
        "stat" => &["label", "value", "delta", "color"],
        "choices" => &["options"],
        "row" | "col" | "card" => &["items", "gap", "color"],
        "grid" => &["items", "gap", "cols", "color"],
        "chart" => &["kind", "labels", "series", "height"],
        "plot" => &["xMin", "xMax", "yMin", "yMax", "height", "params", "series"],
        "diagram" => &["width", "height", "nodes", "edges"],
        "svg" => &["code", "height"],
        _ => return Err(format!("Unknown graphic node: {kind}")),
    };
    for key in obj.keys() {
        if !["type", "id", "title", "span"].contains(&key.as_str()) && !extra.contains(&key.as_str()) {
            return Err(format!("Unknown {kind} field: {key}"));
        }
    }
    optional_text(obj, "title")?;
    if let Some(id) = obj.get("id") {
        let id = text(id)?;
        if id.is_empty() || !budget.ids.insert(id.to_owned()) { return Err("Graphic ids must be nonempty and unique".into()); }
    }
    range(obj, "span", 1.0, 4.0, true)?;
    range(obj, "height", 120.0, 800.0, false)?;
    color(obj)?;
    match kind {
        "text" => {
            string(obj, "content")?;
            if let Some(size) = obj.get("size") {
                if !["body", "h2", "h3", "muted"].contains(&text(size)?) { return Err("Invalid text size".into()); }
            }
        }
        "stat" => {
            string(obj, "label")?;
            let v = required(obj, "value")?;
            if v.is_string() { text(v)?; } else { number(v)?; }
            optional_text(obj, "delta")?;
        }
        "choices" => {
            let options = array(obj, "options", 6, true)?;
            for (index, option) in options.iter().enumerate() {
                let option = object(option)?;
                fields(option, &["label", "prompt"])?;
                for (key, max) in [("label", 80), ("prompt", 500)] {
                    let value = required(option, key)?.as_str().ok_or("Choice text must be a string")?;
                    if value.trim().is_empty() || value.chars().count() > max {
                        return Err(format!("Choice {key} must contain 1–{max} characters"));
                    }
                    if options[..index].iter().any(|previous| previous[key].as_str().is_some_and(|previous| previous.trim() == value.trim())) {
                        return Err(format!("Duplicate choice {key}"));
                    }
                }
            }
        }
        "row" | "col" | "grid" | "card" => {
            range(obj, "gap", 0.0, 32.0, false)?;
            range(obj, "cols", 1.0, 4.0, true)?;
            array(obj, "items", 128, false)?;
            for child in value["items"].as_array_mut().expect("validated children") {
                validate_node(child, depth + 1, budget)?;
            }
        }
        "chart" => validate_chart(obj)?,
        "plot" => { compile_plot(obj)?; }
        "diagram" => validate_diagram(obj)?,
        "svg" => {
            let code = required(obj, "code")?.as_str().ok_or("SVG code must be a string")?;
            let safe = sanitize_graphic_svg(code)?;
            value["code"] = Value::String(safe);
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn validate_chart(obj: &Object) -> Checked<()> {
    let kind = string(obj, "kind")?;
    if !["line", "bar", "area", "donut"].contains(&kind) { return Err("Invalid chart kind".into()); }
    let labels = array(obj, "labels", 2048, true)?;
    for label in labels { text(label)?; }
    let series = array(obj, "series", 8, true)?;
    if kind == "donut" && series.len() != 1 { return Err("Donut charts require one series".into()); }
    for series in series {
        let series = object(series)?;
        fields(series, &["label", "values", "color"])?;
        string(series, "label")?;
        color(series)?;
        let values = array(series, "values", 2048, true)?;
        if values.len() != labels.len() { return Err("Chart series must match labels".into()); }
        let mut sum = 0.0;
        for value in values {
            let n = number(value)?;
            if kind == "donut" && n < 0.0 { return Err("Donut values must be nonnegative".into()); }
            sum += n;
        }
        if kind == "donut" && (!sum.is_finite() || sum <= 0.0) { return Err("Donut sum must be finite and positive".into()); }
    }
    Ok(())
}

fn validate_diagram(obj: &Object) -> Checked<()> {
    if optional_number(obj, "width")?.is_some_and(|v| v <= 0.0) { return Err("Diagram width must be positive".into()); }
    let mut ids = HashSet::new();
    for node in array(obj, "nodes", 32, false)? {
        let node = object(node)?;
        fields(node, &["id", "label", "x", "y", "w", "h", "description", "color"])?;
        let id = string(node, "id")?;
        if id.is_empty() || !ids.insert(id) { return Err("Diagram ids must be nonempty and unique".into()); }
        string(node, "label")?;
        optional_text(node, "description")?;
        num(node, "x")?; num(node, "y")?;
        for key in ["w", "h"] {
            if optional_number(node, key)?.is_some_and(|n| n <= 0.0) { return Err("Diagram dimensions must be positive".into()); }
        }
        color(node)?;
    }
    for edge in array(obj, "edges", 64, false)? {
        let edge = object(edge)?;
        fields(edge, &["from", "to", "label"])?;
        if !ids.contains(string(edge, "from")?) || !ids.contains(string(edge, "to")?) {
            return Err("Diagram edge references an unknown node".into());
        }
        optional_text(edge, "label")?;
    }
    Ok(())
}

struct Parameter<'a> { name: &'a str, value: f64, min: f64, max: f64 }
struct Plot<'a> { params: Vec<Parameter<'a>>, series: Vec<(&'a str, expression::Expression)> }

fn compile_plot(obj: &Object) -> Checked<Plot<'_>> {
    let x_min = num(obj, "xMin")?;
    let x_max = num(obj, "xMax")?;
    if x_min >= x_max || !(x_max - x_min).is_finite() { return Err("Plot xMin must be less than xMax with a finite range".into()); }
    let y_min = optional_number(obj, "yMin")?;
    let y_max = optional_number(obj, "yMax")?;
    if let (Some(min), Some(max)) = (y_min, y_max) {
        if min >= max || !(max - min).is_finite() { return Err("Invalid plot y range".into()); }
    }
    let mut params = Vec::with_capacity(8);
    if obj.contains_key("params") {
        for param in array(obj, "params", 8, false)? {
            let param = object(param)?;
            fields(param, &["name", "label", "value", "min", "max", "step"])?;
            let name = string(param, "name")?;
            if !expression::identifier(name) || ["x", "pi", "e", "tau"].contains(&name)
                || params.iter().any(|p: &Parameter<'_>| p.name == name) {
                return Err("Invalid, reserved or duplicate plot parameter name".into());
            }
            optional_text(param, "label")?;
            let value = num(param, "value")?;
            let min = num(param, "min")?;
            let max = num(param, "max")?;
            if min >= max || !(max - min).is_finite() || value < min || value > max {
                return Err("Invalid plot parameter range or default".into());
            }
            if optional_number(param, "step")?.is_some_and(|step| step <= 0.0 || step > max - min) {
                return Err("Plot parameter step must be positive and within its range".into());
            }
            params.push(Parameter { name, value, min, max });
        }
    }
    let names: Vec<&str> = params.iter().map(|p| p.name).collect();
    let mut series = Vec::with_capacity(8);
    for item in array(obj, "series", 8, true)? {
        let item = object(item)?;
        fields(item, &["label", "expr", "color"])?;
        optional_text(item, "label")?;
        color(item)?;
        let expr = string(item, "expr")?;
        series.push((item.get("label").and_then(Value::as_str).unwrap_or(expr), expression::Expression::parse(expr, &names)?));
    }
    Ok(Plot { params, series })
}

/// Samples a single validated plot at 513 evenly spaced points, preserving gaps as null.
pub fn sample_graphic_plot(source: &str, values: &str) -> Checked<String> {
    let value = parse(source)?;
    let obj = object(&value)?;
    if string(obj, "type")? != "plot" { return Err("Expected a plot node".into()); }
    fields(obj, &["type", "id", "title", "span", "xMin", "xMax", "yMin", "yMax", "height", "params", "series"])?;
    optional_text(obj, "title")?;
    if let Some(id) = obj.get("id") {
        if text(id)?.is_empty() { return Err("Graphic ids must be nonempty".into()); }
    }
    range(obj, "span", 1.0, 4.0, true)?;
    range(obj, "height", 120.0, 800.0, false)?;
    let plot = compile_plot(obj)?;
    let overrides = parse(values).map_err(|error| format!("Invalid plot values: {error}"))?;
    let overrides = object(&overrides)?;
    for name in overrides.keys() {
        if !plot.params.iter().any(|p| p.name == name) { return Err(format!("Unknown plot parameter: {name}")); }
    }
    let mut variables = [0.0; 9];
    for (index, param) in plot.params.iter().enumerate() {
        let n = overrides.get(param.name).map(number).transpose()?.unwrap_or(param.value);
        if n < param.min || n > param.max { return Err(format!("Plot parameter {} is out of range", param.name)); }
        variables[index + 1] = n;
    }
    let x_min = num(obj, "xMin")?;
    let x_max = num(obj, "xMax")?;
    let mut low = f64::INFINITY;
    let mut high = f64::NEG_INFINITY;
    let mut series = Vec::with_capacity(plot.series.len());
    let mut stack = [0.0; 128];
    for (label, expr) in &plot.series {
        let mut points: Vec<(f64, Option<f64>)> = Vec::with_capacity(513);
        for i in 0..=512 {
            let x = if i == 512 { x_max } else { x_min + (x_max - x_min) * (i as f64 / 512.0) };
            variables[0] = x;
            let y = expr.evaluate(&variables, &mut stack);
            let y = if y.is_finite() {
                low = low.min(y); high = high.max(y); Some(y)
            } else { None };
            points.push((x, y));
        }
        series.push(SampleSeries { label, points });
    }
    if !low.is_finite() { low = -1.0; high = 1.0; }
    if low == high {
        let padding = (low.abs() * 0.05).max(1.0);
        low = (low - padding).max(-f64::MAX);
        high = (high + padding).min(f64::MAX);
    }
    let fixed_min = optional_number(obj, "yMin")?;
    let fixed_max = optional_number(obj, "yMax")?;
    let mut y_min = fixed_min.unwrap_or(low);
    let mut y_max = fixed_max.unwrap_or(high);
    if y_min >= y_max {
        if fixed_max.is_none() {
            y_max = (y_min + (y_min.abs() * 0.05).max(1.0)).min(f64::MAX);
        } else if fixed_min.is_none() {
            y_min = (y_max - (y_max.abs() * 0.05).max(1.0)).max(-f64::MAX);
        }
    }
    if y_min >= y_max || !(y_max - y_min).is_finite() { return Err("Sampled plot y range is invalid; provide explicit yMin and yMax".into()); }
    serde_json::to_string(&SampledPlot { x_min, x_max, y_min, y_max, series }).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SampledPlot<'a> {
    x_min: f64,
    x_max: f64,
    y_min: f64,
    y_max: f64,
    series: Vec<SampleSeries<'a>>,
}

#[derive(Serialize)]
struct SampleSeries<'a> {
    label: &'a str,
    points: Vec<(f64, Option<f64>)>,
}
