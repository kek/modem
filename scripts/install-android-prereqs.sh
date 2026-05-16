#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

echo "== Adding Android target =="
rustup target add aarch64-linux-android

echo "== Installing cargo-ndk =="
if ! command -v cargo-ndk >/dev/null; then
    cargo install cargo-ndk
fi

echo "== Building workspace uniffi-bindgen bin =="
# UniFFI 0.28's `uniffi` crate has no published `uniffi-bindgen` binary;
# the canonical pattern is a tiny in-tree crate that calls
# `uniffi::uniffi_bindgen_main()`. See `uniffi-bindgen/` in this workspace.
# We build it (rather than `cargo install`) so the version stays pinned
# to whatever the workspace uses.
( cd "$ROOT_DIR" && cargo build -p uniffi-bindgen --release )

UNIFFI_BINDGEN="$ROOT_DIR/target/release/uniffi-bindgen"

echo
echo "Prerequisites:"
echo "  rustc target aarch64-linux-android : OK"
echo "  cargo-ndk                          : $(cargo-ndk --version)"
echo "  uniffi-bindgen                     : $("$UNIFFI_BINDGEN" --version 2>/dev/null || echo "built at $UNIFFI_BINDGEN")"
echo
echo "Make sure ANDROID_NDK_HOME is set, e.g.:"
echo "  export ANDROID_NDK_HOME=\$HOME/Library/Android/sdk/ndk/29.0.14206865"
