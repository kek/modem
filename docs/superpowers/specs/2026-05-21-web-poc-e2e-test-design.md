# Web POC end-to-end test — design

Status: draft (v1 scope)
Date: 2026-05-21

## Goal

Catch regressions in the `web/` POC automatically. The valuable thing to
verify is *the full pipeline*: page boots, wasm loads, JS glue works,
Web Audio + AudioWorklet route samples correctly, and the wasm receiver
decodes a real microphone-shaped input back to the original bytes. A
broken `main.js` or `worklet.js` must fail this test.

## Non-goals

- Not a unit test of `modem-core`/`modem-codec` (covered by existing Rust
  tests).
- Not a visual-regression / canvas screenshot test.
- Not cross-browser. Chromium only.
- Not in CI yet. Local-first; CI is a separate concern.

## Approach

**Playwright + headless Chromium with a fake microphone fed from a
pre-rendered WAV file.**

Chromium has supported `--use-file-for-fake-audio-capture=<wav>` since
~2014; it's how Chrome's own WebRTC suite exercises audio. Combined with
`--use-fake-device-for-media-stream` and `--use-fake-ui-for-media-stream`,
the page's existing `getUserMedia()` call returns a synthetic mic that
plays back the WAV. The `AudioWorkletNode` we wired up captures from
that stream identically to a real mic. The `WasmReceiver` decodes, and a
`← <text>` line appears in the page log when `stream_complete` fires.

Why not the in-page DSP roundtrip alternative: that test would skip
`main.js`, `worklet.js`, the `AudioContext({sampleRate: 48000})`
negotiation, and the `getUserMedia` → `MediaStreamAudioSourceNode` →
`AudioWorkletNode` chain — i.e., all the code that's specific to the
web POC and most likely to silently regress.

## Architecture

New top-level `tests/web/`. Isolated from the rest of the repo (own
`package.json`, own `node_modules`) so the Rust workspace is unaffected.

```
tests/web/
  package.json             # playwright + @playwright/test
  playwright.config.ts     # one chromium project; auto-starts static server
  fixtures/
    .gitignore             # ignore generated WAVs
    render.mjs             # node script: writes user input file then
                           # shells out to `cargo run -p modem-cli -- tx-wav`
                           # to produce <profile>-<msg>.wav fixtures
  tests/
    page-loads.spec.ts     # health: no console errors, wasm-ready text, canvas exists
    roundtrip.spec.ts      # full pipeline: fake mic → decoded text in log
  README.md
scripts/
  test-web.sh              # builds wasm + fixtures, runs `npx playwright test`
```

Playwright's `webServer` config runs `python3 -m http.server -d web 8765`
on test startup and tears it down at exit. (Same command from
`web/README.md` — no new server code.)

## Fixture generation

Pre-render a WAV per `(profile, message)` pair via `cargo run -p modem-cli
-- tx-wav`. Cached under `tests/web/fixtures/`; regenerated only when the
input message or profile changes (mtime comparison in `render.mjs`).

**Format gotcha:** `cmd_tx_wav` writes 32-bit float WAV (`hound` with
`SampleFormat::Float`). Chrome's `--use-file-for-fake-audio-capture`
parser is known to accept 16-bit PCM reliably but to be picky about
other formats. Approach:

1. First attempt: feed the f32 WAV directly. If Chromium plays it
   correctly, done.
2. If it fails: `render.mjs` post-processes the f32 WAV into a 16-bit PCM
   WAV using a small inline converter (no extra deps — read the f32
   samples, multiply by 32767, clamp, write s16). ~20 lines of JS.

The spec assumes the converter is needed (low cost to include
unconditionally, removes one failure mode).

## Test cases

**`page-loads.spec.ts`** — single test, fast (< 2 s), no audio.

- Launch with default args (no fake mic flags).
- Load `http://localhost:8765/`.
- Listen for `console.error` and `pageerror` events; fail if any.
- Assert footer `#status` text matches `/wasm ready · sampleRate=48000 Hz/`.
- Assert `canvas#tones` exists and has nonzero `clientWidth`.
- Assert the 8 frequency labels appear in `.freq-labels` (basic profile
  sanity).

**`roundtrip.spec.ts`** — parameterized over `["audible", "ultrasonic"]`.

For each profile:

- Build/get cached fixture WAV for `"hello from playwright"` in that
  profile via `render.mjs`.
- Launch a fresh Chromium with:
  - `--use-fake-ui-for-media-stream`
  - `--use-fake-device-for-media-stream`
  - `--use-file-for-fake-audio-capture=<absolute-path-to-fixture>.wav`
- Load `http://localhost:8765/`, wait for `wasm ready` status.
- Select the right profile radio.
- Click `#listen-btn` ("Start listening"). The browser auto-grants mic
  permission (from `--use-fake-ui-for-media-stream`) and starts playing
  the fixture WAV as the mic input.
- Wait up to **20 s** for a `.log-recv` element containing
  `"hello from playwright"`. Timeout is double the worst-case message
  duration (~14 s for one frame).
- Assert the element has class `log-recv` (not `log-recv bad`) — i.e.
  `sha256_ok` was true.

Timeout chosen so a single retry of one frame still fits.

## Data flow

```
test setup:
   "hello from playwright" + profile
        ↓
   cargo run -p modem-cli -- tx-wav  →  fixture.wav (f32)
        ↓
   render.mjs converts to s16        →  fixture.s16.wav
        ↓
   stored under tests/web/fixtures/<profile>.s16.wav

test run:
   playwright launches chromium with --use-file-for-fake-audio-capture=fixture.s16.wav
        ↓
   page loads, user clicks Start Listening
        ↓
   chromium fake mic emits fixture samples
        ↓
   page's AudioWorklet → WasmReceiver.pushSamples
        ↓
   stream_complete → log row "← hello from playwright"
        ↓
   playwright sees the row → asserts text + sha-ok class
```

## Error handling

- WAV not found / cargo build failed: `render.mjs` exits non-zero;
  Playwright reports fixture step failed before any tests run.
- wasm not built: `scripts/test-web.sh` runs `scripts/build-wasm.sh`
  first, which is idempotent and exits non-zero on build failure.
- Static server fails to start on port 8765: Playwright's `webServer`
  config has a 30 s `timeout` and reports the failure cleanly.
- Decode timeout in `roundtrip.spec.ts`: the wait-for-locator API
  surfaces "expected `← hello from playwright` to be visible within
  20000 ms" with the page's HTML attached as a Playwright artifact.

## Risks / open questions

- **Chromium WAV format support.** Documented above; mitigation is the
  s16 converter in `render.mjs`. If even s16 fails, the fallback is to
  write a tiny `tests/web/fixtures/render.rs` Rust binary that uses
  `hound` to emit s16 directly — but that's only needed if JS-side
  conversion has subtle scaling issues.
- **AudioContext sample rate.** The page requests 48 kHz; if Chromium's
  audio backend silently returns 44.1 kHz, decode misaligns and the test
  fails with a timeout. The page already logs a warning in that case;
  the test asserts on `sampleRate=48000` in the status, so a sample-rate
  refusal manifests in `page-loads.spec.ts` rather than in the harder-to-
  debug roundtrip test.
- **Fake-mic startup timing.** Chrome starts replaying the fixture when
  the page's `getUserMedia` resolves. If the page's `AudioWorklet` adds
  latency between `getUserMedia` and the first chunk reaching the
  receiver, the preamble might still be the first audio captured — the
  receiver's preamble search tolerates leading silence (`captures/` has
  similar shape). Tested behaviour, not theoretical, but flagged.
- **Headless Chromium audio backend on macOS.** Some headless setups
  route audio to a null sink that doesn't accept fake-file injection.
  Tested ad-hoc; if it fails, `headless: false` is an acceptable fallback
  for local dev (real Chrome window pops up; tests still pass).
- **First-run weight.** Playwright auto-downloads Chromium (~170 MB) on
  install. One-time cost, but worth flagging.

## Testing the tests

- `page-loads.spec.ts` is fast and self-validating: introduce a deliberate
  console.error in `main.js` and verify the test fails.
- `roundtrip.spec.ts` validation: rebuild the fixture with a different
  string than the assertion checks; verify the test fails. Also: corrupt
  one byte of the fixture WAV and verify the test fails (likely with
  `FrameDropped` log entries — which is *information*, not error).

## Out of scope for v1

- The in-page DSP roundtrip ("approach A" from brainstorming). Can layer
  on later if `roundtrip.spec.ts` turns out flaky or slow.
- CI integration (GitHub Actions etc.). Separate concern; once the local
  test is solid, adding `.github/workflows/web.yml` is mechanical.
- Multiple message lengths / multi-frame fixtures. Single-frame
  short-message coverage is enough for v1.
- Cross-browser (Firefox/WebKit). The fake-mic flag is Chromium-only.

## Rollout

No flag — it's a new test suite, off the critical path. `scripts/test-web.sh`
is the one new entrypoint. Existing `cargo test --workspace` and
`./gradlew testDebugUnitTest` are unaffected.
