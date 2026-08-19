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

const FROZEN_SIBLING_COPY =
  "Confirming this book's identity affects only this book. Other books by this author stay exactly as they are.";

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
    identitySiblings: [
      {
        workId: 8,
        title: "The First Sibling",
        authorName: "Case Writer",
        edition: "Ebook",
        route: "Open Library",
      },
      {
        workId: 9,
        title: "The Second Sibling",
        authorName: "Case Writer",
        edition: "Audiobook",
        route: "Goodreads",
      },
    ],
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
  it("keeps the sibling panel informational with the frozen copy and zero mutations", async () => {
    const work = makeWork({
      formatNeeded: null,
      ebook: { state: "NowhereToLook" },
      audiobook: { state: "NowhereToLook" },
    });
    const api = installWorkRoute(work);
    const mounted = mountWorkRoute();
    try {
      await vi.waitFor(() => expect(mounted.container.textContent).toContain(FROZEN_SIBLING_COPY));
      const panel = mounted.container.querySelector<HTMLElement>(
        '[data-testid="identity-sibling-panel"]',
      );
      expect(panel).not.toBeNull();
      expect(panel?.querySelectorAll("button")).toHaveLength(0);

      const siblingAffordances = Array.from(
        panel?.querySelectorAll<HTMLAnchorElement>("[data-sibling-affordance]") ?? [],
      );
      expect(siblingAffordances).toHaveLength(2);
      for (const affordance of siblingAffordances) {
        affordance.addEventListener("click", (event) => event.preventDefault(), { once: true });
        await act(async () => {
          affordance.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true }));
        });
      }

      expect(api.calls.filter((call) => call.method !== "GET")).toEqual([]);
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
