use nekochat_core::graphics::{sanitize_graphic_svg, sample_graphic_plot, validate_graphic_json};
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
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><foreignObject><div>bad</div></foreignObject></svg>",
        "<svg xmlns=\"http://www.w3.org/2000/svg\"><style>svg{background:url(https://example.com/x)}</style></svg>",
    ] { assert!(sanitize_graphic_svg(source).is_err()); }
}

#[test]
fn plot_parameters_change_real_samples_and_enforce_ranges() {
    let source = json!({"type":"plot","xMin":0,"xMax":2,"params":[{"name":"a","value":2,"min":0,"max":4}],"series":[{"expr":"a*x+1","label":"f"}]}).to_string();
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
