# tests/web — Playwright end-to-end tests for the web POC

Catches regressions in the `web/` POC by driving a headless Chromium
through the full pipeline: page load → wasm init → mic capture (faked
from a WAV) → `WasmReceiver` decode → message appears in the log.

## Run

```bash
./scripts/test-web.sh
```

First run installs Node deps and Playwright's Chromium build (~170 MB,
one-time). Subsequent runs reuse them.

## What it tests

- **`page-loads.spec.ts`** — page boots, wasm initialises, `AudioContext`
  reports 48 kHz, the tone canvas is present, no console errors during
  load.
- **`roundtrip.spec.ts`** — for each of `audible` and `ultrasonic`,
  launches Chromium with `--use-file-for-fake-audio-capture=<wav>`,
  clicks "Start listening", asserts that "hello from playwright" appears
  in the log with `sha256_ok=true` within 20 seconds.

## Fixtures

Pre-rendered by `fixtures/render.mjs` via `cargo run -p modem-cli -- tx-wav`,
post-converted from `hound`'s f32 WAV to 16-bit PCM (the format Chromium's
fake-audio decoder accepts reliably). Cached under `fixtures/*.wav` and
regenerated only when the input message changes.

## Why fake-mic instead of in-page DSP?

The point of these tests is to catch regressions in `main.js` /
`worklet.js` / the `AudioContext` plumbing — not the wasm DSP, which has
its own Rust tests. Driving the decoder directly via `page.evaluate`
would skip exactly the code most likely to silently break.

See `docs/superpowers/specs/2026-05-21-web-poc-e2e-test-design.md` for the
full design.
