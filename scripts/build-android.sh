#!/usr/bin/env bash
set -euo pipefail

# Default ANDROID_NDK_HOME to the NDK we've validated locally if unset.
: "${ANDROID_NDK_HOME:=$HOME/Library/Android/sdk/ndk/29.0.14206865}"
export ANDROID_NDK_HOME

if [[ ! -d "$ANDROID_NDK_HOME" ]]; then
    echo "ANDROID_NDK_HOME does not exist: $ANDROID_NDK_HOME" >&2
    exit 1
fi

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ANDROID_JNI_DIR="$ROOT_DIR/android/app/src/main/jniLibs"
ANDROID_KT_DIR="$ROOT_DIR/android/app/src/main/java/uniffi/modem_ffi"
UNIFFI_BINDGEN="$ROOT_DIR/target/release/uniffi-bindgen"

echo "== Building modem-ffi for aarch64-linux-android (release) =="
cd "$ROOT_DIR"
mkdir -p "$ANDROID_JNI_DIR"
cargo ndk -t arm64-v8a -o "$ANDROID_JNI_DIR" build --release -p modem-ffi

echo "== Ensuring uniffi-bindgen bin is built =="
if [[ ! -x "$UNIFFI_BINDGEN" ]]; then
    cargo build -p uniffi-bindgen --release
fi

echo "== Generating Kotlin bindings =="
mkdir -p "$ANDROID_KT_DIR"
# uniffi-bindgen generate needs the cdylib that has the metadata embedded:
SO_PATH="$ROOT_DIR/target/aarch64-linux-android/release/libmodem_ffi.so"
"$UNIFFI_BINDGEN" generate \
    --library "$SO_PATH" \
    --language kotlin \
    --out-dir "$ROOT_DIR/android/app/src/main/java"

echo
echo "Produced:"
echo "  $ANDROID_JNI_DIR/arm64-v8a/libmodem_ffi.so"
echo "  $ANDROID_KT_DIR/modem_ffi.kt"
