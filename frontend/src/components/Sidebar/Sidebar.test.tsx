import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Vite's build defines this constant; the test config does not, and the
// sidebar reads it when its module loads.
vi.hoisted(() => {
  (globalThis as Record<string, unknown>).__APP_VERSION__ = "0.0.0-test";
});

import { act } from "react";
import { Sidebar } from "@/components/Sidebar/Sidebar";
import { useAuthStore } from "@/stores/auth";
import { useUIStore } from "@/stores/ui";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

const FAILED = "Health check failed";

const healthy: StubReply = {
  status: 200,
  body: {
    llm: { configured: true, enabled: true, provider: "openai", model: "m" },
    indexers: [{ id: 1, name: "Indexer", implementation: "torznab", enabled: true }],
    downloadClients: [
      { id: 1, name: "Client", implementation: "qbittorrent", enabled: true },
    ],
    rssSync: { running: false, lastRunAt: null },
    metadataProviders: [{ name: "openlibrary", status: "ok", lastError: null }],
    library: { workCount: 1, libraryItemCount: 1, totalSizeBytes: 1 },
  },
};

const failing: StubReply = {
  status: 500,
  body: { status: 500, error: "internal", message: "health check failed" },
};

const healthCalls = (calls: ApiCall[]) =>
  calls.filter((c) => c.path === "/system/health-summary").length;

function mountSidebar(reply: () => StubReply) {
  const api = installApiStub((call) =>
    call.path === "/system/health-summary"
      ? reply()
      : {
          status: 404,
          body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
        },
  );
  const mounted = mountWith(newTestClient(), <Sidebar />);
  return {
    api,
    /** The desktop sidebar, the one that follows the collapsed setting. */
    desktop: () => mounted.container.querySelectorAll("aside")[1]!,
    cleanup: () => {
      mounted.cleanup();
      api.restore();
    },
  };
}

async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
  // Let the reply that a timer started reach the screen.
  for (let i = 0; i < 10; i++) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
  }
}

/** What the desktop sidebar's health widget shows. */
function widget(aside: Element, collapsed: boolean) {
  if (collapsed) {
    return {
      failed: aside.querySelector(`[title="${FAILED}"]`) !== null,
      summary:
        aside.querySelector(
          '[title="All systems ok"], [title="Infrastructure issues detected"]',
        ) !== null,
    };
  }
  const statusLinks = Array.from(
    aside.querySelectorAll('a[href="/system/status"]'),
  );
  return {
    failed: statusLinks.some((a) => a.textContent?.includes(FAILED)),
    summary: (aside.textContent ?? "").includes("DL Clients"),
  };
}

let cleanup: () => void = () => {};

beforeEach(() => {
  vi.useFakeTimers();
  useUIStore.setState({ checkForUpdates: false });
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  useAuthStore.setState({ isAdmin: false });
  useUIStore.setState({ sidebarCollapsed: false, checkForUpdates: true });
});

for (const collapsed of [false, true]) {
  const layout = collapsed ? "collapsed" : "expanded";

  describe(`Health widget for an admin, ${layout} sidebar`, () => {
    beforeEach(() => {
      useAuthStore.setState({ isAdmin: true });
      useUIStore.setState({ sidebarCollapsed: collapsed });
    });

    it("cold failure: the first check fails and shows Health check failed; a later success shows the summary", async () => {
      let reply = failing;
      const view = mountSidebar(() => reply);
      cleanup = view.cleanup;
      await advance(0);
      expect(healthCalls(view.api.calls)).toBe(1);
      expect(widget(view.desktop(), collapsed)).toEqual({
        failed: true,
        summary: false,
      });

      reply = healthy;
      await advance(60_000);
      expect(healthCalls(view.api.calls)).toBe(2);
      expect(widget(view.desktop(), collapsed)).toEqual({
        failed: false,
        summary: true,
      });
    });

    it("stale failure: a failed refetch after a success shows Health check failed, not the old summary", async () => {
      let reply = healthy;
      const view = mountSidebar(() => reply);
      cleanup = view.cleanup;
      await advance(0);
      expect(widget(view.desktop(), collapsed)).toEqual({
        failed: false,
        summary: true,
      });

      reply = failing;
      await advance(60_000);
      expect(healthCalls(view.api.calls)).toBe(2);
      expect(widget(view.desktop(), collapsed)).toEqual({
        failed: true,
        summary: false,
      });

      reply = healthy;
      await advance(60_000);
      expect(healthCalls(view.api.calls)).toBe(3);
      expect(widget(view.desktop(), collapsed)).toEqual({
        failed: false,
        summary: true,
      });
    });
  });
}

describe("Health widget for a non-admin", () => {
  it("sends no health request and shows no widget", async () => {
    useAuthStore.setState({ isAdmin: false });
    const view = mountSidebar(() => healthy);
    cleanup = view.cleanup;
    await advance(0);
    await advance(60_000);
    expect(healthCalls(view.api.calls)).toBe(0);
    for (const aside of document.querySelectorAll("aside")) {
      expect(aside.textContent).not.toContain("DL Clients");
      expect(aside.textContent).not.toContain(FAILED);
    }
  });
});
