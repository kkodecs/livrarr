import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NotificationBell } from "@/components/Header/NotificationBell";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import {
  AppToaster,
  clearToasts,
  toastSummary,
} from "@/test-support/toasts";

const notification = {
  id: 3,
  notificationType: "metadataUpdated",
  refKey: null,
  message: "Metadata refreshed for Dune",
  data: {},
  read: false,
  createdAt: "2026-09-29T12:00:00Z",
};

const page = (items: unknown[]) => ({
  status: 200,
  body: { items, total: items.length, page: 1, pageSize: 200 },
});

const serverFailure = (message: string): StubReply => ({
  status: 500,
  body: { status: 500, error: "internal", message },
});

/** The unread badge list always loads; the full list and writes are scripted. */
function bellApi(
  fullList: () => StubReply | "network-error",
  write: (call: ApiCall) => StubReply,
) {
  return installApiStub((call) => {
    if (call.method === "GET" && call.path === "/notification?page_size=200&unreadOnly=true") {
      return page([]);
    }
    if (call.method === "GET" && call.path === "/notification?page_size=200") {
      const reply = fullList();
      if (reply === "network-error") throw new TypeError("Failed to fetch");
      return reply;
    }
    return write(call);
  });
}

const fullLoads = (calls: ApiCall[]) =>
  calls.filter(
    (c) => c.method === "GET" && c.path === "/notification?page_size=200",
  ).length;

function mountBell() {
  return mountWith(
    newTestClient(),
    <>
      <NotificationBell />
      <AppToaster />
    </>,
  );
}

async function openBell(container: HTMLElement) {
  const trigger = container.querySelector("button")!;
  await act(async () => {
    trigger.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

async function click(button: Element) {
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

async function settle(ms = 50) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

function popoverButton(label: string): HTMLButtonElement | undefined {
  return Array.from(document.body.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === label,
  );
}

let restore: () => void = () => {};
afterEach(async () => {
  restore();
  await clearToasts();
});

describe("Notification bell: loading the list fails", () => {
  it("shows the load error with Retry instead of No notifications, and Retry shows the list", async () => {
    let listReply: StubReply | "network-error" = serverFailure("notifications unavailable");
    const api = bellApi(
      () => listReply,
      (call) => serverFailure(`unexpected ${call.method} ${call.path}`),
    );
    const mounted = mountBell();
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await openBell(mounted.container);
    await vi.waitFor(() => expect(fullLoads(api.calls)).toBe(1));
    await settle();

    await vi.waitFor(() =>
      expect(document.body.textContent).toContain(
        "Could not load notifications",
      ),
    );
    expect(document.body.textContent).not.toContain("No notifications");

    listReply = page([notification]);
    const retry = popoverButton("Retry");
    expect(retry).toBeDefined();
    await click(retry!);
    await vi.waitFor(() => expect(fullLoads(api.calls)).toBe(2));
    await vi.waitFor(() =>
      expect(document.body.textContent).toContain(
        "Metadata refreshed for Dune",
      ),
    );
    expect(document.body.textContent).not.toContain(
      "Could not load notifications",
    );
  });
});

describe("Notification bell: an action is rejected", () => {
  async function openWithList(write: (call: ApiCall) => StubReply) {
    const api = bellApi(() => page([notification]), write);
    const mounted = mountBell();
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await openBell(mounted.container);
    await vi.waitFor(() =>
      expect(document.body.textContent).toContain(
        "Metadata refreshed for Dune",
      ),
    );
    expect(toastSummary()).toEqual([]);
    return api;
  }

  const sent = (calls: ApiCall[], method: string, path: string) =>
    calls.filter((c) => c.method === method && c.path === path).length;

  it("mark as read shows one error toast", async () => {
    const api = await openWithList(() => serverFailure("mark read failed"));
    await click(
      document.body.querySelector('button[aria-label="Mark as read"]')!,
    );
    await vi.waitFor(() =>
      expect(sent(api.calls, "PUT", "/notification/3")).toBe(1),
    );
    await settle();
    expect(toastSummary()).toEqual([
      { type: "error", text: expect.stringContaining("Could not mark the notification as read") },
    ]);
  });

  it("dismiss shows one error toast", async () => {
    const api = await openWithList(() => serverFailure("dismiss failed"));
    await click(
      document.body.querySelector(
        'button[aria-label="Dismiss notification"]',
      )!,
    );
    await vi.waitFor(() =>
      expect(sent(api.calls, "DELETE", "/notification/3")).toBe(1),
    );
    await settle();
    expect(toastSummary()).toEqual([
      { type: "error", text: expect.stringContaining("Could not dismiss the notification") },
    ]);
  });

  it("dismiss all shows one error toast", async () => {
    const api = await openWithList(() => serverFailure("dismiss all failed"));
    await click(popoverButton("Dismiss All")!);
    await vi.waitFor(() =>
      expect(sent(api.calls, "DELETE", "/notification")).toBe(1),
    );
    await settle();
    expect(toastSummary()).toEqual([
      { type: "error", text: expect.stringContaining("Could not dismiss notifications") },
    ]);
  });
});
