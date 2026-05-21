# Web POC End-to-End Test Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an automated Playwright + Chromium test that exercises the full `web/` POC pipeline — wasm load, Web Audio routing, `AudioWorklet` capture, `WasmReceiver` decode — via a fake microphone fed from a pre-rendered WAV.

**Architecture:** Top-level `tests/web/` Playwright project, isolated from the Rust workspace. A static file server (`python3 -m http.server -d web 8765`) is auto-started by Playwright. One health-check spec runs with default Chromium; the roundtrip spec spawns its own Chromium per profile with `--use-file-for-fake-audio-capture=<wav>` pointing at fixtures generated from `modem tx-wav`. Fixtures are rendered by a Node-based `globalSetup` hook that shells out to `cargo run` and converts the f32 output to 16-bit PCM (the format Chromium's fake-audio decoder accepts reliably).

**Tech Stack:** Node 24, Playwright `^1.50` + `@playwright/test`, Python `http.server` (already required by `web/README.md`), `cargo` for fixture rendering (already required by the build).

**Spec:** `docs/superpowers/specs/2026-05-21-web-poc-e2e-test-design.md`

---

## File Structure

### New files
- `tests/web/package.json` — pins `@playwright/test`.
- `tests/web/.gitignore` — `node_modules/`, `playwright-report/`, `test-results/`, `fixtures/*.wav`.
- `tests/web/playwright.config.ts` — one Chromium project, `webServer` auto-starts the static file server, `globalSetup` renders fixtures, base URL `http://localhost:8765`.
- `tests/web/fixtures/render.mjs` — pure-function module: given `(profile, message)`, ensure a 16-bit PCM WAV exists at `tests/web/fixtures/<profile>.s16.wav` by shelling out to `cargo run -p modem-cli -- tx-wav` and post-converting from f32.
- `tests/web/fixtures/global-setup.mjs` — Playwright `globalSetup` entrypoint that calls `render.mjs` for both profiles.
- `tests/web/tests/page-loads.spec.ts` — page-health test (no audio).
- `tests/web/tests/roundtrip.spec.ts` — full pipeline test, parameterised over `["audible", "ultrasonic"]`.
- `tests/web/README.md` — short usage doc.
- `scripts/test-web.sh` — orchestrator: ensures wasm is built, runs Playwright.

### No existing files are modified.

---

## Phase 1: Scaffold

### Task 1: Bootstrap the Playwright project

**Files:**
- Create: `tests/web/package.json`
- Create: `tests/web/.gitignore`

- [ ] **Step 1: Create `tests/web/` and the package manifest**

Run:
```bash
mkdir -p /Users/ke/src/modem/tests/web/fixtures /Users/ke/src/modem/tests/web/tests
```

Create `/Users/ke/src/modem/tests/web/package.json`:

```json
{
  "name": "modem-web-tests",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "scripts": {
    "test": "playwright test"
  },
  "devDependencies": {
    "@playwright/test": "^1.50.0"
  }
}
```

- [ ] **Step 2: Create the gitignore**

Create `/Users/ke/src/modem/tests/web/.gitignore`:

```
node_modules/
playwright-report/
test-results/
fixtures/*.wav
```

- [ ] **Step 3: Install Playwright + the Chromium browser**

Run from `/Users/ke/src/modem/tests/web`:
```bash
npm install
npx playwright install chromium
```

Expected: `npm install` resolves `@playwright/test`; `playwright install chromium` reports a single browser download (~170 MB on first run).

- [ ] **Step 4: Sanity-check `playwright` runs**

Run from `/Users/ke/src/modem/tests/web`:
```bash
npx playwright --version
```
Expected: prints `Version 1.x.y`.

- [ ] **Step 5: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: scaffold Playwright project"
jj new
```

---

### Task 2: Playwright config + dummy spec to validate `webServer`

**Files:**
- Create: `tests/web/playwright.config.ts`
- Create: `tests/web/tests/smoke.spec.ts` (temporary; removed by Task 4)

- [ ] **Step 1: Write the config**

Create `/Users/ke/src/modem/tests/web/playwright.config.ts`:

```ts
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false, // audio devices don't enjoy contention
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: "http://localhost:8765",
    trace: "on-first-retry",
  },
  webServer: {
    command: "python3 -m http.server -d ../../web 8765",
    url: "http://localhost:8765/index.html",
    reuseExistingServer: false,
    timeout: 30_000,
    stdout: "pipe",
    stderr: "pipe",
  },
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
    },
  ],
  timeout: 60_000,
});
```

- [ ] **Step 2: Write a temporary smoke test to validate the webServer wires up**

Create `/Users/ke/src/modem/tests/web/tests/smoke.spec.ts`:

```ts
import { test, expect } from "@playwright/test";

test("static server is up and serves index.html", async ({ page }) => {
  const response = await page.goto("/");
  expect(response?.status()).toBe(200);
  await expect(page.locator("h1")).toContainText("modem · web POC");
});
```

- [ ] **Step 3: Make sure the wasm artifact exists (required by the page)**

Run from `/Users/ke/src/modem`:
```bash
ls web/pkg/modem_wasm.js web/pkg/modem_wasm_bg.wasm
```

If either is missing, run:
```bash
./scripts/build-wasm.sh
```

Expected: both files exist.

- [ ] **Step 4: Run the smoke test**

Run from `/Users/ke/src/modem/tests/web`:
```bash
npx playwright test
```

Expected: `1 passed`. The console shows the `python3 -m http.server` log being captured.

- [ ] **Step 5: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: Playwright config + smoke test"
jj new
```

---

## Phase 2: Fixtures

### Task 3: `render.mjs` — generate `<profile>.s16.wav` from `cargo tx-wav`

**Files:**
- Create: `tests/web/fixtures/render.mjs`

- [ ] **Step 1: Write the fixture generator**

Create `/Users/ke/src/modem/tests/web/fixtures/render.mjs`:

```js
// Generate a 16-bit PCM WAV fixture by invoking `modem tx-wav` and
// post-converting from the f32 format `hound` writes. 16-bit PCM is the
// format Chromium's `--use-file-for-fake-audio-capture` accepts reliably.
//
// Used at Playwright globalSetup time; safe to call repeatedly (mtime cache).

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, statSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = path.resolve(__dirname, "../../..");
const FIXTURES = __dirname;

/** Render a fixture; returns the absolute path to the s16 WAV. */
export function renderFixture(profile, message) {
  if (profile !== "audible" && profile !== "ultrasonic") {
    throw new Error(`unknown profile: ${profile}`);
  }
  mkdirSync(FIXTURES, { recursive: true });
  const inputTxt = path.join(FIXTURES, `${profile}.input.txt`);
  const wavF32 = path.join(FIXTURES, `${profile}.f32.wav`);
  const wavS16 = path.join(FIXTURES, `${profile}.s16.wav`);

  // Skip if the input hasn't changed and the s16 WAV already exists.
  const inputUnchanged =
    existsSync(inputTxt) && readFileSync(inputTxt, "utf8") === message;
  if (inputUnchanged && existsSync(wavS16) && existsSync(wavF32)
      && statSync(wavS16).mtimeMs >= statSync(wavF32).mtimeMs) {
    return wavS16;
  }

  writeFileSync(inputTxt, message, "utf8");

  // Shell out to the CLI. `--release` so subsequent runs reuse the cache.
  const res = spawnSync(
    "cargo",
    [
      "run",
      "--release",
      "--quiet",
      "-p",
      "modem-cli",
      "--",
      "tx-wav",
      "--profile",
      profile,
      inputTxt,
      wavF32,
    ],
    { cwd: REPO_ROOT, stdio: ["ignore", "inherit", "inherit"] },
  );
  if (res.status !== 0) {
    throw new Error(`cargo tx-wav exited ${res.status}`);
  }

  convertF32ToS16(wavF32, wavS16);
  return wavS16;
}

/** Read a 32-bit float WAV and write a 16-bit PCM WAV with the same sample rate. */
function convertF32ToS16(src, dst) {
  const buf = readFileSync(src);
  // Minimal WAV parser. The file `hound` writes has the form:
  //   RIFF <size> WAVE fmt  <fmtsize> <fmtdata...> [optional chunks] data <size> <samples...>
  // fmt for IEEE float: audioFormat=3, bitsPerSample=32.
  if (buf.toString("ascii", 0, 4) !== "RIFF" || buf.toString("ascii", 8, 12) !== "WAVE") {
    throw new Error("not a RIFF/WAVE file: " + src);
  }
  let pos = 12;
  let sampleRate = 0;
  let channels = 0;
  let dataOff = -1;
  let dataLen = 0;
  while (pos + 8 <= buf.length) {
    const id = buf.toString("ascii", pos, pos + 4);
    const size = buf.readUInt32LE(pos + 4);
    if (id === "fmt ") {
      const audioFormat = buf.readUInt16LE(pos + 8);
      channels = buf.readUInt16LE(pos + 10);
      sampleRate = buf.readUInt32LE(pos + 12);
      const bits = buf.readUInt16LE(pos + 22);
      if (audioFormat !== 3 || bits !== 32) {
        throw new Error(`expected IEEE float WAV (fmt 3, 32-bit); got fmt ${audioFormat} bits ${bits}`);
      }
    } else if (id === "data") {
      dataOff = pos + 8;
      dataLen = size;
      break;
    }
    pos += 8 + size + (size % 2);
  }
  if (dataOff < 0) throw new Error("no data chunk in " + src);
  if (channels !== 1) throw new Error("expected mono, got " + channels + " channels");

  const sampleCount = Math.floor(dataLen / 4);
  const out = Buffer.alloc(44 + sampleCount * 2);
  // RIFF header
  out.write("RIFF", 0, "ascii");
  out.writeUInt32LE(36 + sampleCount * 2, 4);
  out.write("WAVE", 8, "ascii");
  // fmt chunk (16 bytes, PCM)
  out.write("fmt ", 12, "ascii");
  out.writeUInt32LE(16, 16);
  out.writeUInt16LE(1, 20); // PCM
  out.writeUInt16LE(1, 22); // mono
  out.writeUInt32LE(sampleRate, 24);
  out.writeUInt32LE(sampleRate * 2, 28); // byte rate
  out.writeUInt16LE(2, 32); // block align
  out.writeUInt16LE(16, 34); // bits per sample
  // data chunk
  out.write("data", 36, "ascii");
  out.writeUInt32LE(sampleCount * 2, 40);
  for (let i = 0; i < sampleCount; i++) {
    const f = buf.readFloatLE(dataOff + i * 4);
    let s = Math.round(Math.max(-1, Math.min(1, f)) * 32767);
    out.writeInt16LE(s, 44 + i * 2);
  }
  writeFileSync(dst, out);
}
```

- [ ] **Step 2: Smoke-run the generator from the command line**

Run from `/Users/ke/src/modem`:
```bash
node --input-type=module -e "
  import('./tests/web/fixtures/render.mjs').then(m => {
    const p = m.renderFixture('audible', 'hello from playwright');
    console.log('OK:', p);
  });
"
```

Expected: prints `OK: /Users/ke/src/modem/tests/web/fixtures/audible.s16.wav`. The file exists and is `~ payload_bytes * 2 + 44` bytes more than 1 KB (for the modem this is hundreds of KB).

Verify:
```bash
ls -lh tests/web/fixtures/audible.s16.wav
file tests/web/fixtures/audible.s16.wav
```
Expected: `… RIFF (little-endian) data, WAVE audio, Microsoft PCM, 16 bit, mono 48000 Hz`.

- [ ] **Step 3: Smoke-run for ultrasonic too**

Run from `/Users/ke/src/modem`:
```bash
node --input-type=module -e "
  import('./tests/web/fixtures/render.mjs').then(m => {
    console.log(m.renderFixture('ultrasonic', 'hello from playwright'));
  });
"
```
Expected: prints the ultrasonic fixture path and the file exists.

- [ ] **Step 4: Verify the second invocation is a cache hit**

Run again:
```bash
time node --input-type=module -e "
  import('./tests/web/fixtures/render.mjs').then(m => {
    console.log(m.renderFixture('audible', 'hello from playwright'));
  });
"
```
Expected: completes in well under 1 second (no cargo invocation).

- [ ] **Step 5: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: render.mjs fixture generator (f32 → s16 WAV)"
jj new
```

---

### Task 4: Wire `globalSetup` to pre-render fixtures

**Files:**
- Create: `tests/web/fixtures/global-setup.mjs`
- Modify: `tests/web/playwright.config.ts` (add `globalSetup`)
- Delete: `tests/web/tests/smoke.spec.ts` (no longer needed)

- [ ] **Step 1: Write the globalSetup entrypoint**

Create `/Users/ke/src/modem/tests/web/fixtures/global-setup.mjs`:

```js
import { renderFixture } from "./render.mjs";

export const FIXTURE_MESSAGE = "hello from playwright";

export default async function globalSetup() {
  // Build any fixtures the spec files reference. Idempotent.
  for (const profile of ["audible", "ultrasonic"]) {
    const p = renderFixture(profile, FIXTURE_MESSAGE);
    console.log(`fixture[${profile}] = ${p}`);
  }
}
```

- [ ] **Step 2: Wire it into the Playwright config**

Modify `/Users/ke/src/modem/tests/web/playwright.config.ts` — add `globalSetup` next to the existing `webServer`:

```ts
export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  globalSetup: "./fixtures/global-setup.mjs",
  use: {
    baseURL: "http://localhost:8765",
    trace: "on-first-retry",
  },
  webServer: {
    command: "python3 -m http.server -d ../../web 8765",
    url: "http://localhost:8765/index.html",
    reuseExistingServer: false,
    timeout: 30_000,
    stdout: "pipe",
    stderr: "pipe",
  },
  projects: [
    {
      name: "chromium",
      use: { browserName: "chromium" },
    },
  ],
  timeout: 60_000,
});
```

- [ ] **Step 3: Remove the temporary smoke test**

Run:
```bash
rm /Users/ke/src/modem/tests/web/tests/smoke.spec.ts
```

- [ ] **Step 4: Add a placeholder spec so Playwright doesn't fail with "no tests found"**

Create `/Users/ke/src/modem/tests/web/tests/_placeholder.spec.ts`:

```ts
import { test } from "@playwright/test";

test("globalSetup ran and rendered fixtures", async () => {
  // Real tests will replace this in Tasks 5 and 6. This stub keeps
  // Playwright happy while globalSetup is verified.
});
```

- [ ] **Step 5: Run Playwright and verify globalSetup output**

Run from `/Users/ke/src/modem/tests/web`:
```bash
npx playwright test
```
Expected: console output includes `fixture[audible] = …audible.s16.wav` and `fixture[ultrasonic] = …ultrasonic.s16.wav`. `1 passed`.

- [ ] **Step 6: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: globalSetup renders fixtures before tests"
jj new
```

---

## Phase 3: Real tests

### Task 5: `page-loads.spec.ts` — health check

**Files:**
- Create: `tests/web/tests/page-loads.spec.ts`
- Delete: `tests/web/tests/_placeholder.spec.ts`

- [ ] **Step 1: Write the page-health spec**

Create `/Users/ke/src/modem/tests/web/tests/page-loads.spec.ts`:

```ts
import { test, expect } from "@playwright/test";

test("page loads cleanly, wasm reports 48 kHz, canvas + freq labels render", async ({ page }) => {
  const consoleErrors: string[] = [];
  const pageErrors: string[] = [];
  page.on("console", (msg) => {
    if (msg.type() === "error") consoleErrors.push(msg.text());
  });
  page.on("pageerror", (err) => pageErrors.push(String(err)));

  await page.goto("/");

  // Status footer is set by main.js after wasm init.
  await expect(page.locator("#status")).toHaveText(
    /wasm ready · sampleRate=48000 Hz/,
    { timeout: 15_000 },
  );

  // Tone-bar canvas exists with nonzero width.
  const canvas = page.locator("canvas#tones");
  await expect(canvas).toBeVisible();
  const boxBefore = await canvas.boundingBox();
  expect(boxBefore?.width ?? 0).toBeGreaterThan(0);

  // 8 frequency labels appear under the canvas.
  await expect(page.locator(".freq-labels > span")).toHaveCount(8);

  // No console.error or pageerror events during load.
  expect(consoleErrors, "console.error during load").toEqual([]);
  expect(pageErrors, "pageerror during load").toEqual([]);
});
```

- [ ] **Step 2: Remove the placeholder spec from Task 4**

Run:
```bash
rm /Users/ke/src/modem/tests/web/tests/_placeholder.spec.ts
```

- [ ] **Step 3: Run the test, expect PASS**

Run from `/Users/ke/src/modem/tests/web`:
```bash
npx playwright test page-loads
```
Expected: `1 passed`.

- [ ] **Step 4: Verify the test actually catches a regression**

Temporarily break the page by editing `/Users/ke/src/modem/web/main.js` — find the line:
```js
  els.status.textContent = `wasm ready · sampleRate=${wasmSampleRate()} Hz`;
```
…and change `wasm ready` to `wasm DELIBERATELY BROKEN`. Save.

Re-run the test:
```bash
npx playwright test page-loads
```
Expected: FAIL on the `toHaveText` assertion.

Revert the change (restore `wasm ready`). Re-run:
```bash
npx playwright test page-loads
```
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: page-loads health check spec"
jj new
```

---

### Task 6: `roundtrip.spec.ts` — full pipeline via fake mic, parameterised over profile

**Files:**
- Create: `tests/web/tests/roundtrip.spec.ts`

- [ ] **Step 1: Write the roundtrip spec**

Create `/Users/ke/src/modem/tests/web/tests/roundtrip.spec.ts`:

```ts
import { test, expect, chromium, type Browser, type BrowserContext, type Page } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { FIXTURE_MESSAGE } from "../fixtures/global-setup.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = path.resolve(__dirname, "../fixtures");
const BASE_URL = "http://localhost:8765";
const DECODE_TIMEOUT_MS = 20_000;

const PROFILES = ["audible", "ultrasonic"] as const;

for (const profile of PROFILES) {
  test.describe(`roundtrip · ${profile}`, () => {
    let browser: Browser;
    let context: BrowserContext;
    let page: Page;

    test.beforeAll(async () => {
      const wav = path.join(FIXTURES_DIR, `${profile}.s16.wav`);
      browser = await chromium.launch({
        args: [
          "--use-fake-ui-for-media-stream",
          "--use-fake-device-for-media-stream",
          `--use-file-for-fake-audio-capture=${wav}`,
          "--autoplay-policy=no-user-gesture-required",
        ],
      });
      context = await browser.newContext({ baseURL: BASE_URL });
      page = await context.newPage();
    });

    test.afterAll(async () => {
      await context?.close();
      await browser?.close();
    });

    test(`decodes "${FIXTURE_MESSAGE}"`, async () => {
      await page.goto("/");
      await expect(page.locator("#status")).toHaveText(
        /wasm ready · sampleRate=48000 Hz/,
        { timeout: 15_000 },
      );

      // Select the matching profile radio.
      await page.locator(`input[name="profile"][value="${profile}"]`).check();

      // Start listening; mic permission is auto-granted by --use-fake-ui-for-media-stream.
      await page.locator("#listen-btn").click();
      await expect(page.locator("#mic-status")).toHaveText("mic live");

      // Wait for the decoded message to appear in the log. Class log-recv (without
      // `bad`) means stream_complete fired with sha256_ok=true.
      const recv = page
        .locator(".log-line.log-recv", { hasText: FIXTURE_MESSAGE })
        .first();
      await expect(recv).toBeVisible({ timeout: DECODE_TIMEOUT_MS });

      // Class assertion: should be log-recv only, NOT log-recv bad (sha mismatch).
      const cls = (await recv.getAttribute("class")) ?? "";
      expect(cls.split(/\s+/)).toContain("log-recv");
      expect(cls.split(/\s+/)).not.toContain("bad");
    });
  });
}
```

- [ ] **Step 2: Run both roundtrip tests**

Run from `/Users/ke/src/modem/tests/web`:
```bash
npx playwright test roundtrip
```
Expected: `2 passed` (one per profile). Each test takes ~15–20 s because the modem itself transmits at ~14 s per frame.

If a test fails with "decoded text never appeared", the most likely causes are:
- Chromium didn't accept the s16 WAV — re-run `node --input-type=module -e "import('./tests/web/fixtures/render.mjs').then(m => console.log(m.renderFixture('audible','hello from playwright')))"` and inspect the file with `file` — the magic should be `WAVE audio, Microsoft PCM, 16 bit, mono 48000 Hz`.
- `AudioContext` returned 44.1 kHz — the page logs `⚠ AudioContext returned …` in that case; this also fails `page-loads.spec.ts` so check that first.

- [ ] **Step 3: Verify the test catches a content regression**

Temporarily change the assertion expectation:
```ts
const recv = page.locator(".log-line.log-recv", { hasText: "wrong message" }).first();
```

Re-run:
```bash
npx playwright test roundtrip
```
Expected: FAIL — `Locator ".log-line.log-recv" matching "wrong message" was not visible within 20000 ms`.

Revert the test edit. Re-run; expect `2 passed`.

- [ ] **Step 4: Verify the test catches a sha mismatch**

Temporarily change one byte of the fixture WAV to force a mismatched sha:

```bash
# Overwrite byte 1000 (deep inside the audio payload, past the header)
python3 -c "
data = open('/Users/ke/src/modem/tests/web/fixtures/audible.s16.wav', 'rb').read()
data = bytearray(data)
data[1000] ^= 0xFF
open('/Users/ke/src/modem/tests/web/fixtures/audible.s16.wav', 'wb').write(data)
"
```

Re-run just the audible test:
```bash
npx playwright test roundtrip --grep audible
```
Expected: FAIL — either timeout (no decode at all due to enough errors), or class assertion fails (`bad` class set because sha mismatch).

Restore the fixture by regenerating:
```bash
rm /Users/ke/src/modem/tests/web/fixtures/audible.*
node --input-type=module -e "
  import('./tests/web/fixtures/render.mjs').then(m => m.renderFixture('audible','hello from playwright'));
"
```

Re-run; expect `1 passed`.

- [ ] **Step 5: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: roundtrip spec via fake-mic WAV (audible + ultrasonic)"
jj new
```

---

## Phase 4: Orchestration + docs

### Task 7: `scripts/test-web.sh` — one-command runner

**Files:**
- Create: `scripts/test-web.sh`

- [ ] **Step 1: Write the orchestrator**

Create `/Users/ke/src/modem/scripts/test-web.sh`:

```bash
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
```

- [ ] **Step 2: Make it executable**

Run:
```bash
chmod +x /Users/ke/src/modem/scripts/test-web.sh
```

- [ ] **Step 3: Run it from a clean state to validate end-to-end**

Run from `/Users/ke/src/modem`:
```bash
./scripts/test-web.sh
```
Expected: `3 passed` (page-loads + 2 roundtrip cases). Total wall time ~45–60 s.

- [ ] **Step 4: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: scripts/test-web.sh one-command runner"
jj new
```

---

### Task 8: `tests/web/README.md`

**Files:**
- Create: `tests/web/README.md`

- [ ] **Step 1: Write the README**

Create `/Users/ke/src/modem/tests/web/README.md`:

```markdown
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
```

- [ ] **Step 2: Commit**

```bash
cd /Users/ke/src/modem
jj desc -m "tests/web: README"
jj new
```

---

## Self-Review

**1. Spec coverage**

| Spec section | Task(s) |
|---|---|
| Playwright project scaffold (`package.json`, `.gitignore`) | Task 1 |
| `playwright.config.ts` with `webServer` + `globalSetup` | Tasks 2, 4 |
| `fixtures/render.mjs` (f32 → s16) | Task 3 |
| `fixtures/global-setup.mjs` | Task 4 |
| `tests/page-loads.spec.ts` (no audio, page health) | Task 5 |
| `tests/roundtrip.spec.ts` parameterised over `[audible, ultrasonic]` | Task 6 |
| Decode timeout = 20 s | Task 6 (`DECODE_TIMEOUT_MS = 20_000`) |
| Chromium flags `--use-fake-ui`, `--use-fake-device`, `--use-file-for-fake-audio-capture` | Task 6 |
| Assert class is `log-recv` (not `bad`) | Task 6 |
| `scripts/test-web.sh` orchestrator | Task 7 |
| `tests/web/README.md` | Task 8 |
| Test the test (deliberate regression flips test red) | Tasks 5 (Step 4), 6 (Steps 3 + 4) |
| f32 WAV vs Chromium format compatibility — s16 fallback | Task 3 |
| `AudioContext` sample-rate failure surfacing in `page-loads` not `roundtrip` | Task 5 (`toHaveText(/sampleRate=48000 Hz/)`) |

All sections have at least one task. The "headless Chromium audio backend on macOS" risk noted in the spec isn't directly addressed by a task — if the user hits it, the documented workaround in the spec (`headless: false`) is the answer; not adding a task for a speculative platform issue.

**2. Placeholder scan:** No "TBD", "TODO", "later", "appropriate", "handle edge cases". All code blocks are complete and runnable.

**3. Type consistency:** `FIXTURE_MESSAGE` exported from `global-setup.mjs` and imported in `roundtrip.spec.ts`. `renderFixture(profile, message)` signature consistent across `render.mjs` exporters and `global-setup.mjs` consumer. `<profile>.s16.wav` naming consistent across `render.mjs`, `global-setup.mjs`, and `roundtrip.spec.ts`. Port `8765` consistent across `playwright.config.ts` and the page's own `web/README.md`.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-05-21-web-poc-e2e-test.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task with review between tasks, fast iteration.
2. **Inline Execution** — execute tasks in this session using executing-plans, batch execution with checkpoints.

Which approach?
