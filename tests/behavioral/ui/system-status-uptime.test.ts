import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
// Resolve the same React installation as the frontend from this external test path.
import { act, createElement } from "@/../node_modules/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import StatusPage from "@/pages/system/status/StatusPage";
import * as formatters from "@/utils/format";
import { clearToken, setToken } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import { installApiStub, mountWith, newTestClient } from "@/test-support/apiStub";
import type { HealthCheckResult } from "@/types/api";

// Spec and smoke coverage: frontend/src/utils/format.ts,
// frontend/src/pages/system/status/StatusPage.tsx,
// frontend/src/pages/system/status/StatusPage.test.tsx,
// frontend/src/utils/format.test.ts.
const { formatUptime, formatAbsoluteDate } = formatters;
const now = new Date("2026-10-20T12:00:00Z");
const minute = 60;
const hour = 60 * minute;
const day = 24 * hour;

describe("uptime formatting from the specification", () => {
  it.each([
    [0, "under a minute"],
    [30, "under a minute"],
    [59, "under a minute"],
    [minute, "1m"],
    [15 * minute, "15m"],
    [59 * minute + 59, "59m"],
    [hour, "1h 0m"],
    [3 * hour + 15 * minute, "3h 15m"],
    [23 * hour + 59 * minute, "23h 59m"],
    [2 * day + 3 * hour + 15 * minute, "2d 3h 15m"],
    [day, "1d 0h 0m"],
    [day + 5 * minute, "1d 0h 5m"],
    [100 * day, "100d 0h 0m"],
    [3 * hour + 15 * minute + 59, "3h 15m"],
    [-hour, "under a minute"],
  ])("formats %i seconds as %s", (seconds, expected) => {
    const startup = new Date(now.getTime() - Number(seconds) * 1000).toISOString();
    expect(formatUptime(startup, now)).toBe(expected);
  });

  it.each(["2026-10-18T10:45:00+02:00", "2026-10-18T08:45:00Z"])(
    "honors the timezone in %s",
    (startup) => expect(formatUptime(startup, now)).toBe("2d 3h 15m"),
  );
});

const databaseOk: HealthCheckResult = {
  source: "database",
  checkType: "ok",
  message: "database is reachable",
};
const cleanups: Array<() => void> = [];
afterEach(() => {
  while (cleanups.length) cleanups.pop()!();
  clearToken();
  useAuthStore.setState({ status: "loading", user: null, token: null, isAdmin: false });
});

async function mountStatus(options: {
  rssBytes?: number | null;
  ageSeconds?: number;
  health?: HealthCheckResult[];
} = {}) {
  setToken("uptime-spec-session");
  useAuthStore.setState({
    status: "authenticated", token: "uptime-spec-session", isAdmin: true,
    user: {
      id: 1, username: "owner", role: "admin",
      createdAt: "2026-10-01T00:00:00Z", updatedAt: "2026-10-01T00:00:00Z",
    },
  });
  // Real timers keep react-query scheduling live; 30 seconds leaves ample margin.
  const status = {
    version: "uptime-independent-spec", osInfo: "linux aarch64",
    dataDirectory: "/uptime/config", logFile: "/uptime/logs/server.log",
    startupTime: new Date(Date.now() - (options.ageSeconds ?? (2 * day + 3 * hour + 15 * minute + 30)) * 1000).toISOString(),
    logLevel: "info", rssBytes: options.rssBytes ?? null,
  };
  const api = installApiStub((call) => {
    if (call.method === "GET") {
      if (call.path === "/system/status") return { status: 200, body: status };
      if (call.path === "/system/health") return { status: 200, body: options.health ?? [] };
      if (call.path === "/system/health-summary") return { status: 200, body: {
        llm: { configured: false, enabled: false, provider: null, model: null },
        indexers: [], downloadClients: [],
        rssSync: { running: false, lastRunAt: null }, metadataProviders: [],
        library: { workCount: 0, libraryItemCount: 0, totalSizeBytes: 0 },
      } };
    }
    throw new Error(`Unexpected API request: ${call.method} ${call.path}`);
  });
  cleanups.push(api.restore);
  const client = newTestClient();
  cleanups.push(() => client.clear());
  const mounted = mountWith(client, createElement(StatusPage));
  cleanups.push(mounted.cleanup);
  await vi.waitFor(async () => {
    await act(async () => { await new Promise((resolve) => setTimeout(resolve, 10)); });
    expect(value(mounted.container, "Version")).toBe(status.version);
  });
  return { ...mounted, status };
}

function value(container: HTMLElement, label: string) {
  const dt = Array.from(container.querySelectorAll("dl dt")).find((term) => term.textContent === label);
  expect(dt?.nextElementSibling?.tagName).toBe("DD");
  return dt?.nextElementSibling?.textContent;
}

describe("System → Status uptime", () => {
  it.each([null, 1048576])("places Uptime directly after Startup Time with rssBytes=%s", async (rssBytes) => {
    const { container } = await mountStatus({ rssBytes });
    expect(Array.from(container.querySelectorAll("dl dt"), (dt) => dt.textContent)).toEqual([
      "Version", "OS", "Data Directory", "Log File", "Startup Time", "Uptime",
      ...(rssBytes === null ? [] : ["Memory (RSS)"]),
    ]);
    expect(value(container, "Uptime")).toBe("2d 3h 15m");
  });

  it("preserves system information, empty-list health text and Infrastructure", async () => {
    const { container, status } = await mountStatus();
    for (const [label, expected] of [
      ["Version", status.version], ["OS", status.osInfo],
      ["Data Directory", status.dataDirectory], ["Log File", status.logFile],
      ["Startup Time", formatAbsoluteDate(status.startupTime)],
    ]) expect(value(container, label)).toBe(expected);
    expect(container.textContent).toContain("All systems healthy");
    expect(Array.from(container.querySelectorAll("h2"), (h) => h.textContent)).toContain("Infrastructure");
  });

  it("preserves the ok row for a nonempty ok-only health list", async () => {
    const { container } = await mountStatus({ health: [databaseOk] });
    const rows = container.querySelectorAll("li");
    expect(rows).toHaveLength(1);
    expect(rows[0].querySelector("span")?.textContent).toBe("ok");
    expect(rows[0].textContent).toContain(databaseOk.source);
    expect(rows[0].textContent).toContain(databaseOk.message);
    expect(container.textContent).not.toContain("All systems healthy");
  });

  it("shows under a minute for a newly started server", async () => {
    const { container } = await mountStatus({ ageSeconds: 30 });
    expect(value(container, "Uptime")).toBe("under a minute");
  });
});

it("smoke: frontend/src/utils/format.ts", () => {
  // Checks frontend/src/utils/format.ts.
  for (const name of ["formatUptime", "formatAbsoluteDate", "formatBytes", "formatDuration"] as const) {
    expect(formatters[name]).toBeTypeOf("function");
  }
  expect(formatters.formatDuration(3900)).toBe("1h 5m");
});

it("smoke: frontend/src/pages/system/status/StatusPage.tsx", async () => {
  // Checks frontend/src/pages/system/status/StatusPage.tsx.
  expect(StatusPage).toBeTypeOf("function");
  const { container, status } = await mountStatus();
  expect(container.querySelector("h1")?.textContent).toBe("Status");
  expect(value(container, "Version")).toBe(status.version);
});

it("smoke: frontend/src/pages/system/status/StatusPage.test.tsx", () => {
  // Checks frontend/src/pages/system/status/StatusPage.test.tsx.
  // Three parents reach the repo root from tests/behavioral/ui.
  const file = path.resolve(__dirname, "../../../frontend/src/pages/system/status/StatusPage.test.tsx");
  expect(existsSync(file)).toBe(true);
  expect(readFileSync(file, "utf8")).toContain("Uptime");
});

it("smoke: frontend/src/utils/format.test.ts", () => {
  // Checks frontend/src/utils/format.test.ts.
  const file = path.resolve(__dirname, "../../../frontend/src/utils/format.test.ts");
  expect(existsSync(file)).toBe(true);
  expect(readFileSync(file, "utf8")).toContain("formatUptime");
});
