import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import AuthorSearchPage from "@/pages/search/AuthorSearchPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";

type LookupReply = StubReply | "network-error";

// A network rejection reaches the page as apiFetch's "Unable to reach Livrarr".
const NETWORK_MESSAGE = "Unable to reach Livrarr";

function authorApi(lookup: () => LookupReply) {
  return installApiStub((call) => {
    if (call.method === "GET" && call.path.startsWith("/author/lookup?")) {
      const reply = lookup();
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
  calls.filter((c) => c.path.startsWith("/author/lookup?")).map((c) => c.path);

const buttonLabels = (scope: HTMLElement) =>
  Array.from(scope.querySelectorAll("button")).map(
    (b) => b.textContent?.trim() ?? "",
  );

async function search(scope: HTMLElement, term: string) {
  const input = scope.querySelector<HTMLInputElement>(
    'input[placeholder="Search by author name..."]',
  )!;
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")
      ?.set?.call(input, term);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await act(async () => {
    input.form!.requestSubmit();
  });
}

const found: StubReply = {
  status: 200,
  body: [{ olKey: "OL1A", name: "Ursula K. Le Guin", sortName: "Le Guin, Ursula K." }],
};

let restore: () => void = () => {};
afterEach(() => restore());

describe("Author search: the search request fails", () => {
  it("a 500 shows the server's message and Retry, and Retry repeats the same search", async () => {
    let reply: LookupReply = {
      status: 500,
      body: { status: 500, error: "internal", message: "Author search failed" },
    };
    const api = authorApi(() => reply);
    const mounted = mountWith(newTestClient(), <AuthorSearchPage />);
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await search(mounted.container, "Le Guin");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(
        "Author search failed",
      ),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    expect(mounted.container.textContent).not.toContain("No results");

    reply = found;
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe(lookups(api.calls)[0]);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Ursula K. Le Guin"),
    );
    expect(mounted.container.textContent).not.toContain(
      "Author search failed",
    );
  });

  it("a 502 provider error shows the server's message and Retry, and Retry repeats the same search", async () => {
    let reply: LookupReply = {
      status: 502,
      body: { status: 502, error: "bad_gateway", message: "Author provider unreachable" },
    };
    const api = authorApi(() => reply);
    const mounted = mountWith(newTestClient(), <AuthorSearchPage />);
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await search(mounted.container, "Le Guin");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(
        "Author provider unreachable",
      ),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    expect(mounted.container.textContent).not.toContain("No results");

    reply = found;
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe(lookups(api.calls)[0]);
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Ursula K. Le Guin"),
    );
    expect(mounted.container.textContent).not.toContain(
      "Author provider unreachable",
    );
  });

  it("a network rejection shows the error and Retry, not No results, and Retry repeats the same search", async () => {
    let reply: LookupReply = "network-error";
    const api = authorApi(() => reply);
    const mounted = mountWith(newTestClient(), <AuthorSearchPage />);
    restore = () => {
      mounted.cleanup();
      api.restore();
    };
    await search(mounted.container, "Le Guin");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(1));
    expect(lookups(api.calls)[0]).toBe("/author/lookup?term=Le%20Guin");

    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain(NETWORK_MESSAGE),
    );
    expect(buttonLabels(mounted.container)).toContain("Retry");
    expect(mounted.container.textContent).not.toContain("No results");

    reply = found;
    await clickButton(mounted.container, "Retry");
    await vi.waitFor(() => expect(lookups(api.calls)).toHaveLength(2));
    expect(lookups(api.calls)[1]).toBe("/author/lookup?term=Le%20Guin");
    await vi.waitFor(() =>
      expect(mounted.container.textContent).toContain("Ursula K. Le Guin"),
    );
    expect(mounted.container.textContent).not.toContain(NETWORK_MESSAGE);
    expect(buttonLabels(mounted.container)).not.toContain("Retry");
  });
});
