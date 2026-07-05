import { defineConfig, devices } from "@playwright/test";

// E2E config for livrarr.
//
// The app is single-origin in deployed/staging form: the Rust backend serves the
// built UI at :8789 (this is what scripts/dev-restart.sh deploys, and what the
// kk-build deploy stage runs e2e against post-deploy). Vite's dev proxy (:8787)
// is only for `pnpm dev` and is not used here.
//
// Override the target with E2E_BASE_URL when pointing at a different instance.
const BASE_URL = process.env.E2E_BASE_URL ?? "http://localhost:8789";

export default defineConfig({
  testDir: "e2e",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: "list",
  use: {
    baseURL: BASE_URL,
    trace: "on-first-retry",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  // Local convenience: reuse a running instance (dev-restart.sh) if one is up,
  // otherwise start the prebuilt backend serving the deployed UI. Post-deploy CI
  // hits an already-running server, so reuseExistingServer short-circuits.
  webServer: {
    command: "cd .. && ./target/debug/livrarr --data ./testdata",
    url: "http://localhost:8789/api/v1/setup/status",
    reuseExistingServer: true,
    timeout: 120_000,
  },
});
