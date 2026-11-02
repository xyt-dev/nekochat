use nekochat_core::graphics::{sanitize_graphic_svg, sample_graphic_plot, validate_complete_graphic_json, validate_graphic_json};
use serde_json::{Value, json};

#[test]
fn nested_graphics_preserve_data_and_semantic_colors() {
    let source = json!({"version":1,"items":[{"type":"grid","cols":2,"items":[
        {"type":"stat","label":"Visitors","value":42,"color":"accent"},
        {"type":"chart","kind":"line","labels":["Mon","Tue"],"series":[{"label":"Net","values":[-2,3],"color":"teal"}]}
    ]}]}).to_string();
    let checked: Value = serde_json::from_str(&validate_graphic_json(&source).unwrap()).unwrap();
    assert_eq!(checked["items"][0]["items"][0]["value"], 42);
    assert_eq!(checked["items"][0]["items"][0]["color"], "accent");
    assert_eq!(checked["items"][0]["items"][1]["series"][0]["values"], json!([-2,3]));
}

#[test]
fn invalid_graphical_contracts_are_rejected() {
    for node in [
        json!({"type":"script","code":"alert(1)"}),
        json!({"type":"stat","label":"A","value":1,"color":"url(https://example.com/x)"}),
        json!({"type":"text","content":"A","onClick":"alert(1)"}),
        json!({"type":"chart","kind":"line","labels":["A"],"series":[{"label":"X","values":[1,2]}]}),
        json!({"type":"chart","kind":"donut","labels":["A"],"series":[{"label":"X","values":[-1]}]}),
        json!({"type":"plot","xMin":2,"xMax":1,"series":[{"expr":"x"}]}),
        json!({"type":"plot","xMin":0,"xMax":1,"series":[{"expr":"globalThis.fetch(x)"}]}),
        json!({"type":"diagram","nodes":[{"id":"a","label":"A","x":0,"y":0}],"edges":[{"from":"a","to":"missing"}]}),
    ] {
        assert!(validate_graphic_json(&json!({"version":1,"items":[node]}).to_string()).is_err());
    }
}

#[test]
fn diagram_default_colors_accept_only_the_existing_safe_palette() {
    let mut source = json!({"version":1,"title":"预算怎么被吃掉的","gap":10,"items":[
        {"type":"text","size":"muted","content":"**示例结构，不是实测数值**：窗口先扣固定开销，再扣历史与材料，最后必须给输出留位置。"},
        {"type":"diagram","width":420,"height":280,"color":"blue","nodes":[
            {"id":"w","label":"窗口上限","x":140,"y":8,"w":140,"h":34,"description":"模型单次能处理的最大 token 数"},
            {"id":"s","label":"系统提示与工具","x":140,"y":52,"w":140,"h":34,"description":"常驻开销，通常改不了，但要知道它占了多少"},
            {"id":"h","label":"历史对话","x":140,"y":96,"w":140,"h":34,"description":"会随轮次累积，最容易被忽视的一块"},
            {"id":"m","label":"贴入的材料","x":140,"y":140,"w":140,"h":34,"description":"代码、日志、文档，只留与问题相关的部分"},
            {"id":"o","label":"输出预留","x":140,"y":184,"w":140,"h":34,"description":"不给它留位置，回答就会在中途被截断"},
            {"id":"r","label":"真正可用","x":140,"y":228,"w":140,"h":34,"description":"剩下的部分才决定你能问多复杂的问题","color":"teal"}
        ],"edges":[
            {"from":"w","to":"s","label":"减"},{"from":"s","to":"h","label":"减"},
            {"from":"h","to":"m","label":"减"},{"from":"m","to":"o","label":"减"},
            {"from":"o","to":"r","label":"剩"}
        ]}
    ]});
    for color in ["blue", "accent", "#39a", "#3399aa", "#3399aa80"] {
        source["items"][1]["color"] = json!(color);
        validate_graphic_json(&source.to_string()).unwrap_or_else(|error| panic!("{color}: {error}"));
    }
    for color in [json!("url(https://example.com/color)"), json!("var(--custom)"), json!("#12"), json!(17), Value::Null] {
        source["items"][1]["color"] = color;
        assert!(validate_graphic_json(&source.to_string()).is_err());
    }
    source["items"][1]["color"] = json!("blue");
    source["items"][1]["colour"] = json!("teal");
    assert!(validate_graphic_json(&source.to_string()).is_err());
}

#[test]
fn recursive_and_duplicate_graphical_nodes_are_bounded() {
    let duplicate = json!({"version":1,"items":[{"type":"text","id":"same","content":"A"},{"type":"text","id":"same","content":"B"}]}).to_string();
    assert!(validate_graphic_json(&duplicate).is_err());
    let many = json!({"version":1,"items":vec![json!({"type":"text","content":"A"});129]}).to_string();
    assert!(validate_graphic_json(&many).is_err());
    let mut deep = json!({"type":"text","content":"A"});
    for _ in 0..10 { deep = json!({"type":"col","items":[deep]}); }
    assert!(validate_graphic_json(&json!({"version":1,"items":[deep]}).to_string()).is_err());
}

#[test]
fn svg_cannot_run_scripts_or_fetch_resources() {
    for source in [
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\" onload=\"alert(1)\"><rect width=\"10\" height=\"10\"/></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><image href=\"https://example.com/a.png\"/></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>svg{background:url(https://example.com/x)}</style></svg>",
    ] { assert!(sanitize_graphic_svg(source).is_err()); }
}

#[test]
fn plot_parameters_change_real_samples_and_enforce_ranges() {
    let source = json!({"type":"plot","palette":true,"xMin":0,"xMax":2,"params":[{"name":"a","value":2,"min":0,"max":4}],"series":[{"expr":"a*x+1","label":"f"}]}).to_string();
    let sampled: Value = serde_json::from_str(&sample_graphic_plot(&source,"{\"a\":3}").unwrap()).unwrap();
    assert_eq!(sampled["series"][0]["points"][0],json!([0.0,1.0]));
    assert_eq!(sampled["series"][0]["points"][256],json!([1.0,4.0]));
    assert_eq!(sampled["series"][0]["points"][512],json!([2.0,7.0]));
    assert!(sample_graphic_plot(&source,"{\"a\":5}").is_err());
}

#[test]
fn plot_discontinuities_are_gaps_instead_of_nonfinite_json() {
    let source = json!({"type":"plot","xMin":-1,"xMax":1,"series":[{"expr":"1/x"}]}).to_string();
    let sampled: Value = serde_json::from_str(&sample_graphic_plot(&source,"{}").unwrap()).unwrap();
    assert_eq!(sampled["series"][0]["points"][256],json!([0.0,null]));
    assert_eq!(sampled["series"][0]["points"][512],json!([1.0,1.0]));
}

#[test]
fn svg_lengths_accept_rem_and_other_units_but_reject_malformed_suffixes() {
    for size in ["1rem", "1.5rem", "0rem", "1em", "1ex", "1px", "100%"] {
        let source = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text font-size=\"{size}\">A</text></svg>");
        let safe = sanitize_graphic_svg(&source).unwrap_or_else(|error| panic!("{size}: {error}"));
        assert!(safe.contains(&format!("font-size=\"{size}\"")), "{size}");
        let graphic = json!({"version":1,"items":[{"type":"svg","code":source}]}).to_string();
        assert!(validate_graphic_json(&graphic).is_ok(), "embedded SVG with {size}");
    }
    for size in ["1rrem", "1remem", "1remx", "rem", "1r", "-1rem", "NaNrem", "infrem"] {
        let source = format!("<svg xmlns=\"http://www.w3.org/2000/svg\"><text font-size=\"{size}\">A</text></svg>");
        assert!(sanitize_graphic_svg(&source).is_err(), "{size}");
    }
}

#[test]
fn text_colors_accept_declared_palette_values_and_reject_css_payloads() {
    let source = json!({"version":1,"items":[
        {"type":"text","content":"标题","size":"h2","color":"accent"},
        {"type":"text","content":"正文","color":"#79c"},
        {"type":"text","content":"说明","size":"muted","color":"#12345678"}
    ]}).to_string();
    let checked: Value = serde_json::from_str(&validate_graphic_json(&source).unwrap()).unwrap();
    assert_eq!(checked["items"][0]["color"], "accent");
    assert_eq!(checked["items"][1]["color"], "#79c");
    assert_eq!(checked["items"][2]["color"], "#12345678");
    for color in ["url(https://example.com/asset)", "red;display:none", "var(--untrusted)"] {
        let unsafe_text = json!({"version":1,"items":[{"type":"text","content":"文字","color":color}]}).to_string();
        assert!(validate_graphic_json(&unsafe_text).is_err(), "{color}");
    }
}

#[test]
fn question_choices_preserve_labels_and_complete_prompts_inside_layouts() {
    let source = json!({"version":1,"items":[{"type":"card","items":[{
        "type":"choices","title":"接下来可以问","options":[
            {"label":"看例子","prompt":"请用一个真实例子解释这个概念。"},
            {"label":"比较方案","prompt":"这两种方案分别适合什么情况？"}
        ]}]}]}).to_string();
    let checked: Value = serde_json::from_str(&validate_graphic_json(&source).unwrap()).unwrap();
    assert_eq!(checked["items"][0]["items"][0]["options"][0]["label"], "看例子");
    assert_eq!(checked["items"][0]["items"][0]["options"][1]["prompt"], "这两种方案分别适合什么情况？");
    let boundary = json!({"version":1,"items":[{"type":"choices","options":[
        {"label":"问".repeat(80),"prompt":"题".repeat(500)}
    ]}]}).to_string();
    assert!(validate_graphic_json(&boundary).is_ok());
}

#[test]
fn question_choices_reject_ambiguous_empty_unbounded_or_executable_options() {
    let valid = json!({"label":"看例子","prompt":"请给一个例子。"});
    let too_many: Vec<Value> = (0..7).map(|index| json!({"label":format!("问题{index}"),"prompt":format!("完整提问{index}")})).collect();
    for options in [
        json!([]), json!(too_many), json!([valid.clone(),valid.clone()]),
        json!([{"label":" ","prompt":"请解释。"}]),
        json!([{"label":"解释","prompt":"\n "}]),
        json!([{"label":"问".repeat(81),"prompt":"请解释。"}]),
        json!([{"label":"解释","prompt":"题".repeat(501)}]),
        json!([{"label":"解释"}]), json!(["解释"]),
        json!([{"label":"解释","prompt":"请解释。","url":"https://example.com"}]),
        json!([valid.clone(),{"label":" 看例子 ","prompt":"再给一个例子。"}]),
        json!([valid,{"label":"另一种说法","prompt":" 请给一个例子。 "}]),
    ] {
        let source = json!({"version":1,"items":[{"type":"choices","options":options}]}).to_string();
        assert!(validate_graphic_json(&source).is_err(), "{source}");
    }
}

#[test]
fn completed_graphics_recover_missing_container_ends_without_changing_choices() {
    let source = r#"{"version":1,"items":[{"type":"choices","options":[{"label":"看例子","prompt":"请举一个具体例子。"},{"label":"比较方案","prompt":"比较这两个方案。"},{"label":"解释符号","prompt":"请解释 \"a]}\" 的含义。"}]}"#;
    assert!(validate_graphic_json(source).is_err());
    let checked: Value = serde_json::from_str(&validate_complete_graphic_json(source).unwrap()).unwrap();
    assert_eq!(checked["items"][0]["options"], json!([
        {"label":"看例子","prompt":"请举一个具体例子。"},
        {"label":"比较方案","prompt":"比较这两个方案。"},
        {"label":"解释符号","prompt":"请解释 \"a]}\" 的含义。"}
    ]));
}

#[test]
fn container_completion_rejects_missing_content_invalid_syntax_and_unsafe_fields() {
    for source in [
        r#"{"version":1,"items":[{"type":"choices","options":[{"label":"例子","prompt":"未完成"#,
        r#"{"version":1,"items":[{"type":"choices","options":[{"label":"例子"}"#,
        r#"{"version":1,"items":[{"type":"choices","options":[{"label":"例子","prompt":"请举例。"},"#,
        r#"{"version":1,"items":[{"type":"choices" "options":[]"#,
        r#"{"version":1,"version":1,"items":[]"#,
        r#"{"version":1,"items":[{"type":"choices","options":[{"label":"例子","prompt":"请举例。","url":"https://example.com"}]}"#,
        r#"{"version":1,"items":[{"type":"text","content":"正文","color":"url(https://example.com)"}"#,
    ] {
        assert!(validate_complete_graphic_json(source).is_err(), "{source}");
    }
}

#[test]
fn colorable_components_preserve_explicit_boolean_palette_permissions() {
    let source = json!({"version":1,"items":[
        {"type":"text","content":"说明","palette":true},
        {"type":"stat","label":"数量","value":42,"palette":false},
        {"type":"row","items":[],"palette":true},
        {"type":"col","items":[],"palette":true},
        {"type":"grid","items":[],"palette":true},
        {"type":"card","items":[],"palette":true},
        {"type":"chart","kind":"bar","labels":["A"],"series":[{"label":"数量","values":[2]}],"palette":true},
        {"type":"plot","xMin":0,"xMax":1,"series":[{"expr":"x"}],"palette":true},
        {"type":"diagram","nodes":[],"edges":[],"palette":true},
        {"type":"svg","code":"<svg viewBox=\"0 0 10 10\"><circle cx=\"5\" cy=\"5\" r=\"2\"/></svg>","palette":true}
    ]}).to_string();
    let checked: Value = serde_json::from_str(&validate_graphic_json(&source).unwrap()).unwrap();
    assert_eq!(checked["items"][0]["palette"], true);
    assert_eq!(checked["items"][1]["palette"], false);
    for node in &checked["items"].as_array().unwrap()[2..] { assert_eq!(node["palette"], true); }
}

#[test]
fn palette_permissions_reject_truthy_values_and_noncolorable_choices() {
    for flag in [json!("true"),json!(1),Value::Null,json!({})] {
        let source = json!({"version":1,"items":[{"type":"stat","label":"数量","value":42,"palette":flag}]}).to_string();
        assert!(validate_graphic_json(&source).is_err());
        let source = json!({"type":"plot","xMin":0,"xMax":1,"series":[{"expr":"x"}],"palette":flag}).to_string();
        assert!(sample_graphic_plot(&source,"{}").is_err());
    }
    let source = r#"{"version":1,"items":[{"type":"choices","palette":true,"options":[{"label":"例子","prompt":"请举例。"}]}]}"#;
    assert!(validate_graphic_json(source).is_err());
}

#[test]
fn svg_accessibility_references_allow_only_existing_local_ids() {
    let source = r#"<svg viewBox="0 0 200 100" role="img" aria-labelledby="scene-title scene-desc" aria-describedby="scene-desc"><title id="scene-title">海与星之间</title><desc id="scene-desc">鲸鱼游向星座</desc><circle cx="50" cy="50" r="20"/></svg>"#;
    let safe = sanitize_graphic_svg(source).unwrap();
    let document = roxmltree::Document::parse(&safe).unwrap();
    let root = document.root_element();
    assert_eq!(root.attribute("aria-labelledby"), Some("scene-title scene-desc"));
    assert_eq!(root.attribute("aria-describedby"), Some("scene-desc"));
    for references in ["", "missing", "https://example.com", "#scene-title"] {
        let source = format!(r#"<svg viewBox="0 0 10 10" aria-labelledby="{references}"><title id="scene-title">场景</title></svg>"#);
        assert!(sanitize_graphic_svg(&source).is_err(), "{references}");
    }
}

#[test]
fn svg_theme_colors_preserve_safe_literal_fallbacks_and_reject_external_paints() {
    let source = r##"<svg viewBox="0 0 100 100"><defs><linearGradient id="sea"><stop offset="0" stop-color="var(--nk-blue, #4a90d9)"/><stop offset="1" stop-color="var(--nk-blue, navy)"/></linearGradient></defs><circle cx="50" cy="50" r="30" fill="url(#sea)"/></svg>"##;
    let safe = sanitize_graphic_svg(source).unwrap();
    let document = roxmltree::Document::parse(&safe).unwrap();
    let stops: Vec<_> = document.descendants().filter(|node| node.has_tag_name("stop")).collect();
    assert_eq!(stops[0].attribute("stop-color"), Some("var(--nk-blue, #4a90d9)"));
    assert_eq!(stops[1].attribute("stop-color"), Some("var(--nk-blue, navy)"));
    for paint in ["var(--nk-blue, url(https://example.com))","var(--nk-blue, red);display:none"] {
        let source = format!(r#"<svg viewBox="0 0 10 10"><circle cx="5" cy="5" r="2" fill="{paint}"/></svg>"#);
        assert!(sanitize_graphic_svg(&source).is_err());
    }
}
