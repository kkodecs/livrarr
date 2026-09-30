import { afterEach, describe, expect, it, vi } from "vitest";
import SearchPage from "@/pages/search/SearchPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

const libraryWork = {
  id: 11,
  title: "Dune",
  authorName: "Frank Herbert",
  olKey: "OL1W",
  seriesName: null,
  seriesPosition: null,
  year: 1965,
  coverUrl: null,
};

const found = {
  title: "Dune Messiah",
  authorName: "Frank Herbert",
  olKey: "OL2W",
  authorOlKey: null,
  year: 1969,
  coverUrl: null,
  description: null,
  source: "openlibrary",
};

type LookupReply = StubReply | "network-error";

// A network rejection reaches the page as apiFetch's "Unable to reach Livrarr".
const NETWORK_MESSAGE = "Unable to reach Livrarr";

/** The page's reads; `lookup` answers each /work/lookup request in turn. */
function searchApi(
  lookup: (call: ApiCall) => LookupReply,
  library: unknown[] = [libraryWork],
) {
  return installApiStub((call) => {
    if (call.method === "GET" && call.path === "/config/metadata") {
      return { status: 200, body: { languages: ["en", "fr"] } };
    }
    if (call.method === "GET" && /^\/work\?/.test(call.path)) {
      return {
        status: 200,
        body: { items: library, total: library.length, page: 1, pageSize: 1000 },
      };
    }
    if (call.method === "GET" && call.path.startsWith("/work/lookup?")) {
      const reply = lookup(call);
      if (reply === "network-error") throw new TypeError("Failed to fetch");
      return reply;
    }
    return {
      status: 404,
      body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
    };
  });
}

const lookups = (calls: ApiCall[]) =>
  calls.filter((c) => c.path.startsWith("/work/lookup?")).map((c) => c.path);

function results(rawAvailable: boolean) {
  return {
    status: 200,
    body: { results: [found], filteredCount: 1, rawCount: 3, rawAvailable },
  };
}

const serverFailure: StubReply = {
  status: 500,
  body: { status: 500, error: "internal", message: "Search is unavailable right now" },
};

const buttonLabels = (scope: HTMLElement) =>
  Array.from(scope.querySelectorAll("button")).map(
    (b) => b.textContent?.trim() ?? "",
  );

let restore: () => void = () => {};
afterEach(() => restore());

describe("Book search: the search request fails", () => {
  it("a 500 shows the server's message and Retry, keeps library matches, and Retry repeats the same search", async () => {
    let lookupReply: LookupReply = results(true);
    const api = searchApi(() => lookupReply);
    const mounted = mountWith(newTestClient(), <SearchPage />, {
      path: "/search?q=Dune&lang=fr",
    });
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );

    lookupReply = serverFailure;
    await clickButton(mounted.container, "Raw 3");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    const failedSearch = lookups(api.calls)[1];
    expect(failedSearch).toBe("/work/lookup?term=Dune&lang=fr&raw=true");

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(
        "Search is unavailable right now",
      ),
    );
    const text = mounted.container.textContent ?? "";
    expect(text).not.toContain("No results");
    expect(text).toContain("In Your Library");
    expect(text).toContain("Dune");

    lookupReply = results(true);
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(3));
    expect(lookups(api.calls)[2]).toBe(failedSearch);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );
    expect(mounted.container.textContent).not.toContain(
      "Search is unavailable right now",
    );
  });

  it("a network rejection shows the error and Retry, not No results, and Retry repeats the same search", async () => {
    let lookupReply: LookupReply = "network-error";
    const api = searchApi(() => lookupReply);
    const mounted = mountWith(newTestClient(), <SearchPage />, {
      path: "/search?q=Dune&lang=fr",
    });
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
    const failedSearch = lookups(api.calls)[0];
    expect(failedSearch).toBe("/work/lookup?term=Dune&lang=fr");

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(NETWORK_MESSAGE),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    const text = mounted.container.textContent ?? "";
    expect(text).not.toContain("No results");
    expect(text).toContain("In Your Library");

    lookupReply = results(false);
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe(failedSearch);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );
    expect(mounted.container.textContent).not.toContain(NETWORK_MESSAGE);
    expect(buttonLabels(mounted.container)).not.toContain("Retry");
  });

  it("with no library match, a 500 shows the error and Retry, not No results", async () => {
    let lookupReply: LookupReply = serverFailure;
    const api = searchApi(() => lookupReply, []);
    const mounted = mountWith(newTestClient(), <SearchPage />, {
      path: "/search?q=Dune",
    });
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
    const failedSearch = lookups(api.calls)[0];

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(
        "Search is unavailable right now",
      ),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    expect(mounted.container.textContent).not.toContain("No results");
    expect(mounted.container.textContent).not.toContain("In Your Library");

    lookupReply = results(false);
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe(failedSearch);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Dune Messiah"),
    );
    expect(mounted.container.textContent).not.toContain(
      "Search is unavailable right now",
    );
  });
});
