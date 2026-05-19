#!/usr/bin/env bash
# Cross-compile modem-wasm to wasm32 and produce a browser-ready JS module
# in web/pkg. Installs wasm-bindgen-cli (matching the Cargo.toml version) and
# the wasm32 target on first run.
set -euo pipefail

cd "$(dirname "$0")/.."

WBG_VER="0.2.121"

# Ensure the wasm32 target.
rustup target add wasm32-unknown-unknown >/dev/null

# Ensure matching wasm-bindgen CLI is installed (pinned to the same version
# the wasm-bindgen crate uses in Cargo.toml — mismatched versions error out).
if ! command -v wasm-bindgen >/dev/null 2>&1 \
    || ! wasm-bindgen --version 2>/dev/null | grep -q "$WBG_VER"; then
    echo "installing wasm-bindgen-cli $WBG_VER (one-time)…"
    cargo install --locked wasm-bindgen-cli --version "$WBG_VER"
fi

echo "compiling modem-wasm…"
cargo build --release -p modem-wasm --target wasm32-unknown-unknown

echo "running wasm-bindgen…"
mkdir -p web/pkg
wasm-bindgen \
    --target web \
    --out-dir web/pkg \
    --no-typescript \
    target/wasm32-unknown-unknown/release/modem_wasm.wasm

echo "✓ built: web/pkg/modem_wasm{.js,_bg.wasm}"
ls -lh web/pkg
