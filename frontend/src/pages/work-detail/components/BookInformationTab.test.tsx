import { act } from "react";
import { describe, expect, it, vi } from "vitest";
import WorkDetailPage from "../WorkDetailPage";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";
import type { WorkCoverUiState, WorkDetailResponse } from "@/types/api";

// Superseded 2026-09-23 by PO decision: the "Other books by this author"
// panel and the identitySiblings response field are removed. These strings
// pin their absence.
const REMOVED_SIBLING_HEADING = "Other books by this author";
const REMOVED_SIBLING_COPY =
  "Confirming this book's identity affects only this book. Other books by this author stay exactly as they are.";
const REMOVED_SIBLING_EMPTY_STATE = "No related library books to show.";

function makeWork(
  coverUiState: WorkCoverUiState,
  over: Partial<WorkDetailResponse> = {},
): WorkDetailResponse {
  return {
    id: 7,
    title: "The Current Book",
    authorName: "Case Writer",
    identityStatus: "confirmed",
    enrichmentStatus: "enriched",
    enriching: false,
    parkedByConflicts: false,
    olKey: "OL7W",
    hcKey: null,
    grKey: null,
    isbn13: null,
    asin: null,
    libraryItems: [],
    coverManual: false,
    coverSource: null,
    coverMtime: null,
    audiobookCoverUrl: null,
    audiobookCoverSource: null,
    audiobookCoverMtime: null,
    coverUiState,
    ...over,
  } as unknown as WorkDetailResponse;
}

function installWorkRoute(work: WorkDetailResponse) {
  return installApiStub((call: ApiCall) => {
    if (call.method === "GET" && /^\/work\/\d+$/.test(call.path)) {
      return { status: 200, body: work };
    }
    if (call.method === "GET" && /\/work\/\d+\/pending-anchors$/.test(call.path)) {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path.startsWith("/queue")) {
      return { status: 200, body: { items: [], total: 0, page: 1, pageSize: 50 } };
    }
    throw new Error(`unexpected call ${call.method} ${call.path}`);
  });
}

function mountWorkRoute() {
  return mountWith(newTestClient(), <WorkDetailPage />, {
    path: "/work/7?tab=metadata",
    route: "/work/:id",
  });
}

describe("Book information identity-layer presentation", () => {
  it("renders no sibling panel and issues zero mutations", async () => {
    const work = makeWork({
      formatNeeded: null,
      ebook: { state: "NowhereToLook" },
      audiobook: { state: "NowhereToLook" },
    });
    const api = installWorkRoute(work);
    const mounted = mountWorkRoute();
    try {
      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain("Merging is currently unavailable."),
      );
      expect(Array.from(mounted.container.querySelectorAll("button"))
        .some((button) => /merge/i.test(button.textContent ?? ""))).toBe(false);
      expect(
        mounted.container.querySelector('[data-testid="identity-sibling-panel"]'),
      ).toBeNull();
      expect(mounted.container.querySelectorAll("[data-sibling-affordance]")).toHaveLength(0);
      expect(mounted.container.textContent).not.toContain(REMOVED_SIBLING_HEADING);
      expect(mounted.container.textContent).not.toContain(REMOVED_SIBLING_COPY);
      expect(mounted.container.textContent).not.toContain(REMOVED_SIBLING_EMPTY_STATE);

      expect(api.calls.filter((call) => call.method !== "GET")).toEqual([]);

      const edit = Array.from(mounted.container.querySelectorAll("button"))
        .find((button) => button.textContent?.trim() === "Edit");
      expect(edit).toBeDefined();
      await act(async () => edit!.click());
      expect(document.querySelector<HTMLInputElement>('input[name="title"]')?.readOnly).toBe(false);
      expect(document.querySelector<HTMLInputElement>('input[name="authorName"]')?.readOnly).toBe(false);
      expect(document.querySelector<HTMLInputElement>('input[name="seriesName"]')?.readOnly).toBe(false);
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("renders one shared format panel, every slot state, and source-only labels", async () => {
    const first = makeWork({
      formatNeeded: {
        candidates: [
          { id: "file-cover", source: "Your file" },
          { id: "chosen-cover", source: "Yours" },
        ],
      },
      ebook: { state: "Selected", source: "Provider" },
      audiobook: { state: "Searching" },
    });
    const firstApi = installWorkRoute(first);
    const firstMount = mountWorkRoute();
    try {
      await vi.waitFor(() =>
        expect(firstMount.container.textContent).toContain("Cover found — format needed"),
      );
      expect(firstMount.container.querySelectorAll('[data-cover-panel="FormatNeeded"]')).toHaveLength(1);
      expect(firstMount.container.querySelector('[data-cover-slot="ebook"]')?.textContent).toContain(
        "Provider",
      );
      expect(firstMount.container.textContent).toContain("Your file");
      expect(firstMount.container.textContent).toContain("Yours");
      expect(
        firstMount.container.querySelector('[data-cover-slot="audiobook"]')?.getAttribute(
          "data-cover-state",
        ),
      ).toBe("Searching");
      expect(firstMount.container.textContent).not.toMatch(/validated|unvalidated|trust:/i);
      expect(firstApi.calls.filter((call) => call.method !== "GET")).toEqual([]);
    } finally {
      firstMount.cleanup();
      firstApi.restore();
    }

    const second = makeWork({
      formatNeeded: null,
      ebook: { state: "NoCoverFound" },
      audiobook: { state: "NowhereToLook" },
    });
    const secondApi = installWorkRoute(second);
    const secondMount = mountWorkRoute();
    try {
      await vi.waitFor(() => expect(secondMount.container.textContent).toContain("No cover found"));
      expect(
        secondMount.container.querySelector('[data-cover-slot="ebook"]')?.getAttribute(
          "data-cover-state",
        ),
      ).toBe("NoCoverFound");
      expect(
        secondMount.container.querySelector('[data-cover-slot="audiobook"]')?.getAttribute(
          "data-cover-state",
        ),
      ).toBe("NowhereToLook");
      expect(secondMount.container.querySelectorAll('[data-cover-panel="FormatNeeded"]')).toHaveLength(0);
      expect(secondApi.calls.filter((call) => call.method !== "GET")).toEqual([]);
    } finally {
      secondMount.cleanup();
      secondApi.restore();
    }
  });
});
