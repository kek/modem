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
