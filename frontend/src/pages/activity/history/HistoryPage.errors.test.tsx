import { afterEach, describe, expect, it, vi } from "vitest";
import HistoryPage from "./HistoryPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

const historyRow = {
  id: 1,
  workId: 11,
  eventType: "imported",
  data: { title: "Imported Dune" },
  date: "2026-09-29T12:00:00Z",
};

const libraryWork = { id: 11, title: "Dune", authorName: "Frank Herbert" };

const isHistoryRead = (c: ApiCall) =>
  c.method === "GET" && c.path.startsWith("/history?");
const isLibraryRead = (c: ApiCall) =>
  c.method === "GET" && /^\/work\?/.test(c.path);

const buttonLabels = (scope: HTMLElement) =>
  Array.from(scope.querySelectorAll("button")).map(
    (b) => b.textContent?.trim() ?? "",
  );

let restore: () => void = () => {};
afterEach(() => restore());

describe("History: the whole-library read fails", () => {
  it("shows the library error with Retry and no spinner, and Retry sends both reads and renders the page", async () => {
    let libraryReply: StubReply = {
      status: 500,
      body: { status: 500, error: "internal", message: "library read failed" },
    };
    const api = installApiStub((call) => {
      if (isHistoryRead(call)) {
        return {
          status: 200,
          body: { items: [historyRow], total: 1, page: 1, pageSize: 200 },
        };
      }
      if (isLibraryRead(call)) return libraryReply;
      return {
        status: 404,
        body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
      };
    });
    const mounted = mountWith(newTestClient(), <HistoryPage />);
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() =>
      expect(api.calls.filter(isLibraryRead)).toHaveLength(1),
    );

    await vi.waitFor(() =>
      expect(mounted.container.querySelector(".animate-spin")).toBeNull(),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    expect(mounted.container.textContent).toContain("library read failed");

    const historyBefore = api.calls.filter(isHistoryRead).length;
    libraryReply = {
      status: 200,
      body: { items: [libraryWork], total: 1, page: 1, pageSize: 1000 },
    };
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() =>
      expect(api.calls.filter(isLibraryRead)).toHaveLength(2),
    );
    await vi.waitFor(() =>
      expect(api.calls.filter(isHistoryRead).length).toBeGreaterThan(
        historyBefore,
      ),
    );
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Imported Dune"),
    );
    expect(mounted.container.textContent).not.toContain("library read failed");
  });
});
