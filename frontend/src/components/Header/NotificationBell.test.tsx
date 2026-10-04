import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
  recordAddedToasts,
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

const CHECK_FAILED = "Could not check for new notifications";
const STILL_TRYING = "Could not check for new notifications; still trying.";
const UNREAD_PATH = "/notification?page_size=200&unreadOnly=true";
const FULL_PATH = "/notification?page_size=200";

type CheckReply = StubReply | "network-error";

/** The bell's unread check and full list, each answered by its own script. */
function checkApi(unread: () => CheckReply, fullList: () => CheckReply) {
  return installApiStub((call) => {
    const reply =
      call.method === "GET" && call.path === UNREAD_PATH
        ? unread()
        : call.method === "GET" && call.path === FULL_PATH
          ? fullList()
          : serverFailure(`unexpected ${call.method} ${call.path}`);
    if (reply === "network-error") throw new TypeError("Failed to fetch");
    return reply;
  });
}

const unreadChecks = (calls: ApiCall[]) =>
  calls.filter((c) => c.method === "GET" && c.path === UNREAD_PATH).length;

const unreadItem = (id: number) => ({ ...notification, id });

/** Text of an element without the parts hidden from assistive technology. */
function spokenText(el: Element): string {
  if (el.getAttribute("aria-hidden") === "true") return "";
  return Array.from(el.childNodes)
    .map((node) =>
      node instanceof Element ? spokenText(node) : (node.textContent ?? ""),
    )
    .join("");
}

/** The bell's trigger: the button that opens its popover. */
function bellTrigger(container: HTMLElement): HTMLButtonElement {
  return container.querySelector("button")!;
}

/** What the bell itself shows: its unread count, and whether the warning mark is there. */
function bellFace(container: HTMLElement) {
  const trigger = bellTrigger(container);
  const named = (el: Element) => {
    const labelledBy = el.getAttribute("aria-labelledby");
    const byIds = labelledBy
      ? labelledBy
          .split(/\s+/)
          .map((id) => document.getElementById(id)?.textContent ?? "")
          .join(" ")
          .trim()
      : null;
    return (
      el.getAttribute("aria-label") === CHECK_FAILED ||
      el.getAttribute("title") === CHECK_FAILED ||
      byIds === CHECK_FAILED ||
      spokenText(el).trim() === CHECK_FAILED
    );
  };
  const mark = Array.from(trigger.querySelectorAll("*")).some(named);
  const count = (trigger.textContent ?? "").match(/\d+/)?.[0] ?? null;
  return {
    count,
    mark: mark && (trigger.textContent ?? "").includes("!"),
  };
}

describe("Notification bell: the unread check fails", () => {
  let recorder: ReturnType<typeof recordAddedToasts> | null = null;

  beforeEach(() => {
    vi.useFakeTimers();
    recorder = recordAddedToasts();
  });

  afterEach(() => {
    recorder?.stop();
    recorder = null;
    vi.useRealTimers();
  });

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

  function mount(unread: () => CheckReply, fullList: () => CheckReply = () => page([])) {
    const api = checkApi(unread, fullList);
    const mounted = mountBell();
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    return { api, container: mounted.container };
  }

  function expectNoPopUp() {
    expect(recorder!.added()).toEqual([]);
    expect(toastSummary()).toEqual([]);
  }

  it("a first check answering 500 shows the mark and no count; opening the bell shows the still-trying line", async () => {
    const view = mount(() => serverFailure("unread check failed"));
    await advance(0);
    expect(unreadChecks(view.api.calls)).toBe(1);
    expect(bellFace(view.container)).toEqual({ count: null, mark: true });

    await openBell(view.container);
    await advance(0);
    expect(fullLoads(view.api.calls)).toBe(1);
    expect(document.body.textContent).toContain(STILL_TRYING);
    expectNoPopUp();
  });

  it("a failed check after a success replaces the count with the mark until the next success", async () => {
    let unread: CheckReply = page([unreadItem(1), unreadItem(2)]);
    const view = mount(() => unread);
    await advance(0);
    expect(bellFace(view.container)).toEqual({ count: "2", mark: false });

    unread = serverFailure("unread check failed");
    await advance(30_000);
    expect(unreadChecks(view.api.calls)).toBe(2);
    expect(bellFace(view.container)).toEqual({ count: null, mark: true });

    await openBell(view.container);
    await advance(0);
    expect(document.body.textContent).toContain(STILL_TRYING);

    unread = "network-error";
    await advance(30_000);
    expect(unreadChecks(view.api.calls)).toBe(3);
    expect(bellFace(view.container)).toEqual({ count: null, mark: true });
    expect(document.body.textContent).toContain(STILL_TRYING);

    unread = page([unreadItem(1)]);
    await advance(30_000);
    expect(unreadChecks(view.api.calls)).toBe(4);
    expect(bellFace(view.container)).toEqual({ count: "1", mark: false });
    expect(document.body.textContent).toContain("Notifications");
    expect(document.body.textContent).not.toContain(STILL_TRYING);
    expectNoPopUp();
  });

  const malformed: Array<[string, CheckReply]> = [
    ["200 {}", { status: 200, body: {} }],
    ["404", { status: 404, body: { status: 404, error: "not_found", message: "not found" } }],
    ["200 with no body", { status: 200 }],
    ['200 { "items": {} }', { status: 200, body: { items: {} } }],
    ['200 { "items": "ab" }', { status: 200, body: { items: "ab" } }],
  ];

  for (const [shape, reply] of malformed) {
    it(`a first check answering ${shape} shows the mark and no count, with no pop-up`, async () => {
      const view = mount(() => reply);
      await advance(0);
      expect(unreadChecks(view.api.calls)).toBe(1);
      expect(bellFace(view.container)).toEqual({ count: null, mark: true });
      expectNoPopUp();
    });
  }

  it("with the check failed and the full list failing too, only the list's own error shows", async () => {
    let fullList: CheckReply = serverFailure("notifications unavailable");
    const view = mount(
      () => serverFailure("unread check failed"),
      () => fullList,
    );
    await advance(0);
    expect(bellFace(view.container)).toEqual({ count: null, mark: true });

    await openBell(view.container);
    await advance(0);
    expect(fullLoads(view.api.calls)).toBe(1);
    expect(document.body.textContent).toContain("Could not load notifications");
    expect(popoverButton("Retry")).toBeDefined();
    expect(document.body.textContent).not.toContain(STILL_TRYING);

    fullList = page([]);
    await click(popoverButton("Retry")!);
    await advance(0);
    expect(fullLoads(view.api.calls)).toBe(2);
    expect(document.body.textContent).not.toContain("Could not load notifications");
    expect(document.body.textContent).toContain(STILL_TRYING);
    expectNoPopUp();
  });

  // F3, code review r1: after a first failed check there is no cached count,
  // and the query library resets the errored query to pending when the next
  // check starts. The mark and the line follow the last completed check, so
  // they stay while that check is held, through a poll and a focus refetch.
  it("after a first failed check, the mark and the line stay while the next check is pending, until a check succeeds", async () => {
    let unread: () => StubReply | Promise<StubReply> = () =>
      serverFailure("unread check failed");
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === UNREAD_PATH) return unread();
      if (call.method === "GET" && call.path === FULL_PATH) return page([]);
      return serverFailure(`unexpected ${call.method} ${call.path}`);
    });
    const mounted = mountBell();
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    const view = { api, container: mounted.container };
    const held = () => {
      let answer: (reply: StubReply) => void = () => {};
      const reply = new Promise<StubReply>((resolve) => {
        answer = resolve;
      });
      unread = () => reply;
      return {
        answer: async (r: StubReply) => {
          await act(async () => answer(r));
          await advance(0);
        },
      };
    };
    const expectWarning = () => {
      expect(bellFace(view.container)).toEqual({ count: null, mark: true });
      expect(document.body.textContent).toContain(STILL_TRYING);
    };

    await advance(0);
    expect(unreadChecks(view.api.calls)).toBe(1);
    expect(bellFace(view.container)).toEqual({ count: null, mark: true });
    await openBell(view.container);
    await advance(0);
    expect(fullLoads(view.api.calls)).toBe(1);
    expect(document.body.textContent).not.toContain("Could not load notifications");
    expectWarning();

    // The next poll is held: the warning stays.
    const poll = held();
    await advance(30_000);
    expect(unreadChecks(view.api.calls)).toBe(2);
    expectWarning();

    // That poll fails too: the warning stays.
    await poll.answer(serverFailure("unread check failed again"));
    expectWarning();

    // A focus refetch is held: the warning stays.
    const focus = held();
    await act(async () => {
      window.dispatchEvent(new Event("visibilitychange"));
    });
    await advance(0);
    expect(unreadChecks(view.api.calls)).toBe(3);
    expectWarning();

    // It succeeds: the count shows and the warning is gone.
    await focus.answer(page([unreadItem(1)]));
    expect(bellFace(view.container)).toEqual({ count: "1", mark: false });
    expect(document.body.textContent).not.toContain(STILL_TRYING);
    expectNoPopUp();
  });

  // F3-R2, code review r2: a success and a later failure can finish with the
  // same clock reading. The clock is held still, so every check completes at
  // one time; the mark and the line still follow the order of the checks.
  it("with every check finishing at the same clock time, a failure after a success shows the mark and the line until the next success", async () => {
    let unread: CheckReply = page([unreadItem(1), unreadItem(2)]);
    const api = checkApi(() => unread, () => page([]));
    const frozen = vi.spyOn(Date, "now").mockReturnValue(Date.now());
    const mounted = mountBell();
    restore = () => {
      mounted.cleanup();
      api.restore();
      frozen.mockRestore();
    };
    const view = { api, container: mounted.container };

    await advance(0);
    expect(unreadChecks(view.api.calls)).toBe(1);
    expect(bellFace(view.container)).toEqual({ count: "2", mark: false });
    await openBell(view.container);
    await advance(0);
    expect(fullLoads(view.api.calls)).toBe(1);
    expect(document.body.textContent).not.toContain(STILL_TRYING);

    unread = serverFailure("unread check failed");
    await advance(30_000);
    expect(unreadChecks(view.api.calls)).toBe(2);
    expect(bellFace(view.container)).toEqual({ count: null, mark: true });
    expect(document.body.textContent).toContain(STILL_TRYING);

    unread = page([unreadItem(1)]);
    await advance(30_000);
    expect(unreadChecks(view.api.calls)).toBe(3);
    expect(bellFace(view.container)).toEqual({ count: "1", mark: false });
    expect(document.body.textContent).not.toContain(STILL_TRYING);
    expectNoPopUp();
  });
});
