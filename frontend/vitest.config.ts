import { defineConfig } from "vitest/config";
import path from "path";

const repoRoot = path.resolve(__dirname, "..");

export default defineConfig({
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  server: {
    fs: {
      allow: [repoRoot],
    },
  },
  test: {
    environment: "jsdom",
    coverage: {
      provider: "v8",
      include: ["src/**/*.{ts,tsx}"],
      exclude: ["src/vite-env.d.ts", "src/main.tsx"],
    },
    include: [
      path.join(__dirname, "src/**/*.test.{ts,tsx}"),
      path.join(repoRoot, "tests/behavioral/ui/**/*.test.ts"),
      path.join(repoRoot, "tests/implementation/ui/**/*.test.ts"),
    ],
  },
});
