import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import StatusPage from "@/pages/system/status/StatusPage";
import { setToken, clearToken } from "@/api/client";
import { useAuthStore } from "@/stores/auth";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import type { HealthCheckResult } from "@/types/api";

const VERSION = "9.9.9-status-test";

const systemStatus: StubReply = {
  status: 200,
  body: {
    version: VERSION,
    osInfo: "linux x86_64",
    dataDirectory: "/config",
    logFile: "/config/logs/livrarr.log.2026-10-06",
    startupTime: "2026-10-06T00:00:00Z",
    logLevel: "info",
    rssBytes: null,
  },
};

const healthSummary: StubReply = {
  status: 200,
  body: {
    llm: { configured: false, enabled: false, provider: null, model: null },
    indexers: [],
    downloadClients: [],
    rssSync: { running: false, lastRunAt: null },
    metadataProviders: [],
    library: { workCount: 0, libraryItemCount: 0, totalSizeBytes: 0 },
  },
};

const databaseOk: HealthCheckResult = {
  source: "database",
  checkType: "ok",
  message: "database is reachable",
};

/** The public health reply as the server sends it while the database answers. */
const publicHealthOk: StubReply = { status: 200, body: [databaseOk] };

const PROXY_WARNING =
  'Ignored [server] trusted_proxies entry "nginx": use an IP address or range such as 172.18.0.0/16; host names and ports are not supported';

const DATABASE_FAILURE =
  "database check failed: did not answer within 2 seconds";

function signInAdmin() {
  setToken("test-session");
  useAuthStore.setState({
    status: "authenticated",
    token: "test-session",
    isAdmin: true,
    user: {
      id: 1,
      username: "owner",
      role: "admin",
      createdAt: "2026-10-01T00:00:00Z",
      updatedAt: "2026-10-01T00:00:00Z",
    },
  });
}

const unstubbed = (call: ApiCall): StubReply => ({
  status: 404,
  body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
});

/**
 * The page's reads as the server answers an admin. `adminHealth` is the
 * admin-only health list; the public `/health` route answers as it does for
 * a working database; `/system/status` answers with `statusReply`.
 */
function statusApi(
  adminHealth: HealthCheckResult[],
  statusReply: StubReply = systemStatus,
) {
  return installApiStub((call) => {
    if (call.method !== "GET") return unstubbed(call);
    switch (call.path) {
      case "/system/status":
        return statusReply;
      case "/system/health-summary":
        return healthSummary;
      case "/system/health":
        return { status: 200, body: adminHealth };
      case "/health":
        return publicHealthOk;
      default:
        return unstubbed(call);
    }
  });
}

function mountStatus(api: ReturnType<typeof installApiStub>) {
  const mounted = mountWith(newTestClient(), <StatusPage />);
  return {
    ...mounted,
    cleanup: () => {
      mounted.cleanup();
      api.restore();
    },
  };
}

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

/** The value shown in the system-info row labelled `label`, or null when absent. */
function infoRow(scope: HTMLElement, label: string): string | null {
  const term = Array.from(scope.querySelectorAll("dt")).find(
    (dt) => dt.textContent?.trim() === label,
  );
  return term?.nextElementSibling?.textContent ?? null;
}

/** The Health Checks row whose text contains `text`, or undefined. */
function healthRow(scope: HTMLElement, text: string): HTMLLIElement | undefined {
  return Array.from(scope.querySelectorAll("li")).find((li) =>
    (li.textContent ?? "").includes(text),
  );
}

/** A row's check-type label: its text and colour classes. */
function rowLabel(row: HTMLLIElement | undefined) {
  if (!row) return null;
  const label = Array.from(row.querySelectorAll("span")).find((s) =>
    ["ok", "warning", "error"].includes(s.textContent?.trim() ?? ""),
  );
  if (!label) return null;
  return {
    text: label.textContent?.trim(),
    amber: label.classList.contains("text-amber-400"),
    red: label.classList.contains("text-red-400"),
  };
}

const requested = (calls: ApiCall[], path: string) =>
  calls.filter((c) => c.method === "GET" && c.path === path).length;

let cleanup: () => void = () => {};
afterEach(() => {
  cleanup();
  cleanup = () => {};
  clearToken();
  useAuthStore.setState({
    status: "loading",
    user: null,
    token: null,
    isAdmin: false,
  });
});

describe("System → Status: the Health Checks list comes from the admin health route", () => {
  it("requests GET /api/v1/system/health and never GET /api/v1/health", async () => {
    signInAdmin();
    const api = statusApi([databaseOk]);
    const mounted = mountStatus(api);
    cleanup = mounted.cleanup;
    await settle();

    // Controls: the page loaded its system information.
    await vi.waitFor(() =>
      expect(infoRow(mounted.container, "Version")).toBe(VERSION),
    );
    await settle();
    expect(requested(api.calls, "/system/status")).toBeGreaterThan(0);

    expect({
      adminHealthReads: requested(api.calls, "/system/health") > 0,
      publicHealthReads: requested(api.calls, "/health"),
    }).toEqual({ adminHealthReads: true, publicHealthReads: 0 });
  });

  it("shows a config warning from the admin reply in an amber row labelled warning", async () => {
    signInAdmin();
    const api = statusApi([
      databaseOk,
      { source: "config", checkType: "warning", message: PROXY_WARNING },
    ]);
    const mounted = mountStatus(api);
    cleanup = mounted.cleanup;
    await settle();

    await vi.waitFor(() =>
      expect(infoRow(mounted.container, "Version")).toBe(VERSION),
    );
    await settle();

    expect(rowLabel(healthRow(mounted.container, PROXY_WARNING))).toEqual({
      text: "warning",
      amber: true,
      red: false,
    });
  });

  it("shows a failed database check as a red row and still shows the Version row", async () => {
    signInAdmin();
    const api = statusApi([
      { source: "database", checkType: "error", message: DATABASE_FAILURE },
    ]);
    const mounted = mountStatus(api);
    cleanup = mounted.cleanup;
    await settle();

    await vi.waitFor(() =>
      expect(infoRow(mounted.container, "Version")).toBe(VERSION),
    );
    await settle();
    expect(infoRow(mounted.container, "Version")).toBe(VERSION);

    expect(rowLabel(healthRow(mounted.container, DATABASE_FAILURE))).toEqual({
      text: "error",
      amber: false,
      red: true,
    });
  });
});

describe("System → Status when the signed-in reads fail", () => {
  it("with only /system/status answering 500, shows Something went wrong and Retry", async () => {
    signInAdmin();
    const api = statusApi([databaseOk], { status: 500 });
    const mounted = mountStatus(api);
    cleanup = mounted.cleanup;
    await settle();

    // Control: the page asked for its system information.
    await vi.waitFor(() =>
      expect(requested(api.calls, "/system/status")).toBeGreaterThan(0),
    );
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Something went wrong"),
    );

    const buttons = Array.from(mounted.container.querySelectorAll("button")).map(
      (b) => b.textContent?.trim() ?? "",
    );
    expect(buttons).toContain("Retry");
  });

  it("with every signed-in read answering 500 and the public health 503, shows Something went wrong and Retry", async () => {
    signInAdmin();
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/health") {
        return {
          status: 503,
          body: [
            {
              source: "database",
              checkType: "error",
              message: "database check failed",
            },
          ],
        };
      }
      if (call.method === "GET" && call.path.startsWith("/system/")) {
        return { status: 500 };
      }
      return unstubbed(call);
    });
    const mounted = mountStatus(api);
    cleanup = mounted.cleanup;
    await settle();

    // Control: the page asked for its system information.
    await vi.waitFor(() =>
      expect(requested(api.calls, "/system/status")).toBeGreaterThan(0),
    );
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Something went wrong"),
    );

    const buttons = Array.from(mounted.container.querySelectorAll("button")).map(
      (b) => b.textContent?.trim() ?? "",
    );
    expect(buttons).toContain("Retry");
  });
});
