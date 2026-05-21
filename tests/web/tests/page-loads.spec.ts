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
