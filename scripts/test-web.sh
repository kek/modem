#!/usr/bin/env bash
# One-command runner for the web POC end-to-end test suite.
#
# Ensures the wasm artifact + Playwright browser are installed, then runs
# the test suite (page-loads + roundtrip × {audible, ultrasonic}).
set -euo pipefail

cd "$(dirname "$0")/.."
REPO="$PWD"

# Wasm artifact is required by the page and is loaded at test time.
if [ ! -f web/pkg/modem_wasm.js ] || [ ! -f web/pkg/modem_wasm_bg.wasm ]; then
    echo "wasm not built; running scripts/build-wasm.sh first"
    "$REPO/scripts/build-wasm.sh"
fi

cd tests/web

# First run installs node deps + Chromium (~170 MB browser download).
if [ ! -d node_modules/@playwright ]; then
    echo "installing playwright (one-time)…"
    npm install
fi
if ! npx playwright install --dry-run chromium >/dev/null 2>&1; then
    echo "downloading Chromium for Playwright (one-time)…"
    npx playwright install chromium
fi

exec npx playwright test "$@"
