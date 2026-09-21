import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import SearchPage from "@/pages/search/SearchPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
} from "@/test-support/apiStub";
import expectedFactsJson from "../../../../tests/behavioral/add_metadata_preservation/fixtures/expected-facts.json?raw";

const records = JSON.parse(expectedFactsJson);

describe("SearchPage Add facts", () => {
  // SAVE-W1: REQ-001/008; AC-002
  it("Add echoes the chosen complete Google selection without reconstruction or refetch", async () => {
    // Start with the complete captured Google record. These explicit synthetic
    // additions distinguish deep transport from rebuilding the visible card,
    // retaining only the first contributor, or slicing a long identifier list.
    const facts = structuredClone(records.google_books);
    facts.subtitle = "Transport subtitle sentinel";
    facts.seriesName = "Dune";
    facts.seriesPosition = 1.5;
    facts.contributors.push(
      { name: "Contributor One" },
      {
        name: "Contributor Two",
        providerAuthorId: "transport-author-reference",
      },
    );
    // Existing long-list sentinel: real ISBN spellings supplied synthetically
    // as Google facts. This is transport coverage, not a Google capture claim.
    facts.references.push(
      ...records.hardcover.references.filter((reference: { kind: string }) =>
        reference.kind.startsWith("isbn_"),
      ),
    );
    const expected = structuredClone(facts);
    const chosen = {
      title: "Dune",
      authorName: "Frank Herbert",
      source: "google_books",
      language: "en",
      year: null,
      olKey: null,
      authorOlKey: null,
      hcKey: null,
      grKey: null,
      asin: null,
      candidateId: null,
      detailUrl: null,
      isbn13: "9780441013593",
      coverUrl: facts.coverUrl,
      description: "Only a card summary",
      seriesName: null,
      seriesPosition: null,
      rating: "4.50",
      facts,
    };
    const other = {
      ...chosen,
      title: "Dune Messiah",
      isbn13: "9780593098233",
      facts: { ...facts, description: "Other result facts", references: [] },
    };
    const unexpected: string[] = [];
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/config/metadata") {
        return { status: 200, body: { languages: ["fr", "en"] } };
      }
      if (call.method === "GET" && /^\/work(?:\?|$)/.test(call.path)) {
        return { status: 200, body: { items: [], total: 0 } };
      }
      if (call.method === "GET" && call.path.startsWith("/work/lookup?")) {
        return {
          status: 200,
          body: {
            results: [other, chosen],
            filteredCount: 2,
            rawCount: 2,
            rawAvailable: false,
          },
        };
      }
      if (call.method === "POST" && call.path === "/work") {
        // The outgoing request is the observable under test. A controlled
        // failure keeps the real page mounted so navigation adds no requests.
        return { status: 503, body: { message: "Controlled Add response" } };
      }
      unexpected.push(`${call.method} ${call.path}`);
      return { status: 500, body: { message: "Unexpected test request" } };
    });
    vi.useFakeTimers();
    const client = newTestClient();
    const mounted = mountWith(client, <SearchPage />, {
      path: "/search?q=Dune&lang=fr",
    });
    try {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
      const buttons = [...mounted.container.querySelectorAll("button")].filter(
        (button) => button.textContent?.trim() === "Add",
      );
      expect(buttons).toHaveLength(2);
      const chosenButton = buttons[1]!;
      await clickButton(chosenButton.parentElement!, "Add");
      const posts = api.calls.filter(
        (call) => call.method === "POST" && call.path === "/work",
      );
      expect(posts).toHaveLength(1);
      const body = posts[0]!.body as Record<string, unknown>;
      expect(body).toMatchObject({
        title: "Dune",
        authorName: "Frank Herbert",
        language: "en",
        year: null,
        coverUrl: chosen.coverUrl,
        coverManual: false,
        isbn13: "9780441013593",
      });
      expect(
        api.calls.filter((call) => call.path.startsWith("/work/lookup?")),
      ).toHaveLength(1);
      expect(unexpected).toEqual([]);
      expect(facts).toEqual(expected);
      expect(
        body.facts,
        "real Add must include the complete selection",
      ).toBeDefined();
      expect(body.facts).toEqual(expected);
    } finally {
      mounted.cleanup();
      client.clear();
      api.restore();
      vi.useRealTimers();
    }
  });
  // SAVE-W2: REQ-001/006/008; AC-002
  it("Add carries the captured Goodreads truncation, series and rating without refetch", async () => {
    const facts = structuredClone(records.goodreads);
    const expected = structuredClone(facts);
    const chosen = {
      title: facts.decoratedTitle,
      authorName: "Frank Herbert",
      source: "goodreads",
      language: null,
      year: null,
      olKey: null,
      authorOlKey: null,
      hcKey: null,
      grKey: "44767458",
      asin: null,
      candidateId: null,
      detailUrl: "https://www.goodreads.com/book/show/44767458",
      isbn13: null,
      coverUrl: facts.coverUrl,
      description: "Only a card summary",
      seriesName: "Dune",
      seriesPosition: 1,
      rating: "4.29",
      facts,
    };
    const unexpected: string[] = [];
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/config/metadata") {
        return { status: 200, body: { languages: ["fr", "en"] } };
      }
      if (call.method === "GET" && /^\/work(?:\?|$)/.test(call.path)) {
        return { status: 200, body: { items: [], total: 0 } };
      }
      if (call.method === "GET" && call.path.startsWith("/work/lookup?")) {
        return {
          status: 200,
          body: {
            results: [chosen],
            filteredCount: 1,
            rawCount: 1,
            rawAvailable: false,
          },
        };
      }
      if (call.method === "POST" && call.path === "/work") {
        return { status: 503, body: { message: "Controlled Add response" } };
      }
      unexpected.push(`${call.method} ${call.path}`);
      return { status: 500, body: { message: "Unexpected test request" } };
    });
    vi.useFakeTimers();
    const client = newTestClient();
    const mounted = mountWith(client, <SearchPage />, {
      path: "/search?q=Dune&lang=fr",
    });
    try {
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1);
      });
      const buttons = [...mounted.container.querySelectorAll("button")].filter(
        (button) => button.textContent?.trim() === "Add",
      );
      expect(buttons).toHaveLength(1);
      await clickButton(buttons[0]!.parentElement!, "Add");
      const posts = api.calls.filter(
        (call) => call.method === "POST" && call.path === "/work",
      );
      expect(posts).toHaveLength(1);
      const body = posts[0]!.body as Record<string, unknown>;
      expect(body).toMatchObject({
        title: "Dune (Dune, #1)",
        authorName: "Frank Herbert",
        grKey: "44767458",
        language: null,
        year: null,
        coverUrl: chosen.coverUrl,
        coverManual: false,
      });
      expect(
        api.calls.filter((call) => call.path.startsWith("/work/lookup?")),
      ).toHaveLength(1);
      expect(unexpected).toEqual([]);
      expect(facts).toEqual(expected);
      expect(expected.descriptionTruncated).toBe(true);
      expect(expected.seriesName).toBe("Dune");
      expect(expected.seriesPosition).toBe(1);
      expect(expected.rating).toBe(4.29);
      expect(expected.ratingCount).toBe(1709797);
      expect(expected.provider).toBe("goodreads");
      expect(expected).not.toHaveProperty("originalYear");
      expect(expected).not.toHaveProperty("originalPublishDate");
      expect(expected).not.toHaveProperty("editionPublishDate");
      expect(expected).not.toHaveProperty("language");
      expect(
        body.facts,
        "real Add must carry the complete captured Goodreads selection",
      ).toBeDefined();
      expect(body.facts).toEqual(expected);
    } finally {
      mounted.cleanup();
      client.clear();
      api.restore();
      vi.useRealTimers();
    }
  });
});
