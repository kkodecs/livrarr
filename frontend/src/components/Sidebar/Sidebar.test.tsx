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

const adminUser = {
  id: 1,
  username: "admin",
  role: "admin" as const,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
};
const normalUser = { ...adminUser, id: 2, username: "reader", role: "user" as const };

function signInAs(role: "admin" | "user") {
  useAuthStore.setState({
    status: "authenticated",
    user: role === "admin" ? adminUser : normalUser,
    isAdmin: role === "admin",
  });
}

/**
 * The four ways the menu is drawn: the desktop sidebar expanded or collapsed,
 * and the phone drawer, which follows the same collapsed setting.
 */
const menuModes = [
  { name: "desktop expanded", drawer: false, collapsed: false },
  { name: "desktop collapsed", drawer: false, collapsed: true },
  { name: "phone drawer", drawer: true, collapsed: false },
  { name: "phone drawer, collapsed setting on", drawer: true, collapsed: true },
] as const;

type MenuMode = (typeof menuModes)[number];

const REMOVED = [
  "Bookshelf",
  "Calendar",
  "Cutoff Unmet",
  "Profiles",
  "Custom Formats",
  "Development",
];
const ADMIN_PAGES = {
  "Manual Import": "/import",
  "Readarr Import": "/import/readarr",
  "Media Management": "/settings/mediamanagement",
  Status: "/system/status",
  Logs: "/system/logs",
} as const;

/** Mount the real sidebar for a role and mode; expanded groups are opened. */
async function openMenu(role: "admin" | "user", mode: MenuMode) {
  signInAs(role);
  useUIStore.setState({
    sidebarCollapsed: mode.collapsed,
    mobileSidebarOpen: mode.drawer,
  });
  const view = mountSidebar(() => healthy);
  cleanup = view.cleanup;
  await advance(0);
  const asides = document.querySelectorAll("aside");
  const aside = mode.drawer ? asides[0]! : asides[1]!;
  const nav = aside.querySelector("nav")!;
  // Collapsible group headings: open each one so every item is drawn.
  for (const heading of Array.from(nav.querySelectorAll("button"))) {
    await act(async () => {
      heading.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
  }
  return nav;
}

/** Labels of the greyed "Coming Soon" items. */
function greyedLabels(nav: Element): string[] {
  return Array.from(nav.querySelectorAll('[title="Coming Soon"]'))
    .map((el) => el.textContent?.trim() ?? "")
    .sort();
}

/** Labels of every menu item, linked or greyed, as drawn in an expanded menu. */
function itemLabels(nav: Element): string[] {
  return Array.from(nav.querySelectorAll('a, [title="Coming Soon"]')).map(
    (el) => el.textContent?.trim() ?? "",
  );
}

/** Group headings: the pinned headings and the collapsible ones. */
function headings(nav: Element): string[] {
  return Array.from(nav.querySelectorAll("button, div.uppercase")).map(
    (el) => el.textContent?.trim() ?? "",
  );
}

const linkTo = (nav: Element, path: string) =>
  nav.querySelector(`a[href="${path}"]`);

/** The menu group (a direct child of the nav) that holds a given link. */
function groupOf(nav: Element, path: string): Element | null {
  const link = linkTo(nav, path);
  if (!link) return null;
  return Array.from(nav.children).find((group) => group.contains(link)) ?? null;
}

describe("Menu items that do nothing are removed", () => {
  afterEach(() => {
    useUIStore.setState({ mobileSidebarOpen: false });
    useAuthStore.setState({ status: "loading", user: null });
  });

  for (const mode of menuModes) {
    it(`admin, ${mode.name}: only General, Notifications and Tags stay greyed, the six removed items and the More heading are gone`, async () => {
      const nav = await openMenu("admin", mode);
      // Control: the menu drew for an admin (an admin-only link is present).
      expect(linkTo(nav, "/settings/indexers")).not.toBeNull();
      if (mode.collapsed) {
        expect(nav.querySelectorAll('[title="Coming Soon"]').length).toBe(3);
        for (const greyed of Array.from(
          nav.querySelectorAll('[title="Coming Soon"]'),
        )) {
          expect(greyed.closest("a")).toBeNull();
        }
      } else {
        expect(greyedLabels(nav)).toEqual(["General", "Notifications", "Tags"]);
        for (const greyed of Array.from(
          nav.querySelectorAll('[title="Coming Soon"]'),
        )) {
          expect(greyed.closest("a")).toBeNull();
        }
        const labels = itemLabels(nav);
        expect(labels.filter((l) => REMOVED.includes(l))).toEqual([]);
        expect(headings(nav)).not.toContain("More");
      }
    });

    it(`normal user, ${mode.name}: only Tags stays greyed, the six removed items and the More heading are gone`, async () => {
      const nav = await openMenu("user", mode);
      // Control: the menu drew for a normal user (no admin-only link).
      expect(linkTo(nav, "/")).not.toBeNull();
      expect(linkTo(nav, "/settings/indexers")).toBeNull();
      if (mode.collapsed) {
        expect(nav.querySelectorAll('[title="Coming Soon"]').length).toBe(1);
      } else {
        expect(greyedLabels(nav)).toEqual(["Tags"]);
        const labels = itemLabels(nav);
        expect(labels.filter((l) => REMOVED.includes(l))).toEqual([]);
        expect(headings(nav)).not.toContain("More");
      }
    });

    it(`admin, ${mode.name}: Unmapped Files is in the Activity group and links to /unmapped`, async () => {
      const nav = await openMenu("admin", mode);
      const activity = groupOf(nav, "/activity/queue");
      expect(activity).not.toBeNull();
      const unmapped = activity!.querySelector('a[href="/unmapped"]');
      expect(unmapped).not.toBeNull();
      if (!mode.collapsed) {
        expect(unmapped!.textContent?.trim()).toBe("Unmapped Files");
      }
    });

    it(`normal user, ${mode.name}: no Unmapped Files item`, async () => {
      const nav = await openMenu("user", mode);
      expect(linkTo(nav, "/activity/queue")).not.toBeNull();
      expect(linkTo(nav, "/unmapped")).toBeNull();
      expect(nav.textContent).not.toContain("Unmapped Files");
    });

    it(`normal user, ${mode.name}: the five admin pages are not offered; Download Clients, UI and About Livrarr are`, async () => {
      const nav = await openMenu("user", mode);
      const offered = (path: string) => linkTo(nav, path) !== null;
      expect({
        downloadClients: offered("/settings/downloadclients"),
        ui: offered("/settings/ui"),
        about: offered("/system/about"),
      }).toEqual({ downloadClients: true, ui: true, about: true });
      expect(
        Object.entries(ADMIN_PAGES)
          .filter(([, path]) => offered(path))
          .map(([label]) => label),
      ).toEqual([]);
      if (!mode.collapsed) {
        const labels = itemLabels(nav);
        expect(
          labels.filter((l) => Object.keys(ADMIN_PAGES).includes(l)),
        ).toEqual([]);
      }
    });

    it(`admin, ${mode.name}: the five admin pages are offered`, async () => {
      const nav = await openMenu("admin", mode);
      expect(
        Object.entries(ADMIN_PAGES)
          .filter(([, path]) => linkTo(nav, path) === null)
          .map(([label]) => label),
      ).toEqual([]);
    });
  }
});
