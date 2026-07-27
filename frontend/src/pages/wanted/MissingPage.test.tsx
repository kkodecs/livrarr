import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import MissingPage from "./MissingPage";
import HistoryPage from "@/pages/activity/history/HistoryPage";
import QueuePage from "@/pages/activity/queue/QueuePage";
import SearchPage from "@/pages/search/SearchPage";
import { listWorks, getQueue } from "@/api";
import type { PaginatedResponse, WorkDetailResponse } from "@/types/api";

// Only the network boundary is stubbed. The extra stubs beyond listWorks are the
// other @/api calls the three sibling consumers make; they exist so those pages
// can be mounted for real in the shared-cache test below.
vi.mock("@/api", () => ({
  listWorks: vi.fn(),
  getHistory: vi.fn().mockResolvedValue({ items: [] }),
  getQueue: vi.fn(),
  getMetadataConfig: vi.fn().mockResolvedValue({ languages: ["en"] }),
  removeQueueItem: vi.fn(),
  retryImport: vi.fn(),
  lookupWorks: vi.fn(),
  addWork: vi.fn(),
}));

const PAGE_SIZE = 1000;

function makeWork(
  id: number,
  over: Partial<WorkDetailResponse> = {},
): WorkDetailResponse {
  return {
    id,
    title: `Work ${id}`,
    authorName: `Author ${id}`,
    authorId: null,
    year: 2020,
    addedAt: "2026-07-01T00:00:00.000Z",
    monitorEbook: false,
    monitorAudiobook: false,
    libraryItems: [],
    ...over,
  } as unknown as WorkDetailResponse;
}

function ebookItem(id: number) {
  return {
    id,
    path: `/library/${id}.epub`,
    mediaType: "ebook" as const,
    fileSize: 1,
    importedAt: "2026-07-01T00:00:00.000Z",
    progressPct: null,
    durationSeconds: null,
    finishedAt: null,
  };
}

function audiobookItem(id: number) {
  return {
    id,
    path: `/library/${id}.m4b`,
    mediaType: "audiobook" as const,
    fileSize: 1,
    importedAt: "2026-07-01T00:00:00.000Z",
    progressPct: null,
    durationSeconds: null,
    finishedAt: null,
  };
}

/** Serve a paginated works library the way GET /work does: one page per call. */
function pagedMock(pages: Record<number, WorkDetailResponse[]>, total: number) {
  vi.mocked(listWorks).mockImplementation((params?: { page?: number }) => {
    const page = params?.page ?? 1;
    const resp: PaginatedResponse<WorkDetailResponse> = {
      items: pages[page] ?? [],
      total,
      page,
      pageSize: PAGE_SIZE,
    };
    return Promise.resolve(resp);
  });
}

function pageResponse(
  page: number,
  items: WorkDetailResponse[],
  total: number,
): PaginatedResponse<WorkDetailResponse> {
  return { items, total, page, pageSize: PAGE_SIZE };
}

/** Library noise: unmonitored, fully-owned works that must never be listed. */
function filler(id: number): WorkDetailResponse {
  return makeWork(id, { libraryItems: [ebookItem(id), audiobookItem(id)] });
}

/** A promise the test resolves by hand, to hold one page of the walk in flight. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

function tabButton(container: HTMLElement, label: string): HTMLButtonElement {
  const btn = Array.from(container.querySelectorAll("button")).find((b) =>
    b.textContent?.startsWith(label),
  );
  if (!btn) throw new Error(`no tab button labelled "${label}"`);
  return btn;
}

/** Each tab button carries its badge count in a trailing <span>. */
function tabCount(container: HTMLElement, label: string): string {
  return tabButton(container, label).querySelector("span")?.textContent ?? "";
}

async function clickTab(container: HTMLElement, label: string) {
  const btn = tabButton(container, label);
  await act(async () => {
    btn.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

/** Titles of the rows the missing table renders, in row order. */
function rowTitles(container: HTMLElement): string[] {
  return Array.from(container.querySelectorAll("tbody tr")).map(
    (tr) => tr.querySelectorAll("td")[1]?.textContent?.trim() ?? "",
  );
}

/** Missing-badge labels rendered on one work's row. */
function badgesFor(container: HTMLElement, workId: number): string[] {
  return Array.from(container.querySelectorAll('a[href*="tab=releases"]'))
    .filter((a) => (a.getAttribute("href") ?? "").startsWith(`/work/${workId}?`))
    .map((a) => a.textContent?.trim() ?? "");
}

/** Mount any page against a caller-supplied client, so several can share one cache. */
function mountWith(queryClient: QueryClient, ui: ReactNode) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(
      <MemoryRouter>
        <QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>
      </MemoryRouter>,
    );
  });
  return {
    container,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function newClient() {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
}

function renderPage() {
  const queryClient = newClient();
  return { queryClient, ...mountWith(queryClient, <MissingPage />) };
}

beforeEach(() => {
  vi.mocked(listWorks).mockClear();
});

describe("MissingPage — issue #177 reproductions", () => {
  // AC-001 (REQ-001): a work monitored for ebook only, owning an audiobook file but
  // no ebook file, sitting beyond the first works page, must be listed.
  it("lists a monitored ebook-missing work that lives on the last of 3 pages", async () => {
    const target = makeWork(2400, {
      title: "The Last Page Target",
      monitorEbook: true,
      libraryItems: [audiobookItem(9001)],
    });
    pagedMock(
      {
        1: Array.from({ length: PAGE_SIZE }, (_, i) =>
          makeWork(i + 1, { libraryItems: [ebookItem(i + 1)] }),
        ),
        2: Array.from({ length: PAGE_SIZE }, (_, i) =>
          makeWork(1000 + i + 1, { libraryItems: [ebookItem(1000 + i + 1)] }),
        ),
        3: [
          ...Array.from({ length: 499 }, (_, i) =>
            makeWork(2000 + i + 1, { libraryItems: [ebookItem(2000 + i + 1)] }),
          ),
          target,
        ],
      },
      2500,
    );

    const { container, cleanup } = renderPage();
    try {
      // Assert on the rendered rows, not ambient page text.
      await vi.waitFor(
        () => expect(rowTitles(container)).toContain("The Last Page Target"),
        { timeout: 3000 },
      );
      // AC-001 names both tabs: the ebook-filtered view must list it too.
      await clickTab(container, "Ebooks");
      expect(rowTitles(container)).toEqual(["The Last Page Target"]);
    } finally {
      cleanup();
    }
  });

  // AC-006 (REQ-004): badges name only monitored-and-missing formats — a work
  // monitored for audiobook only must not carry an "Ebook" missing badge.
  it("does not badge an unmonitored format as missing", async () => {
    const audioOnly = makeWork(5, {
      title: "Audio Only Wanted",
      monitorAudiobook: true,
      libraryItems: [],
    });
    pagedMock(
      { 1: [audioOnly, ...Array.from({ length: 20 }, (_, i) => makeWork(100 + i))] },
      21,
    );

    const { container, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () => expect(container.textContent).toContain("Audio Only Wanted"),
        { timeout: 3000 },
      );
      const badges = Array.from(
        container.querySelectorAll('a[href*="tab=releases"]'),
      ).filter((a) => (a.getAttribute("href") ?? "").includes("/work/5"));
      const badgeTexts = badges.map((a) => a.textContent?.trim());
      expect(badgeTexts).toContain("Audiobook");
      expect(badgeTexts).not.toContain("Ebook");
    } finally {
      cleanup();
    }
  });
});

describe("MissingPage — complete-library answer", () => {
  // AC-002 (REQ-001): the per-media-type rule is exact. The subjects sit on page 2
  // so only a completed walk can reach them.
  it("lists a work for every monitored type it lacks, and only those", async () => {
    const dualEbookPresent = makeWork(2001, {
      title: "Dual Monitored Ebook Present",
      monitorEbook: true,
      monitorAudiobook: true,
      libraryItems: [ebookItem(2001)],
    });
    const allPresent = makeWork(2002, {
      title: "Fully Present Work",
      monitorEbook: true,
      monitorAudiobook: true,
      libraryItems: [ebookItem(2002), audiobookItem(2002)],
    });
    const unmonitored = makeWork(2003, {
      title: "Unmonitored Orphan",
      libraryItems: [],
    });
    pagedMock(
      {
        1: Array.from({ length: PAGE_SIZE }, (_, i) => filler(i + 1)),
        2: [dualEbookPresent, allPresent, unmonitored],
      },
      PAGE_SIZE + 3,
    );

    const { container, cleanup } = renderPage();
    try {
      // (a) listed under All; (b) the fully-present work and (c) the unmonitored
      // one are absent — the exact row list is the assertion for all three.
      await vi.waitFor(
        () =>
          expect(rowTitles(container)).toEqual(["Dual Monitored Ebook Present"]),
        { timeout: 5000 },
      );
      // AC-006, second clause: the dual-monitored work badges Audiobook only.
      expect(badgesFor(container, 2001)).toEqual(["Audiobook"]);

      await clickTab(container, "Audiobooks");
      expect(rowTitles(container)).toEqual(["Dual Monitored Ebook Present"]);

      await clickTab(container, "Ebooks");
      expect(rowTitles(container)).toEqual([]);
      expect(container.textContent).toContain("No missing items");
    } finally {
      cleanup();
    }
  });

  // AC-003 (REQ-002): badge counts are library-wide and per-type, with qualifying
  // works on non-adjacent pages (1 and 3).
  it("counts missing works across non-adjacent pages, per media type", async () => {
    const p1Ebook = makeWork(1, {
      title: "Page One Ebook Wanted",
      monitorEbook: true,
      libraryItems: [],
    });
    const p3Dual = makeWork(2401, {
      title: "Page Three Dual",
      monitorEbook: true,
      monitorAudiobook: true,
      libraryItems: [ebookItem(2401)],
    });
    const p3Audio = makeWork(2402, {
      title: "Page Three Audio Wanted",
      monitorAudiobook: true,
      libraryItems: [],
    });
    pagedMock(
      {
        1: [
          p1Ebook,
          ...Array.from({ length: PAGE_SIZE - 1 }, (_, i) => filler(i + 2)),
        ],
        2: Array.from({ length: PAGE_SIZE }, (_, i) => filler(1000 + i + 1)),
        3: [
          ...Array.from({ length: 498 }, (_, i) => filler(2000 + i + 1)),
          p3Dual,
          p3Audio,
        ],
      },
      2500,
    );

    const { container, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () => expect(tabCount(container, "All Missing")).toBe("3"),
        { timeout: 5000 },
      );
      // p1Ebook only; p3Dual owns its ebook, p3Audio is not monitored for ebook.
      expect(tabCount(container, "Ebooks")).toBe("1");
      expect(tabCount(container, "Audiobooks")).toBe("2");
    } finally {
      cleanup();
    }
  });

  // AC-004 (REQ-003): a half-walked library is never presented as a final answer.
  it("shows loading, never the empty state, while a later page is in flight", async () => {
    const target = makeWork(1500, {
      title: "Second Page Wanted",
      monitorEbook: true,
      libraryItems: [],
    });
    const page2 = deferred<PaginatedResponse<WorkDetailResponse>>();
    vi.mocked(listWorks).mockImplementation((params?: { page?: number }) => {
      const page = params?.page ?? 1;
      if (page === 2) return page2.promise;
      return Promise.resolve(
        pageResponse(
          page,
          page === 1
            ? Array.from({ length: PAGE_SIZE }, (_, i) => filler(i + 1))
            : [],
          2500,
        ),
      );
    });

    const { container, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () =>
          expect(
            vi.mocked(listWorks).mock.calls.some((c) => c[0]?.page === 2),
          ).toBe(true),
        { timeout: 5000 },
      );
      expect(container.querySelector(".animate-spin")).not.toBeNull();
      expect(container.textContent).not.toContain("No missing items");

      await act(async () => {
        page2.resolve(pageResponse(2, [target], 2500));
        await page2.promise;
      });
      await vi.waitFor(
        () => expect(rowTitles(container)).toEqual(["Second Page Wanted"]),
        { timeout: 5000 },
      );
    } finally {
      cleanup();
    }
  });

  // AC-004 (REQ-003): a failed page surfaces the error state with retry, never a
  // false "all clear".
  it("shows the error state with retry when a page of the walk fails", async () => {
    vi.mocked(listWorks).mockImplementation((params?: { page?: number }) => {
      const page = params?.page ?? 1;
      if (page === 2) return Promise.reject(new Error("page 2 unavailable"));
      return Promise.resolve(
        pageResponse(
          page,
          Array.from({ length: PAGE_SIZE }, (_, i) => filler(i + 1)),
          2500,
        ),
      );
    });

    const { container, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () =>
          expect(
            Array.from(container.querySelectorAll("button")).some((b) =>
              b.textContent?.includes("Retry"),
            ),
          ).toBe(true),
        { timeout: 5000 },
      );
      expect(container.textContent).toContain("Something went wrong");
      expect(container.textContent).not.toContain("No missing items");
    } finally {
      cleanup();
    }
  });

  // REQ-001/REQ-003 (review R-003): the library can grow while the walk is in
  // flight — a running list import adds works continuously. Page 1 reports a
  // library of 2 pages, page 2 reports 3; the only missing work is on page 3.
  it("keeps walking when a later page reports a bigger library", async () => {
    const late = makeWork(2100, {
      title: "Arrived Mid Walk",
      monitorEbook: true,
      libraryItems: [],
    });
    const totalByPage: Record<number, number> = { 1: 1500, 2: 2500, 3: 2500 };
    vi.mocked(listWorks).mockImplementation((params?: { page?: number }) => {
      const page = params?.page ?? 1;
      const items =
        page === 3
          ? [late]
          : Array.from({ length: PAGE_SIZE }, (_, i) =>
              filler(page * 10_000 + i),
            );
      return Promise.resolve(
        pageResponse(page, items, totalByPage[page] ?? 2500),
      );
    });

    const { container, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () => expect(rowTitles(container)).toEqual(["Arrived Mid Walk"]),
        { timeout: 5000 },
      );
    } finally {
      cleanup();
    }
  });

  // AC-005 (REQ-001): a <=1,000-work library renders exactly as before, and asks
  // for one page only — the page size requested must match the walk's divisor.
  it("renders a single-page library unchanged, with one request", async () => {
    const wanted = makeWork(1, {
      title: "Small Library Wanted",
      monitorEbook: true,
      libraryItems: [],
    });
    const owned = makeWork(2, {
      title: "Small Library Owned",
      monitorEbook: true,
      libraryItems: [ebookItem(2)],
    });
    const unmonitored = makeWork(3, {
      title: "Small Library Unmonitored",
      libraryItems: [],
    });
    pagedMock({ 1: [wanted, owned, unmonitored] }, 3);

    const { container, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () => expect(rowTitles(container)).toEqual(["Small Library Wanted"]),
        { timeout: 5000 },
      );
      expect(tabCount(container, "All Missing")).toBe("1");
      expect(tabCount(container, "Ebooks")).toBe("1");
      expect(tabCount(container, "Audiobooks")).toBe("0");
      expect(listWorks).toHaveBeenCalledTimes(1);
      expect(listWorks).toHaveBeenCalledWith({ page: 1, pageSize: PAGE_SIZE });
    } finally {
      cleanup();
    }
  });

  // AC-007 (REQ-005): the walk must not create, replace or reshape the ["works"]
  // entry Search, Queue and History read as one paginated page — and its own key
  // stays "works"-prefixed so existing invalidations still cover it.
  it('never writes the shared ["works"] cache entry', async () => {
    const target = makeWork(1200, {
      title: "Cache Seam Target",
      monitorAudiobook: true,
      libraryItems: [],
    });
    pagedMock(
      {
        1: Array.from({ length: PAGE_SIZE }, (_, i) => filler(i + 1)),
        2: [
          target,
          ...Array.from({ length: 499 }, (_, i) => filler(2000 + i + 1)),
        ],
      },
      1500,
    );

    const { container, queryClient, cleanup } = renderPage();
    try {
      await vi.waitFor(
        () => expect(rowTitles(container)).toEqual(["Cache Seam Target"]),
        { timeout: 5000 },
      );

      expect(queryClient.getQueryData(["works"])).toBeUndefined();

      const cached = queryClient.getQueryCache().getAll();
      expect(cached).toHaveLength(1);
      const missingKey = cached[0]?.queryKey ?? [];
      expect(missingKey[0]).toBe("works");
      expect(missingKey.length).toBeGreaterThan(1);
      // The walk's entry holds the whole library as a raw array, not the
      // paginated response shape the siblings expect from ["works"].
      expect(queryClient.getQueryData(missingKey)).toHaveLength(1500);
    } finally {
      cleanup();
    }
  });
});

describe("MissingPage — sibling consumers of the shared works cache", () => {
  // AC-007 (REQ-005), sibling half. The cache-mechanism test above proves the walk
  // does not touch ["works"]; this one proves the three real consumers still work,
  // by mounting each of them for real against the same client after the walk ran.
  it("leaves History, Queue and Search rendering from the paginated works response", async () => {
    const target = makeWork(1200, {
      title: "Sibling Seam Target",
      monitorAudiobook: true,
      libraryItems: [],
    });
    pagedMock(
      {
        1: Array.from({ length: PAGE_SIZE }, (_, i) => filler(i + 1)),
        2: [
          target,
          ...Array.from({ length: 499 }, (_, i) => filler(2000 + i + 1)),
        ],
      },
      1500,
    );
    vi.mocked(getQueue).mockResolvedValue({
      items: [
        {
          id: 1,
          title: "Some Release v1",
          status: "sent",
          size: 1024,
          mediaType: "ebook",
          indexer: "TestIndexer",
          downloadClient: "TestClient",
          // Resolved through the shared works data by workName().
          workId: 1,
          protocol: "torrent",
          error: null,
          grabbedAt: "2026-07-01T00:00:00.000Z",
          progress: null,
        },
      ],
      total: 1,
      page: 1,
      perPage: 25,
    });

    const queryClient = newClient();

    // 1. The Missing page walks the whole 2-page library, then goes away.
    const missing = mountWith(queryClient, <MissingPage />);
    try {
      await vi.waitFor(
        () => expect(rowTitles(missing.container)).toEqual(["Sibling Seam Target"]),
        { timeout: 5000 },
      );
    } finally {
      missing.cleanup();
    }
    expect(queryClient.getQueryData(["works", "missing-all"])).toHaveLength(1500);
    // From here on, every listWorks call belongs to a sibling.
    vi.mocked(listWorks).mockClear();

    // 2. History renders its list only once `works` is defined — if the walk had
    //    replaced ["works"] with its raw array, `select: res => res.items` would
    //    yield undefined and this page would sit on its loading spinner forever.
    const history = mountWith(queryClient, <HistoryPage />);
    try {
      await vi.waitFor(
        () => expect(history.container.textContent).toContain("No activity yet"),
        { timeout: 5000 },
      );
    } finally {
      history.cleanup();
    }

    // 3. Queue names the grabbed work out of the same shared data: the real title
    //    "Work 1", not workName()'s "Work #1" not-found fallback.
    const queue = mountWith(queryClient, <QueuePage />);
    try {
      await vi.waitFor(
        () => expect(queue.container.textContent).toContain("Some Release v1"),
        { timeout: 5000 },
      );
      const workCell = Array.from(
        queue.container.querySelectorAll('tbody a[href^="/work/"]'),
      ).map((a) => a.textContent?.trim());
      expect(workCell).toEqual(["Work 1"]);
    } finally {
      queue.cleanup();
    }

    // 4. Search renders its normal no-query view (the search form).
    const search = mountWith(queryClient, <SearchPage />);
    try {
      await vi.waitFor(
        () =>
          expect(
            search.container.querySelector('input[placeholder^="Search by title"]'),
          ).not.toBeNull(),
        { timeout: 5000 },
      );
    } finally {
      search.cleanup();
    }

    // The siblings' contract is unchanged: a bare listWorks() with no paging args.
    // One call for all three — they share the ["works"] entry, which is the point.
    const siblingCalls = vi.mocked(listWorks).mock.calls;
    expect(siblingCalls).toHaveLength(1);
    expect(siblingCalls.every((c) => c[0] === undefined)).toBe(true);

    // And ["works"] holds only that single page, in the paginated response shape —
    // never the walk's 1500-item array.
    const shared = queryClient.getQueryData<PaginatedResponse<WorkDetailResponse>>(
      ["works"],
    );
    expect(shared).toBeDefined();
    expect(Array.isArray(shared)).toBe(false);
    expect(shared?.items).toHaveLength(PAGE_SIZE);
    expect(shared?.total).toBe(1500);
    // The walk's own entry is still there, still separate.
    expect(queryClient.getQueryData(["works", "missing-all"])).toHaveLength(1500);
  });
});
