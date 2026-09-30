import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

// Vite's build defines this constant; the test config does not, and the
// sidebar reads it when its module loads.
vi.hoisted(() => {
  (globalThis as Record<string, unknown>).__APP_VERSION__ = "0.0.0-test";
});

import { act } from "react";
import { createRoot } from "react-dom/client";
import { toast } from "sonner";
import { App } from "@/App";
import { useAuthStore } from "@/stores/auth";
import { installApiStub, type ApiCall, type StubReply } from "@/test-support/apiStub";

// The whole app is mounted for real; only the network boundary is stubbed.
// Endpoints the shell and the first screen read get a minimal valid reply;
// anything else answers 404 so no screen depends on it.
function stubApi(setupRequired: boolean) {
  return installApiStub((call: ApiCall): StubReply => {
    const path = call.path.split("?")[0];
    if (path === "/auth/me") {
      return {
        status: 200,
        body: {
          user: {
            id: 1,
            username: "admin",
            role: "admin",
            createdAt: "2026-01-01T00:00:00Z",
            updatedAt: "2026-01-01T00:00:00Z",
          },
          authType: "session",
        },
      };
    }
    if (path === "/setup/status") {
      return { status: 200, body: { setupRequired } };
    }
    if (path === "/work") {
      return {
        status: 200,
        body: { items: [], total: 0, page: 1, pageSize: 1000 },
      };
    }
    if (path === "/queue") {
      return {
        status: 200,
        body: { items: [], total: 0, page: 1, perPage: 50 },
      };
    }
    if (path === "/notification") {
      return {
        status: 200,
        body: { items: [], total: 0, page: 1, pageSize: 200 },
      };
    }
    return {
      status: 404,
      body: { error: "not_found", message: `unstubbed ${path}`, status: 404 },
    };
  });
}

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

function mountApp(path: string) {
  window.history.pushState({}, "", path);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(<App />);
  });
  return {
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function elementsWithText(text: string): Element[] {
  return Array.from(document.body.querySelectorAll("*")).filter(
    (el) =>
      el.textContent === text &&
      Array.from(el.children).every((child) => child.textContent !== text),
  );
}

describe("App notifications", () => {
  let restore: () => void = () => {};
  let cleanup: () => void = () => {};

  beforeEach(() => {
    localStorage.clear();
    localStorage.setItem("livrarr-tour-completed", "true");
    useAuthStore.setState({ status: "loading", user: null, token: null });
  });

  afterEach(async () => {
    toast.dismiss();
    await settle();
    cleanup();
    restore();
    localStorage.clear();
  });

  it("shows a notification once on a main page", async () => {
    localStorage.setItem("livrarr_token", "session-token");
    restore = stubApi(false).restore;
    cleanup = mountApp("/").cleanup;
    await settle(200);
    expect(useAuthStore.getState().status).toBe("authenticated");
    expect(window.location.pathname).toBe("/");

    act(() => {
      toast("probe");
    });
    await settle();

    expect(elementsWithText("probe")).toHaveLength(1);
    expect(document.querySelectorAll("[data-sonner-toaster]")).toHaveLength(1);
  });

  it("shows a notification once on the setup screen", async () => {
    restore = stubApi(true).restore;
    cleanup = mountApp("/setup").cleanup;
    await settle(200);
    expect(useAuthStore.getState().status).toBe("setup_required");
    expect(window.location.pathname).toBe("/setup");

    act(() => {
      toast("probe");
    });
    await settle();

    expect(elementsWithText("probe")).toHaveLength(1);
  });
});
