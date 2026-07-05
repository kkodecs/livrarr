import { test, expect } from "@playwright/test";

// First smoke flow: the app loads and the React shell renders.
// Robust to auth/setup state — the title is static and #root is populated once
// React mounts, whether the rendered route is login, first-run setup, or the app.
test("app shell loads and renders", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/Livrarr/i);
  await expect(page.locator("#root")).not.toBeEmpty();
});
