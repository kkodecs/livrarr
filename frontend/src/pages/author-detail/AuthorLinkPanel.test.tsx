import { describe, it, expect, afterEach, vi } from "vitest";
import AuthorDetailPage from "./AuthorDetailPage";
import {
  clickButton,
  clickTitled,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import {
  EMPTY_BIBLIOGRAPHY,
  makeAuthor,
  makeRoute,
  makeSweep,
} from "@/test-support/authorFixtures";
import type { AuthorResponse } from "@/types/api";

let restore: (() => void) | null = null;

afterEach(() => {
  restore?.();
  restore = null;
});

function stub(handler: (call: ApiCall) => StubReply | Promise<StubReply>) {
  const s = installApiStub(handler);
  restore = s.restore;
  return s;
}

function mountAuthor(client = newTestClient()) {
  return {
    client,
    ...mountWith(client, <AuthorDetailPage />, {
      path: "/author/7",
      route: "/author/:id",
    }),
  };
}

describe("removeAuthorRoute — removing a link from the routes panel", () => {
  // IR tdd_directive 3. Removing a route leaves a tombstone no automatic
  // process may undo, and an author who loses their Open Library route loses
  // the ability to be monitored with it.
  it("tombstones the route, keeps its provenance visible, and drops monitorability", async () => {
    const linked = makeAuthor({
      linkState: "linked",
      monitorable: true,
      olKey: "OL111A",
      routes: [
        makeRoute({
          id: 9,
          provider: "open_library",
          value: "OL111A",
          provenance: "legacy_unguarded",
        }),
      ],
    });
    const afterRemoval: AuthorResponse = makeAuthor({
      linkState: "unlinked",
      monitorable: false,
      olKey: null,
      routes: [
        makeRoute({
          id: 9,
          provider: "open_library",
          value: "OL111A",
          provenance: "legacy_unguarded",
          state: "removed",
          removedAt: "2026-07-30T09:00:00.000Z",
        }),
      ],
    });

    let removed = false;
    const { calls } = stub((call) => {
      if (call.path === "/author/7") {
        return {
          status: 200,
          body: { author: removed ? afterRemoval : linked, works: [] },
        };
      }
      if (call.path === "/author/7/route/9" && call.method === "DELETE") {
        removed = true;
        return { status: 204 };
      }
      if (call.path === "/author/7/bibliography") {
        return { status: 200, body: EMPTY_BIBLIOGRAPHY };
      }
      if (call.path === "/author-link-sweep/progress") {
        return { status: 200, body: makeSweep({ total: 1, completed: 1 }) };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const { container, cleanup } = mountAuthor();
    try {
      await vi.waitFor(
        () => expect(container.textContent).toContain("Can be monitored"),
        { timeout: 5000 },
      );
      expect(container.textContent).toContain("From before linking existed");
      expect(container.textContent).toContain("In use");

      await clickTitled(container, "Remove this link");
      // The confirmation dialog is portalled outside the page container.
      await vi.waitFor(
        () => expect(document.body.textContent).toContain("Remove this link?"),
        { timeout: 5000 },
      );
      // Naming the consequence before it happens.
      expect(document.body.textContent).toContain(
        "This author will stop being monitorable.",
      );
      await clickButton(document.body, "Remove");

      const deleteIndex = calls.findIndex((c) => c.path === "/author/7/route/9");
      expect(deleteIndex).toBeGreaterThanOrEqual(0);
      expect(calls[deleteIndex]?.method).toBe("DELETE");

      // The route stays on screen as a tombstone, with the provenance it had.
      await vi.waitFor(
        () => expect(container.textContent).toContain("Cannot be monitored"),
        { timeout: 5000 },
      );
      expect(container.textContent).toContain("Removed");
      expect(container.textContent).toContain("OL111A");
      expect(container.textContent).toContain("From before linking existed");
      expect(container.textContent).not.toContain("In use");
      // Nothing local: the author was re-read from the server, and so were the
      // sweep counters, after the removal.
      const paths = calls.slice(deleteIndex + 1).map((c) => c.path);
      expect(paths).toContain("/author/7");
      expect(paths).toContain("/author-link-sweep/progress");
    } finally {
      cleanup();
    }
  });
});

describe("reResolveAuthor — asking for another look", () => {
  // IR tdd_directive 4. The provider seam stays blocked for the whole test:
  // nothing on this path may wait for a provider. The 202 is the whole answer,
  // and the progress that follows comes from persisted state, not from memory
  // of having clicked.
  it("shows queued immediately from the 202 and then reads persisted progress", async () => {
    const unlinked = makeAuthor({ linkState: "unlinked", monitorable: false });
    let queued = false;

    const { calls } = stub((call) => {
      if (call.path === "/author/7") {
        // The author never becomes linked: the provider seam is blocked for
        // the whole test, so any UI that waited for a link would hang here.
        return { status: 200, body: { author: unlinked, works: [] } };
      }
      if (call.path === "/author/7/resolve" && call.method === "POST") {
        queued = true;
        return { status: 202 };
      }
      if (call.path === "/author/7/bibliography") {
        return { status: 200, body: EMPTY_BIBLIOGRAPHY };
      }
      if (call.path === "/author-link-sweep/progress") {
        return {
          status: 200,
          body: queued
            ? makeSweep({ total: 4, completed: 1, queued: 2, running: 1 })
            : makeSweep({ total: 4, completed: 4 }),
        };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const { container, cleanup } = mountAuthor();
    try {
      await vi.waitFor(
        () => expect(container.textContent).toContain("Look again"),
        { timeout: 5000 },
      );

      await clickTitled(container, "Queue this author for another look");

      const resolveIndex = calls.findIndex((c) => c.path === "/author/7/resolve");
      expect(resolveIndex).toBeGreaterThanOrEqual(0);
      expect(calls[resolveIndex]?.method).toBe("POST");

      // The 202 alone puts the UI in its queued state — the button is not
      // left spinning waiting on anything.
      await vi.waitFor(
        () => expect(container.textContent).toContain("Linking sweep:"),
        { timeout: 5000 },
      );
      expect(container.textContent).not.toContain("Queueing…");
      // The author is still unlinked. Nothing pretended otherwise.
      expect(container.textContent).toContain("Cannot be monitored");

      // The numbers on screen came from the persisted progress read that
      // followed the 202, not from anything the click remembered.
      const paths = calls.slice(resolveIndex + 1).map((c) => c.path);
      expect(paths).toContain("/author-link-sweep/progress");
      expect(container.textContent).toContain(
        "Linking sweep: 1 of 4 authors done, 2 waiting, 1 in progress.",
      );
    } finally {
      cleanup();
    }
  });
});
