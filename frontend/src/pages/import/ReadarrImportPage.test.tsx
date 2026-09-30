import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";
import ReadarrImportPage from "./ReadarrImportPage";

function baseReply(call: ApiCall) {
  if (call.method === "GET" && call.path === "/rootfolder") {
    return { status: 200, body: [] };
  }
  if (call.method === "GET" && call.path === "/import/readarr/history") {
    return { status: 200, body: { imports: [] } };
  }
  throw new Error(`unexpected call ${call.method} ${call.path}`);
}

function changeInput(input: HTMLInputElement, value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

afterEach(() => {
  useAuthStore.setState({ isAdmin: false });
});

describe("Readarr approved origins", () => {
  it("lists, adds, and removes origins through the real API wrapper", async () => {
    useAuthStore.setState({ isAdmin: true });
    const origins = [
      {
        id: 1,
        origin: "http://existing-readarr.internal:8787",
        createdAt: "2026-08-16T23:40:00Z",
      },
    ];
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/import/readarr/origin") {
        return { status: 200, body: origins };
      }
      if (call.method === "POST" && call.path === "/import/readarr/origin") {
        expect(call.body).toEqual({
          url: "http://new-readarr.internal:8787/path",
        });
        origins.push({
          id: 2,
          origin: "http://new-readarr.internal:8787",
          createdAt: "2026-08-16T23:45:00Z",
        });
        return { status: 200, body: origins[1] };
      }
      if (
        call.method === "DELETE" &&
        call.path === "/import/readarr/origin/2"
      ) {
        origins.splice(1, 1);
        return { status: 204 };
      }
      return baseReply(call);
    });
    const mounted = mountWith(newTestClient(), <ReadarrImportPage />);
    try {
      await act(async () => {
        await Promise.resolve();
      });
      await Promise.resolve();
      await act(async () => {
        await Promise.resolve();
      });
      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "http://existing-readarr.internal:8787",
        ),
      );

      const input = mounted.container.querySelector<HTMLInputElement>(
        'input[placeholder="readarr.internal:8787"]',
      );
      expect(input).not.toBeNull();
      changeInput(input!, "new-readarr.internal:8787/path");
      await clickButton(mounted.container, "Approve origin");

      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "http://new-readarr.internal:8787",
        ),
      );
      const remove = mounted.container.querySelector<HTMLButtonElement>(
        'button[aria-label="Remove http://new-readarr.internal:8787"]',
      );
      expect(remove).not.toBeNull();
      await act(async () => {
        remove!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
      await vi.waitFor(() =>
        expect(mounted.container.textContent).not.toContain(
          "http://new-readarr.internal:8787",
        ),
      );

      expect(api.calls.map((call) => `${call.method} ${call.path}`)).toEqual(
        expect.arrayContaining([
          "GET /import/readarr/origin",
          "POST /import/readarr/origin",
          "DELETE /import/readarr/origin/2",
        ]),
      );
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("shows the private-origin approval hint after a connect failure", async () => {
    useAuthStore.setState({ isAdmin: true });
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/import/readarr/origin") {
        return { status: 200, body: [] };
      }
      if (call.method === "POST" && call.path === "/import/readarr/connect") {
        return {
          status: 500,
          body: {
            status: 500,
            error: "internal",
            message: "unable to connect to the Readarr instance",
          },
        };
      }
      return baseReply(call);
    });
    const mounted = mountWith(newTestClient(), <ReadarrImportPage />);
    try {
      const inputs =
        mounted.container.querySelectorAll<HTMLInputElement>("input");
      changeInput(inputs[0]!, "private-readarr.internal:8787");
      changeInput(inputs[1]!, "secret-key");
      await clickButton(mounted.container, "Connect");

      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "Private or local Readarr addresses must be approved",
        ),
      );
      expect(api.calls).toContainEqual(
        expect.objectContaining({
          method: "POST",
          path: "/import/readarr/connect",
        }),
      );
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});

describe("Readarr import progress: a poll fails", () => {
  const LOST = "Lost contact with Livrarr. Progress may be out of date; still trying.";

  const progress = (filesProcessed: number) => ({
    running: true,
    importId: "imp-1",
    phase: "importing",
    authorsProcessed: 1,
    authorsTotal: 4,
    worksProcessed: 2,
    worksTotal: 8,
    filesProcessed,
    filesTotal: 12,
    filesSkipped: 0,
    errors: [],
  });

  function occurrences(scope: HTMLElement, text: string) {
    return (scope.textContent ?? "").split(text).length - 1;
  }

  async function nextPoll() {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });
  }

  it("shows one warning line per failure run, cleared by the next successful poll", async () => {
    useAuthStore.setState({ isAdmin: true });
    let pollReply: "network-error" | { status: number; body: unknown } = {
      status: 503,
      body: { status: 503, error: "unavailable", message: "busy" },
    };
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/import/readarr/origin") {
        return { status: 200, body: [] };
      }
      if (call.method === "GET" && call.path === "/rootfolder") {
        return {
          status: 200,
          body: [
            { id: 5, path: "/books", mediaType: "ebook", freeSpace: null, totalSpace: null },
          ],
        };
      }
      if (call.method === "POST" && call.path === "/import/readarr/connect") {
        return {
          status: 200,
          body: {
            rootFolders: [
              { id: 1, name: "Books", path: "/readarr/books", accessible: true, freeSpace: null, totalSpace: null },
            ],
          },
        };
      }
      if (call.method === "POST" && call.path === "/import/readarr/preview") {
        return {
          status: 200,
          body: {
            authorsToCreate: 4,
            authorsExisting: 0,
            worksToCreate: 8,
            worksExisting: 0,
            filesToImport: 12,
            filesToSkip: 0,
            skippedItems: [],
            importFiles: [],
          },
        };
      }
      if (call.method === "POST" && call.path === "/import/readarr/start") {
        return { status: 200, body: { importId: "imp-1" } };
      }
      if (call.method === "GET" && call.path === "/import/readarr/progress") {
        if (pollReply === "network-error") throw new TypeError("Failed to fetch");
        return pollReply;
      }
      return baseReply(call);
    });
    const mounted = mountWith(newTestClient(), <ReadarrImportPage />);
    const polls = () =>
      api.calls.filter((c) => c.path === "/import/readarr/progress").length;
    try {
      // Connect picks the first Livrarr root folder, so the list must be loaded.
      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "No private Readarr origins are approved.",
        ),
      );
      await vi.waitFor(() =>
        expect(api.calls.some((c) => c.path === "/rootfolder")).toBe(true),
      );
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 20));
      });
      const inputs = mounted.container.querySelectorAll<HTMLInputElement>("input");
      changeInput(inputs[0]!, "readarr.example:8787");
      changeInput(inputs[1]!, "secret-key");
      await clickButton(mounted.container, "Connect");
      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain("Preview Import"),
      );
      await clickButton(mounted.container, "Preview Import");
      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain("Confirm & Import 12 files"),
      );

      vi.useFakeTimers();
      await clickButton(mounted.container, "Confirm & Import 12 files");
      await act(async () => {
        await vi.advanceTimersByTimeAsync(0);
      });
      expect(
        api.calls.some((c) => c.path === "/import/readarr/start"),
      ).toBe(true);

      // Two failed polls before any progress exists: HTTP 503, then network.
      await nextPoll();
      expect(polls()).toBe(1);
      expect(occurrences(mounted.container, LOST)).toBe(1);
      pollReply = "network-error";
      await nextPoll();
      expect(polls()).toBe(2);
      expect(occurrences(mounted.container, LOST)).toBe(1);

      // A successful poll removes the line and shows the progress.
      pollReply = { status: 200, body: progress(3) };
      await nextPoll();
      expect(polls()).toBe(3);
      expect(occurrences(mounted.container, LOST)).toBe(0);
      expect(mounted.container.textContent).toContain("Import Progress");
      expect(mounted.container.textContent).toContain("3/12");

      // Two more failures while the old numbers are on screen: HTTP 500, then network.
      pollReply = { status: 500, body: { status: 500, error: "internal", message: "down" } };
      await nextPoll();
      expect(polls()).toBe(4);
      expect(occurrences(mounted.container, LOST)).toBe(1);
      expect(mounted.container.textContent).toContain("3/12");
      pollReply = "network-error";
      await nextPoll();
      expect(polls()).toBe(5);
      expect(occurrences(mounted.container, LOST)).toBe(1);
      expect(mounted.container.textContent).toContain("3/12");

      // The next successful poll removes it.
      pollReply = { status: 200, body: progress(4) };
      await nextPoll();
      expect(polls()).toBe(6);
      expect(occurrences(mounted.container, LOST)).toBe(0);
      expect(mounted.container.textContent).toContain("4/12");
    } finally {
      vi.useRealTimers();
      mounted.cleanup();
      api.restore();
    }
  });
});
