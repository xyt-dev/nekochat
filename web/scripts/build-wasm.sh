#!/usr/bin/env bash
# Builds crates/mdwasm to WASM and generates the JS bindings into web/src/wasm/.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
cargo build -p nekochat-mdwasm --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir web/src/wasm target/wasm32-unknown-unknown/release/nekochat_mdwasm.wasm
