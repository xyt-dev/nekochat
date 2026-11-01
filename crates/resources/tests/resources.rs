use nekochat_resources::{ResourceCatalog, ResourceChunk};
use std::path::Path;

fn catalog() -> ResourceCatalog {
    ResourceCatalog::load(Path::new("/nonexistent-nekochat-resource-test/skills")).unwrap()
}

fn render(chunks: Vec<ResourceChunk>) -> String {
    chunks
        .into_iter()
        .map(|chunk| match chunk {
            ResourceChunk::Text(text) => text,
            ResourceChunk::Sticker(sticker) => format!("<{}:{}>", sticker.pack_id, sticker.name),
        })
        .collect()
}

#[test]
fn bundled_catalog_exposes_complete_offline_assets_and_annotations() {
    let catalog = catalog();
    let packs = catalog.packs();
    for pack in &packs {
        for asset in &pack.assets {
            let image = catalog.asset(&pack.id, &asset.id).unwrap();
            assert_eq!(image.mime, "image/gif");
            assert!(image.bytes.starts_with(b"GIF89a") || image.bytes.starts_with(b"GIF87a"));
        }
    }
    assert!(catalog.asset("nekowhale", "../NOTICE").is_none());
    assert!(catalog.asset("../nekowhale", "4a44767ab58b.gif").is_none());
}

#[test]
fn every_stream_boundary_holds_markers_and_preserves_fenced_literals() {
    let catalog = catalog();
    let allowed = vec!["nekowhale".into()];
    let source = "开始\n```rust\n[sticker:猫猫搓手]\n```\n好了[sticker:大肥鱼/10N01_猫猫搓手]结束";
    let expected = "开始\n```rust\n[sticker:猫猫搓手]\n```\n好了<nekowhale:猫猫搓手>结束";
    for split in source
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(source.len()))
    {
        let mut filter = catalog.filter(&allowed, &[]).unwrap();
        let mut chunks = filter.push(&source[..split]);
        chunks.extend(filter.push(&source[split..]));
        chunks.extend(filter.finish());
        assert_eq!(render(chunks), expected, "boundary {split}");
    }
}

#[test]
fn single_character_chunks_preserve_indented_long_fences_and_bound_invalid_markers() {
    let catalog = catalog();
    let literal = "   ~~~~text\r\n[sticker:猫猫搓手]\r\n~~~\r\n[sticker:猫猫搓手]\r\n   ~~~~\r\n";
    let mut filter = catalog.filter(&["nekowhale".into()], &[]).unwrap();
    let mut chunks = Vec::new();
    for character in literal.chars() {
        chunks.extend(filter.push(&character.to_string()));
    }
    chunks.extend(filter.push("[sticker:猫猫搓手]"));
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), format!("{literal}<nekowhale:猫猫搓手>"));
    let mut filter = catalog.filter(&["nekowhale".into()], &[]).unwrap();
    let mut chunks = filter.push(&format!("before[sticker:{}", "x".repeat(5000)));
    chunks.extend(filter.push("more]after[sticker:猫猫搓手]"));
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), "beforeafter<nekowhale:猫猫搓手>");
}

#[test]
fn unauthorized_unknown_ambiguous_and_repeated_markers_never_choose_replacements() {
    let catalog = catalog();
    let mut filter = catalog.filter(&["nekowhale".into()], &[]).unwrap();
    let mut chunks = filter.push("A[sticker:宣判]B[sticker:大肥鱼和GPT娘/2TT02_情侣杯]C[sticker:不存在]D[sticker:10N01_猫猫搓手]E[sticker:摸下巴]F");
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), "ABCD<nekowhale:猫猫搓手>EF");
    let mut filter = catalog
        .filter(
            &["nekowhale".into()],
            &["nekowhale/4a44767ab58b.gif".into()],
        )
        .unwrap();
    let mut chunks = filter.push("[sticker:猫猫搓手]text");
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), "text");
}

#[test]
fn malformed_partial_markers_do_not_leak_but_other_brackets_remain() {
    let catalog = catalog();
    for suffix in ["[", "[st", "[sticker:", "[sticker:猫猫搓手"] {
        let mut filter = catalog.filter(&["nekowhale".into()], &[]).unwrap();
        let mut chunks = filter.push(&format!("ok{suffix}"));
        chunks.extend(filter.finish());
        assert_eq!(render(chunks), "ok");
    }
    let mut filter = catalog.filter(&[], &[]).unwrap();
    let mut chunks = filter.push("[ordinary] [sticker:nope]");
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), "[ordinary] ");
}

#[test]
fn prompt_authorization_is_order_independent_and_rejects_unknown_packs() {
    let catalog = catalog();
    let a = catalog
        .prompt(
            &["nekowhale-gpt".into(), "nekowhale".into()],
            "neutral",
            "rare",
        )
        .unwrap();
    let b = catalog
        .prompt(
            &[
                "nekowhale".into(),
                "nekowhale-gpt".into(),
                "nekowhale".into(),
            ],
            "neutral",
            "rare",
        )
        .unwrap();
    assert_eq!(a, b);
    assert!(
        catalog
            .prompt(&["missing".into()], "neutral", "normal")
            .is_err()
    );
    assert!(catalog.filter(&["missing".into()], &[]).is_err());
    assert!(catalog.prompt(&[], "self", "often").unwrap().is_empty());
}

fn custom_pack(root: &Path, file: &str) {
    let pack = root.join("custom");
    std::fs::create_dir_all(pack.join("assets")).unwrap();
    std::fs::write(
        pack.join("pack.json"),
        r#"{"id":"custom","name":"Custom","description":"Local","attribution":"Owner"}"#,
    )
    .unwrap();
    std::fs::write(pack.join("SKILL.md"), "Static instructions only").unwrap();
    std::fs::write(
        pack.join("stickers.json"),
        serde_json::json!([{
            "id":"test.gif", "name":"独特表情", "file":file,
            "aliases":["编号_独特表情"], "emotion":"开心", "tags":[], "use":"测试", "dir":""
        }])
        .to_string(),
    )
    .unwrap();
    std::fs::write(
        pack.join("assets/test.gif"),
        b"GIF89a\x01\x00\x01\x00\x00\x00\x00;",
    )
    .unwrap();
}

#[test]
fn custom_pack_loads_static_bytes_and_unique_fuzzy_alias() {
    let root = tempfile::tempdir().unwrap();
    custom_pack(root.path(), "assets/test.gif");
    let catalog = ResourceCatalog::load(root.path()).unwrap();
    // Loaded once: subsequent filesystem changes cannot alter a running catalog.
    std::fs::remove_file(root.path().join("custom/assets/test.gif")).unwrap();
    assert_eq!(
        catalog.asset("custom", "test.gif").unwrap().bytes.as_ref(),
        b"GIF89a\x01\x00\x01\x00\x00\x00\x00;"
    );
    let mut filter = catalog.filter(&["custom".into()], &[]).unwrap();
    let mut chunks = filter.push("[sticker:独特]");
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), "<custom:独特表情>");
}

#[test]
fn custom_manifest_cannot_escape_its_pack() {
    let root = tempfile::tempdir().unwrap();
    custom_pack(root.path(), "../outside.gif");
    std::fs::write(root.path().join("outside.gif"), b"GIF89a").unwrap();
    assert!(ResourceCatalog::load(root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_assets_and_pack_directories_are_rejected() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    custom_pack(root.path(), "assets/test.gif");
    let asset = root.path().join("custom/assets/test.gif");
    std::fs::remove_file(&asset).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    symlink(outside.path(), &asset).unwrap();
    assert!(ResourceCatalog::load(root.path()).is_err());
    let root = tempfile::tempdir().unwrap();
    symlink(outside.path(), root.path().join("escape")).unwrap();
    assert!(ResourceCatalog::load(root.path()).is_err());
}

#[test]
fn instruction_packs_preserve_graphics_and_ignore_sticker_identity() {
    let root = tempfile::tempdir().unwrap();
    custom_pack(root.path(), "assets/test.gif");
    std::fs::write(root.path().join("custom/stickers.json"), "[]").unwrap();
    std::fs::write(root.path().join("custom/SKILL.md"), "Use declared graphics with local controls.").unwrap();
    let catalog = ResourceCatalog::load(root.path()).unwrap();
    let allowed = ["custom".into()];
    let prompt = catalog.prompt(&allowed, "self", "often").unwrap();
    assert_eq!(prompt, catalog.prompt(&allowed, "neutral", "rare").unwrap());
    let mut filter = catalog.filter(&allowed, &[]).unwrap();
    let source = "```neko-ui\n{\"version\":1,\"items\":[]}\n```";
    let mut chunks = filter.push(source);
    chunks.extend(filter.finish());
    assert_eq!(render(chunks), source);
}
