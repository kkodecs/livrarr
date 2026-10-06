import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// Vite's build defines this constant; the test config does not, and the
// sidebar reads it when its module loads.
vi.hoisted(() => {
  (globalThis as Record<string, unknown>).__APP_VERSION__ = "0.0.0-test";
});

import { act } from "react";
import { createRoot } from "react-dom/client";
import {
  installApiStub,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

// The whole app is mounted for real at an address, signed in through the real
// auth store with a stored session token; only the network boundary is
// stubbed. The app keeps one query cache per module load, so every test loads
// a fresh copy of the app's modules: no test sees another test's cached data.

type Role = "admin" | "user";

const user = (role: Role) => ({
  id: role === "admin" ? 1 : 2,
  username: role === "admin" ? "admin" : "reader",
  role,
  createdAt: "2026-01-01T00:00:00Z",
  updatedAt: "2026-01-01T00:00:00Z",
});

const FORBIDDEN: StubReply = {
  status: 403,
  body: { status: 403, error: "forbidden", message: "forbidden" },
};

/** Routes the server refuses to a normal user (`RequireAdmin`). */
const ADMIN_ONLY = new Set([
  "/rootfolder",
  "/remotepathmapping",
  "/config/mediamanagement",
  "/config/naming",
  "/config/email",
  "/config/metadata",
  "/system/status",
  "/system/health",
  "/system/health-summary",
  "/system/logs/tail",
  "/import/readarr/origin",
]);

const empty = (pageSize: number) => ({
  status: 200,
  body: { items: [], total: 0, page: 1, pageSize },
});

const ADMIN_REPLIES: Record<string, StubReply> = {
  "/rootfolder": { status: 200, body: [] },
  "/remotepathmapping": { status: 200, body: [] },
  "/config/mediamanagement": {
    status: 200,
    body: {
      cwaIngestPath: null,
      preferredEbookFormats: [],
      preferredAudiobookFormats: [],
    },
  },
  "/config/naming": {
    status: 200,
    body: {
      authorFolderFormat: "{Author}",
      bookFolderFormat: "{Title}",
      renameFiles: false,
      replaceIllegalChars: true,
    },
  },
  "/config/email": {
    status: 200,
    body: {
      enabled: false,
      smtpHost: "",
      smtpPort: 587,
      encryption: "starttls",
      username: null,
      passwordSet: false,
      fromAddress: null,
      recipientEmail: null,
      sendOnImport: false,
    },
  },
  "/config/metadata": { status: 200, body: { languages: ["en"] } },
  "/system/status": {
    status: 200,
    body: {
      version: "0.0.0-test",
      osInfo: "linux x86_64",
      dataDirectory: "/config",
      logFile: "/config/logs/livrarr.log",
      startupTime: "2026-01-01T00:00:00Z",
      logLevel: "info",
      rssBytes: null,
    },
  },
  "/system/health": {
    status: 200,
    body: [
      { source: "database", checkType: "ok", message: "database is reachable" },
    ],
  },
  "/system/health-summary": {
    status: 200,
    body: {
      llm: { configured: true, enabled: true, provider: "openai", model: "m" },
      indexers: [],
      downloadClients: [],
      rssSync: { running: false, lastRunAt: null },
      metadataProviders: [],
      library: { workCount: 0, libraryItemCount: 0, totalSizeBytes: 0 },
    },
  },
  "/system/logs/tail": { status: 200, body: [] },
  "/import/readarr/origin": { status: 200, body: [] },
};

function stubApi(role: Role) {
  return installApiStub((call: ApiCall): StubReply => {
    const path = call.path.split("?")[0]!;
    if (path === "/auth/me") {
      return { status: 200, body: { user: user(role), authType: "session" } };
    }
    if (path === "/setup/status") {
      return { status: 200, body: { setupRequired: false } };
    }
    if (ADMIN_ONLY.has(path)) {
      return role === "admin" ? ADMIN_REPLIES[path]! : FORBIDDEN;
    }
    if (path === "/work") return empty(1000);
    if (path === "/queue") return empty(50);
    if (path === "/notification") return empty(200);
    if (path === "/config/languages") {
      return { status: 200, body: { languages: ["en"] } };
    }
    if (path === "/health") return { status: 200, body: [] };
    if (path === "/import/readarr/history") {
      return { status: 200, body: { imports: [] } };
    }
    return {
      status: 404,
      body: { status: 404, error: "not_found", message: `unstubbed ${path}` },
    };
  });
}

async function settle(ms = 20) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function waitFor(condition: () => boolean, what: string, timeoutMs = 5000) {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await settle();
  }
}

let teardown: () => void = () => {};

/** Load a fresh app, sign in as the role with a stored session, open the address. */
async function openAs(role: Role, path: string) {
  localStorage.clear();
  localStorage.setItem("livrarr_token", "session-token");
  localStorage.setItem("livrarr-tour-completed", "true");
  localStorage.setItem(
    "livrarr_ui",
    JSON.stringify({ state: { checkForUpdates: false }, version: 0 }),
  );
  vi.resetModules();
  const api = stubApi(role);
  const { App } = await import("@/App");
  const { useAuthStore } = await import("@/stores/auth");
  window.history.pushState({}, "", path);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(<App />);
  });
  teardown = () => {
    act(() => root.unmount());
    container.remove();
    api.restore();
  };
  await waitFor(
    () => useAuthStore.getState().status === "authenticated",
    "sign-in",
  );
  // Control: the session is the role the test names.
  expect(useAuthStore.getState().isAdmin).toBe(role === "admin");
  return { calls: api.calls };
}

const main = () => document.querySelector("main");
const mainText = () => main()?.textContent ?? "";
/** The page's own heading (toolbar h1). */
const pageHeading = () =>
  Array.from(main()?.querySelectorAll("h1") ?? []).map(
    (h) => h.textContent?.trim() ?? "",
  );
/** The title of an empty-state page such as the not-found page. */
const emptyTitle = () =>
  Array.from(main()?.querySelectorAll("h3") ?? []).map(
    (h) => h.textContent?.trim() ?? "",
  );

const sentPaths = (calls: ApiCall[]) =>
  calls.map((c) => c.path.split("?")[0]!);

const COMING = "This feature is coming in a future release.";
const NOT_FOUND_TITLE = "Page Not Found";
const NOT_FOUND_LINE = "This page does not exist.";

beforeEach(() => {
  teardown = () => {};
});

afterEach(() => {
  teardown();
  localStorage.clear();
  window.history.pushState({}, "", "/");
});

const ROLES: Role[] = ["admin", "user"];

describe("Removed placeholder addresses reach the not-found page", () => {
  const removed: Array<[string, string]> = [
    ["/shelf", "Bookshelf"],
    ["/calendar", "Calendar"],
    ["/wanted/cutoff", "Cutoff Unmet"],
    ["/settings/profiles", "Profiles"],
    ["/settings/customformats", "Custom Formats"],
    ["/settings/development", "Development"],
  ];
  for (const role of ROLES) {
    for (const [path, oldTitle] of removed) {
      it(`${role} at ${path} sees Page Not Found, not "${oldTitle}"`, async () => {
        await openAs(role, path);
        await waitFor(() => emptyTitle().length > 0, "a page title");
        expect(window.location.pathname).toBe(path);
        expect(emptyTitle()).toEqual([NOT_FOUND_TITLE]);
        expect(emptyTitle()).not.toContain(oldTitle);
      });
    }
  }

  for (const role of ROLES) {
    it(`${role} at /wanted/missing still opens Missing`, async () => {
      await openAs(role, "/wanted/missing");
      await waitFor(() => emptyTitle().length > 0, "the Missing page");
      expect(emptyTitle()).toEqual(["No missing items"]);
    });

    it(`${role} at /settings/tags still shows Tags`, async () => {
      await openAs(role, "/settings/tags");
      await waitFor(() => emptyTitle().length > 0, "a page title");
      expect(emptyTitle()).toEqual(["Tags"]);
    });
  }
});

describe("The not-found page says the page does not exist", () => {
  for (const role of ROLES) {
    for (const path of ["/calendar", "/no-such-page"]) {
      it(`${role} at ${path}`, async () => {
        await openAs(role, path);
        await waitFor(() => emptyTitle().length > 0, "a page title");
        expect(window.location.pathname).toBe(path);
        expect({
          title: emptyTitle(),
          saysDoesNotExist: mainText().includes(NOT_FOUND_LINE),
          saysComing: mainText().includes(COMING),
        }).toEqual({
          title: [NOT_FOUND_TITLE],
          saysDoesNotExist: true,
          saysComing: false,
        });
      });
    }
  }

  const kept: Array<[Role, string, string]> = [
    ["admin", "/settings/general", "General Settings"],
    ["admin", "/settings/notifications", "Notifications"],
    ["admin", "/settings/tags", "Tags"],
    ["user", "/settings/tags", "Tags"],
  ];
  for (const [role, path, title] of kept) {
    it(`${role} at ${path} keeps its own title and the coming-soon line`, async () => {
      await openAs(role, path);
      await waitFor(() => emptyTitle().length > 0, "a page title");
      expect(emptyTitle()).toEqual([title]);
      expect(mainText()).toContain(COMING);
    });
  }
});

/** Each admin page, its heading, and the admin-only calls it sends on opening. */
const ADMIN_PAGES: Array<{ path: string; heading: string; adminCalls: string[] }> = [
  { path: "/import", heading: "Manual Import", adminCalls: [] },
  {
    path: "/import/readarr",
    heading: "Readarr Import",
    adminCalls: ["/rootfolder", "/import/readarr/origin"],
  },
  {
    path: "/settings/mediamanagement",
    heading: "Media Management",
    adminCalls: [
      "/rootfolder",
      "/remotepathmapping",
      "/config/mediamanagement",
      "/config/naming",
      "/config/email",
    ],
  },
  {
    path: "/system/status",
    heading: "Status",
    adminCalls: ["/system/status", "/system/health", "/system/health-summary"],
  },
  {
    path: "/system/logs",
    heading: "Logs",
    adminCalls: ["/system/logs/tail", "/system/status"],
  },
  { path: "/unmapped", heading: "Unmapped Files", adminCalls: ["/rootfolder"] },
];

describe("Admin pages typed by a normal user", () => {
  for (const page of ADMIN_PAGES) {
    it(`normal user at ${page.path} lands on / and the page's admin-only calls are not sent`, async () => {
      const { calls } = await openAs("user", page.path);
      // Wait until either the guard has moved the address or the page has
      // mounted (its heading shows or its calls went out).
      await waitFor(
        () =>
          window.location.pathname === "/" ||
          pageHeading().includes(page.heading) ||
          page.adminCalls.some((p) => sentPaths(calls).includes(p)),
        "the guard or the page",
      );
      await settle(100);
      expect({
        path: window.location.pathname,
        adminCallsSent: page.adminCalls.filter((p) =>
          sentPaths(calls).includes(p),
        ),
      }).toEqual({ path: "/", adminCallsSent: [] });
    });

    it(`admin at ${page.path} sees ${page.heading}`, async () => {
      await openAs("admin", page.path);
      await waitFor(() => pageHeading().length > 0, "the page heading");
      expect(window.location.pathname).toBe(page.path);
      expect(pageHeading()).toEqual([page.heading]);
    });
  }
});

describe("Settings with no sub-page", () => {
  for (const path of ["/settings", "/settings/"]) {
    it(`normal user at ${path} ends at /settings/ui showing UI Settings`, async () => {
      const { calls } = await openAs("user", path);
      await waitFor(
        () =>
          window.location.pathname === "/settings/ui" ||
          pageHeading().length > 0 ||
          sentPaths(calls).includes("/rootfolder"),
        "a settings page",
      );
      await waitFor(
        () =>
          pageHeading().length > 0 ||
          (sentPaths(calls).includes("/rootfolder") &&
            mainText().length > 0),
        "the page to draw",
      );
      expect({
        path: window.location.pathname,
        heading: pageHeading(),
      }).toEqual({ path: "/settings/ui", heading: ["UI Settings"] });
    });

    it(`admin at ${path} sees Media Management`, async () => {
      await openAs("admin", path);
      await waitFor(() => pageHeading().length > 0, "the page heading");
      expect(pageHeading()).toEqual(["Media Management"]);
    });
  }
});
