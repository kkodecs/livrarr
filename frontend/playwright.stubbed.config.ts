import { defineConfig, devices } from "@playwright/test";

// Browser tests for the readers and the audio player: epub.js rendering,
// pdf.js and audio playback need a real browser. The working tree is built
// once and served by `vite preview` (a production bundle with no file watcher,
// so edits made while the tests run cannot reload the page). Every /api request
// is answered inside the test with page.route, so no Livrarr server or
// database is involved.
//
//   pnpm exec playwright test --config playwright.stubbed.config.ts
const PORT = 5317;
const OUT_DIR = "node_modules/.e2e-stubbed-ui";

export default defineConfig({
  testDir: "e2e-stubbed",
  fullyParallel: true,
  workers: 4,
  retries: 0,
  reporter: "list",
  timeout: 120_000,
  use: {
    baseURL: `http://localhost:${PORT}`,
    trace: "retain-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        launchOptions: { args: ["--autoplay-policy=no-user-gesture-required"] },
      },
    },
  ],
  webServer: {
    command: `pnpm exec vite build --outDir ${OUT_DIR} --logLevel warn && pnpm exec vite preview --outDir ${OUT_DIR} --port ${PORT} --strictPort`,
    url: `http://localhost:${PORT}/`,
    reuseExistingServer: false,
    timeout: 240_000,
  },
});
