import {
  expect,
  test as base,
  type Browser,
  type Page,
} from "@playwright/test";
import {
  cfiAfter,
  renditionShows,
  renditionStartCfi,
  spineStep,
} from "./epubReader";
import {
  clientError,
  emptyOk,
  expectNoToast,
  holdRequests,
  notFound,
  ok,
  sampleBook,
  savedAt,
  serverError,
  stubApi,
  waitForHeld,
  type ApiCall,
  type Reply,
} from "./stubbedApi";

// Opening an EPUB whose saved place cannot be read shows an error screen
// with Retry and "Read from the beginning", and nothing is saved until the
// saved place is known. Reading from the beginning saves nothing for the
// rest of that session.

const LOAD_ERROR = "Could not load this book.";
const DETAIL = "Your saved place could not be loaded.";
const NOTE =
  "Your saved place could not be loaded. Progress is not being saved this time.";
const FROM_START = "Read from the beginning";

const PROGRESS = /^\/workfile\/(\d+)\/progress$/;

interface ReaderSetup {
  progress: (id: number) => Reply;
  download?: (id: number) => Reply;
  anchors?: (id: number) => Reply;
  prompt?: (id: number) => Reply;
}

const epub = (): Reply => ({
  status: 200,
  bytes: sampleBook("sample.epub"),
  contentType: "application/epub+zip",
});

function stubReader(page: Page, setup: ReaderSetup) {
  return stubApi(page, (call) => {
    const bare = call.path.split("?")[0]!;
    if (call.method === "PUT" && PROGRESS.test(bare))
      return ok({ success: true });
    if (call.method === "POST" && /^\/workfile\/\d+\/bookmarks$/.test(bare))
      return ok({ id: 50 });
    if (call.method !== "GET") return undefined;
    let m = /^\/workfile\/(\d+)$/.exec(bare);
    if (m)
      return ok({
        id: Number(m[1]),
        path: `Sample Author/Sample ${m[1]}.epub`,
        mediaType: "ebook",
        fileSize: 1000,
        importedAt: "2026-09-01T00:00:00Z",
        progressPct: null,
        durationSeconds: null,
        finishedAt: null,
      });
    m = /^\/workfile\/(\d+)\/download$/.exec(bare);
    if (m) return setup.download?.(Number(m[1])) ?? epub();
    m = /^\/workfile\/(\d+)\/bookmarks$/.exec(bare);
    if (m) return ok([]);
    m = PROGRESS.exec(bare);
    if (m) return setup.progress(Number(m[1]));
    m = /^\/workfile\/(\d+)\/cross-format\/anchors$/.exec(bare);
    if (m) return setup.anchors?.(Number(m[1])) ?? notFound("none");
    m = /^\/workfile\/(\d+)\/cross-format\/prompt$/.exec(bare);
    if (m) return setup.prompt?.(Number(m[1])) ?? ok(null);
    return undefined;
  });
}

/**
 * Positions in the sample book, read from the real rendition in the fixed
 * viewport below. Each saved place is where the rendition itself reports a
 * page starting inside a section, past that section's first page, and opening
 * the book there starts the page there again.
 */
interface Places {
  /** Where the book opens with no saved place. */
  start: string;
  /** Item 1's saved place, inside section 10. */
  item1: string;
  /** Item 2's saved place, inside section 6. */
  item2: string;
  /** The resume prompt's place, inside section 8. */
  jump: string;
}

const VIEWPORT = { width: 1280, height: 720 };

/** The rendition's start position once it has stopped moving. */
async function settledStart(page: Page): Promise<string> {
  let last: string | null = null;
  let same = 0;
  for (let i = 0; i < 60; i++) {
    await page.waitForTimeout(250);
    const cfi = await renditionStartCfi(page);
    same = cfi !== null && cfi === last ? same + 1 : 0;
    last = cfi;
    if (same >= 4) return cfi!;
  }
  throw new Error(`the rendition did not settle; last start ${last}`);
}

/** Where the book opens, in a fresh page, when the saved-place read answers `progress`. */
async function opensAt(
  browser: Browser,
  baseURL: string | undefined,
  progress: Reply,
): Promise<string> {
  const context = await browser.newContext({ viewport: VIEWPORT, baseURL });
  try {
    const page = await context.newPage();
    await stubReader(page, { progress: () => progress });
    await page.goto("/read/1");
    await expect(page.locator('button[title="Add bookmark"]')).toBeVisible({
      timeout: 15_000,
    });
    return await settledStart(page);
  } finally {
    await context.close();
  }
}

/** A page start inside spine section `section`, past its first page. */
async function pageStartInside(
  browser: Browser,
  baseURL: string | undefined,
  section: number,
): Promise<string> {
  const sectionStart = `epubcfi(/6/${section}!/4/2/1:0)`;
  const place = await opensAt(
    browser,
    baseURL,
    savedAt(`epubcfi(/6/${section}!/4/26/1:272)`),
  );
  expect(spineStep(place), `${place} is in section ${section}`).toBe(section);
  expect(
    cfiAfter(place, sectionStart),
    `${place} is past the first page of section ${section}`,
  ).toBe(true);
  expect(
    await opensAt(browser, baseURL, savedAt(place)),
    `opening at ${place} starts the page there`,
  ).toBe(place);
  return place;
}

const test = base.extend<object, { places: Places }>({
  places: [
    async ({ browser }, use, workerInfo) => {
      const baseURL = workerInfo.project.use.baseURL;
      const places: Places = {
        start: await opensAt(browser, baseURL, notFound("no progress")),
        item1: await pageStartInside(browser, baseURL, 10),
        item2: await pageStartInside(browser, baseURL, 6),
        jump: await pageStartInside(browser, baseURL, 8),
      };
      await use(places);
    },
    { scope: "worker" },
  ],
});
test.use({ viewport: VIEWPORT });

function saves(calls: ApiCall[], id?: number) {
  return calls.filter(
    (c) =>
      c.method === "PUT" &&
      PROGRESS.test(c.path) &&
      (id === undefined || c.path === `/workfile/${id}/progress`),
  );
}

function reads(calls: ApiCall[], id: number) {
  return calls.filter(
    (c) => c.method === "GET" && c.path === `/workfile/${id}/progress`,
  );
}

const retry = (page: Page) => page.getByRole("button", { name: "Retry" });
const fromStart = (page: Page) =>
  page.getByRole("button", { name: FROM_START });
const note = (page: Page) => page.getByText(NOTE, { exact: true });
const loading = (page: Page) => page.getByText("Loading...", { exact: true });

/** The saved-place error screen, with both actions, and no book. */
async function expectSavedPlaceError(page: Page) {
  await expect(page.getByText(LOAD_ERROR)).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText(DETAIL, { exact: true })).toBeVisible();
  await expect(retry(page)).toBeVisible();
  await expect(fromStart(page)).toBeVisible();
  await expect(page.locator(".epub-container")).toHaveCount(0);
  await expect(page.locator('button[title="Add bookmark"]')).toHaveCount(0);
}

/** The book is open and the rendition's start position is `cfi`. */
async function expectOpenAt(page: Page, cfi: string) {
  await expect(page.locator('button[title="Add bookmark"]')).toBeVisible({
    timeout: 15_000,
  });
  await expect
    .poll(() => renditionStartCfi(page), {
      message: `the book opens at ${cfi}`,
      timeout: 15_000,
    })
    .toBe(cfi);
}

/** The rendition's start position is still `cfi`. */
async function expectStillAt(page: Page, cfi: string, what: string) {
  expect(await renditionStartCfi(page), what).toBe(cfi);
}

/** No error screen, no note, no toast. */
async function expectNoMessage(page: Page) {
  await expect(page.getByText(LOAD_ERROR)).toHaveCount(0);
  await expect(page.getByText(DETAIL)).toHaveCount(0);
  await expectNoToast(page, 0);
}

/** Page positions are known, so a relocation has a percentage and would save. */
async function waitForLocations(page: Page, id: number) {
  await expect
    .poll(
      () =>
        page.evaluate(
          (key) => localStorage.getItem(key),
          `livrarr-locations-${id}`,
        ),
      {
        message: `item ${id}'s page positions are known`,
        timeout: 30_000,
      },
    )
    .not.toBeNull();
}

/** The toolbar's reading percentage, 0 when it is not shown. */
async function shownPercent(page: Page): Promise<number> {
  const label = page.locator("span.tabular-nums");
  if ((await label.count()) === 0) return 0;
  return Number((await label.innerText()).replace("%", ""));
}

/**
 * Turns the page `turns` times over about five seconds, then waits five
 * more. The toolbar's percentage proves the turns reached places that a
 * normal session saves.
 */
async function turnPagesAndWait(page: Page, id: number, turns: number) {
  await waitForLocations(page, id);
  await page.evaluate(() =>
    (document.activeElement as HTMLElement | null)?.blur(),
  );
  for (let i = 0; i < turns; i++) {
    await page.keyboard.press("ArrowRight");
    await page.waitForTimeout(5_000 / turns);
  }
  await expect
    .poll(() => shownPercent(page), {
      message: "the page turns reached a later place",
    })
    .toBeGreaterThan(0);
  await page.waitForTimeout(5_000);
}

/** One page turn sends a save to item `id`'s progress. */
async function expectTurnSaves(page: Page, calls: ApiCall[], id: number) {
  await waitForLocations(page, id);
  const before = saves(calls, id).length;
  await page.evaluate(() =>
    (document.activeElement as HTMLElement | null)?.blur(),
  );
  await page.keyboard.press("ArrowRight");
  await expect
    .poll(() => saves(calls, id).length, {
      message: `a page turn saves to /workfile/${id}/progress`,
      timeout: 8_000,
    })
    .toBeGreaterThan(before);
}

/** Opens item `id` in the same page, as a link inside the app does. */
async function openItemInPage(page: Page, id: number) {
  await page.evaluate((itemId) => {
    window.history.pushState(null, "", `/read/${itemId}`);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, id);
}

const failedReads: Array<[string, Reply]> = [
  ["a 500", serverError("progress read failed")],
  ["a network failure", "network-error"],
  ["a 200 with no body", emptyOk],
  ["a 403", clientError(403, "forbidden")],
  ["a 200 with null", ok(null)],
  ["a 200 with {}", ok({})],
  ['a 200 with { "position": 12 }', ok({ position: 12 })],
  ['a 200 with { "position": null }', ok({ position: null })],
];

test.describe("Opening an EPUB whose saved place cannot be read", () => {
  // AC-023 (500, network failure, no body) and AC-053 (the rest).
  for (const [label, failure] of failedReads) {
    test(`${label}: the error screen with both actions, and no save`, async ({
      page,
    }) => {
      const calls = await stubReader(page, { progress: () => failure });
      await page.goto("/read/1");
      await expect.poll(() => reads(calls, 1).length).toBe(1);
      await expectSavedPlaceError(page);
      await page.waitForTimeout(5_000);
      expect(saves(calls), "no save").toEqual([]);
      await expectSavedPlaceError(page);
    });
  }

  // AC-024 (from a 500) and AC-053 (from {}).
  for (const [label, failure] of [
    ["a 500", serverError("progress read failed")],
    ["a 200 with {}", ok({})],
  ] as Array<[string, Reply]>) {
    test(`Retry after ${label} reads again and opens at the saved place, which then saves`, async ({
      page,
      places,
    }) => {
      let progress: Reply = failure;
      const calls = await stubReader(page, { progress: () => progress });
      await page.goto("/read/1");
      await expectSavedPlaceError(page);
      progress = savedAt(places.item1);
      await retry(page).click();
      await expect.poll(() => reads(calls, 1).length).toBe(2);
      await expectOpenAt(page, places.item1);
      await expectNoMessage(page);
      await expectTurnSaves(page, calls, 1);
    });
  }

  // AC-025.
  test("a held Retry shows Loading..., then fails back to the error screen", async ({
    page,
  }) => {
    const calls = await stubReader(page, {
      progress: () => serverError("progress read failed"),
    });
    await page.goto("/read/1");
    await expectSavedPlaceError(page);
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await retry(page).click();
    await waitForHeld(held, 1, "the second saved-place read");
    await expect(loading(page)).toBeVisible();
    await page.waitForTimeout(2_000);
    await expect(loading(page)).toBeVisible();
    await held[0]!.release(serverError("progress read failed"));
    await expectSavedPlaceError(page);
    await page.waitForTimeout(2_000);
    expect(saves(calls), "no save").toEqual([]);
  });

  // AC-029 (from a 500) and AC-053 (from { "position": 12 }).
  for (const [label, failure] of [
    ["a 500", serverError("progress read failed")],
    ['a 200 with { "position": 12 }', ok({ position: 12 })],
  ] as Array<[string, Reply]>) {
    test(`"${FROM_START}" after ${label} opens at the start with the note, and page turns save nothing`, async ({
      page,
      places,
    }) => {
      const calls = await stubReader(page, { progress: () => failure });
      await page.goto("/read/1");
      await expectSavedPlaceError(page);
      await fromStart(page).click();
      await expectOpenAt(page, places.start);
      await expect(note(page)).toBeVisible();
      await turnPagesAndWait(page, 1, 3);
      expect(saves(calls), "no save").toEqual([]);
      await expect(note(page)).toBeVisible();
    });
  }

  // AC-030.
  test(`in a "${FROM_START}" session, Add bookmark saves the bookmark and no place`, async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: () => serverError("progress read failed"),
    });
    await page.goto("/read/1");
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await turnPagesAndWait(page, 1, 1);
    await page.locator('button[title="Add bookmark"]').click();
    await expect
      .poll(
        () =>
          calls.filter(
            (c) => c.method === "POST" && c.path === "/workfile/1/bookmarks",
          ).length,
      )
      .toBe(1);
    await page.waitForTimeout(5_000);
    expect(saves(calls), "no save").toEqual([]);
  });

  // AC-031.
  test(`in a "${FROM_START}" session, the resume banner's Jump moves the reader and saves nothing`, async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: () => serverError("progress read failed"),
      anchors: () => ok([{ cfi: "epubcfi(/6/2!/4/2/1:0)", ts: 0 }]),
      prompt: () =>
        ok({
          format: "audiobook",
          position: places.jump,
          label: "Chapter 4",
        }),
    });
    await page.goto("/read/1");
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await turnPagesAndWait(page, 1, 1);
    await page.getByRole("button", { name: "Jump" }).click();
    // The banner and the note change where pages break, so the page start
    // differs from a session without them; the prompt's place is on screen.
    await expect
      .poll(() => renditionShows(page, places.jump), {
        message: `the reader shows ${places.jump}`,
        timeout: 15_000,
      })
      .toBe(true);
    await page.waitForTimeout(5_000);
    expect(saves(calls), "no save").toEqual([]);
    await expect(note(page)).toBeVisible();
  });

  // AC-032.
  test(`a reload ends the "${FROM_START}" session`, async ({
    page,
    places,
  }) => {
    let progress: Reply = serverError("progress read failed");
    const calls = await stubReader(page, { progress: () => progress });
    await page.goto("/read/1");
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await expect(note(page)).toBeVisible();
    progress = savedAt(places.item1);
    await page.reload();
    await expectOpenAt(page, places.item1);
    await expect(page.getByText(DETAIL)).toHaveCount(0);
    await expectTurnSaves(page, calls, 1);
  });

  // AC-033.
  test("when the download fails too, the download's screen shows, and its Retry reads the saved place again", async ({
    page,
    places,
  }) => {
    let download: Reply = serverError("download failed");
    let progress: Reply = serverError("progress read failed");
    const calls = await stubReader(page, {
      progress: () => progress,
      download: () => download,
    });
    await page.goto("/read/1");
    await expect(page.getByText(LOAD_ERROR)).toBeVisible({ timeout: 10_000 });
    await expect.poll(() => reads(calls, 1).length).toBe(1);
    await expect(retry(page)).toBeVisible();
    await expect(fromStart(page)).toHaveCount(0);
    download = epub();
    progress = savedAt(places.item1);
    await retry(page).click();
    await expectOpenAt(page, places.item1);
  });
});

test.describe("Opening an EPUB whose saved place reads normally", () => {
  // AC-026 and AC-054.
  for (const [label, reply] of [
    ["a 404", notFound("no progress")],
    ['a 200 with { "position": "" }', savedAt("")],
  ] as Array<[string, Reply]>) {
    test(`${label} opens at the beginning with no message, and a page turn saves`, async ({
      page,
      places,
    }) => {
      const calls = await stubReader(page, { progress: () => reply });
      await page.goto("/read/1");
      await expectOpenAt(page, places.start);
      await expectNoMessage(page);
      await expectTurnSaves(page, calls, 1);
    });
  }

  // AC-027.
  test("a slow read shows Loading... until it opens at the saved place", async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: () => savedAt(places.item1),
    });
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await page.goto("/read/1");
    await waitForHeld(held, 1, "the saved-place read");
    await expect(loading(page)).toBeVisible();
    await page.waitForTimeout(3_000);
    await expect(loading(page)).toBeVisible();
    expect(saves(calls), "no save while the read is held").toEqual([]);
    await held[0]!.release();
    await expectOpenAt(page, places.item1);
  });
});

test.describe("Changing book in the same page", () => {
  // AC-063.
  test(`a late success for the previous book changes nothing in a "${FROM_START}" session`, async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: (id) =>
        id === 1 ? savedAt(places.item1) : serverError("progress read failed"),
    });
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await page.goto("/read/1");
    await waitForHeld(held, 1, "item 1's saved-place read");
    await openItemInPage(page, 2);
    await expect.poll(() => reads(calls, 2).length).toBe(1);
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await held[0]!.release();
    await page.waitForTimeout(1_000);
    await expectStillAt(page, places.start, "the late reply leaves the reader where it was");
    await expect(note(page)).toBeVisible();
    await turnPagesAndWait(page, 2, 3);
    expect(saves(calls), "no save to either book").toEqual([]);
    await expect(note(page)).toBeVisible();
  });

  // AC-064.
  test("a late failure for the previous book changes nothing once the new book has opened", async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: (id) =>
        id === 1 ? serverError("progress read failed") : savedAt(places.item2),
    });
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await page.goto("/read/1");
    await waitForHeld(held, 1, "item 1's saved-place read");
    await openItemInPage(page, 2);
    await expectOpenAt(page, places.item2);
    await held[0]!.release();
    await page.waitForTimeout(2_000);
    await expect(page.getByText(LOAD_ERROR)).toHaveCount(0);
    await expectStillAt(page, places.item2, "the late reply leaves the reader where it was");
    await expectTurnSaves(page, calls, 2);
    expect(saves(calls, 1), "no save to item 1").toEqual([]);
  });

  // AC-065.
  test(`opening another book ends the "${FROM_START}" session`, async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: (id) =>
        id === 1 ? serverError("progress read failed") : savedAt(places.item2),
    });
    await page.goto("/read/1");
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await expect(note(page)).toBeVisible();
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/2/progress",
    );
    await openItemInPage(page, 2);
    await waitForHeld(held, 1, "item 2's saved-place read");
    await expect(loading(page)).toBeVisible();
    await held[0]!.release();
    await expectOpenAt(page, places.item2);
    await expect(note(page)).toHaveCount(0);
    await expectTurnSaves(page, calls, 2);
  });
});

test.describe("Returning to a book in the same page", () => {
  // F2, code review r1: opening A again after B is a new opening. Nothing
  // from A's earlier opening (its open book, its "Read from the beginning"
  // note) may show, and nothing may save, until the new read has answered.
  const heldPaths = (paths: string[]) => (url: URL, method: string) =>
    method === "GET" && paths.includes(url.pathname);

  /** Holds item 1's saved-place reads after the first one. */
  async function holdLaterReadsOfItem1(page: Page) {
    let seen = 0;
    return holdRequests(page, (url, method) => {
      if (method !== "GET" || url.pathname !== "/api/v1/workfile/1/progress")
        return false;
      seen += 1;
      return seen > 1;
    });
  }

  test(`after a "${FROM_START}" session on A and a visit to B, A waits for its new read and saves nothing`, async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: () => serverError("progress read failed"),
    });
    const later = await holdLaterReadsOfItem1(page);
    const itemTwo = await holdRequests(
      page,
      heldPaths(["/api/v1/workfile/2/progress", "/api/v1/workfile/2/download"]),
    );
    await page.goto("/read/1");
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await expect(note(page)).toBeVisible();

    await openItemInPage(page, 2);
    await waitForHeld(itemTwo.held, 2, "item 2's saved-place read and download");
    await openItemInPage(page, 1);
    await waitForHeld(later.held, 1, "item 1's new saved-place read");
    await expect(loading(page)).toBeVisible();
    await expect(page.locator(".epub-container")).toHaveCount(0);
    await expect(note(page)).toHaveCount(0);
    await page.waitForTimeout(3_000);
    await expect(loading(page)).toBeVisible();
    expect(saves(calls), "no save while the new read is held").toEqual([]);

    await later.held[0]!.release(serverError("progress read failed"));
    await expectSavedPlaceError(page);
    await page.waitForTimeout(2_000);
    expect(saves(calls), "no save after the new read fails").toEqual([]);

    await fromStart(page).click();
    await expectOpenAt(page, places.start);
    await expect(note(page)).toBeVisible();
    await turnPagesAndWait(page, 1, 3);
    expect(saves(calls), "no save to either book").toEqual([]);
    await expect(note(page)).toBeVisible();
  });

  test("after A was open at its saved place and a visit to B, A waits for its new read", async ({
    page,
    places,
  }) => {
    const calls = await stubReader(page, {
      progress: () => savedAt(places.item1),
    });
    const later = await holdLaterReadsOfItem1(page);
    const itemTwo = await holdRequests(
      page,
      heldPaths(["/api/v1/workfile/2/progress", "/api/v1/workfile/2/download"]),
    );
    await page.goto("/read/1");
    await expectOpenAt(page, places.item1);
    await waitForLocations(page, 1);

    await openItemInPage(page, 2);
    await waitForHeld(itemTwo.held, 2, "item 2's saved-place read and download");
    const before = saves(calls).length;
    await openItemInPage(page, 1);
    await waitForHeld(later.held, 1, "item 1's new saved-place read");
    await expect(loading(page)).toBeVisible();
    await expect(page.locator(".epub-container")).toHaveCount(0);
    await page.waitForTimeout(3_000);
    await expect(loading(page)).toBeVisible();
    expect(
      saves(calls).slice(before),
      "no save while the new read is held",
    ).toEqual([]);

    await later.held[0]!.release();
    await expectOpenAt(page, places.item1);
    await expectTurnSaves(page, calls, 1);
  });
});
