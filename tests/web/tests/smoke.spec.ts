import { test, expect } from "@playwright/test";

test("static server is up and serves index.html", async ({ page }) => {
  const response = await page.goto("/");
  expect(response?.status()).toBe(200);
  await expect(page.locator("h1")).toContainText("modem · web POC");
});
