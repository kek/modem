import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false, // audio devices don't enjoy contention
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
