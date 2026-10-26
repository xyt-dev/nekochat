use std::path::Path;

fn main() {
    // rust-embed embeds `web/dist` in release builds but cannot tell cargo about files that
    // appear or disappear. Watch the dist directory, or `web` while it is not built yet (a
    // missing path would make cargo rebuild this crate every time).
    let watched = if Path::new("../../web/dist").exists() {
        "../../web/dist"
    } else {
        "../../web"
    };
    println!("cargo:rerun-if-changed={watched}");
}
