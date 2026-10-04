import { expect, test, type Page } from "@playwright/test";
import {
  buttonShows,
  controls,
  expectNoControls,
  expectPlaying,
  media,
  openItemInPage,
  playButton,
  progressReads,
  promptRequests,
  saves,
  stubPlayer,
  TWO_CHAPTERS,
  waitForItemMetadata,
  type PlayerSetup,
} from "./audioPlayer";
import {
  clientError,
  emptyOk,
  expectNoToast,
  expectOnlyToast,
  holdRequests,
  notFound,
  ok,
  savedAt,
  serverError,
  toastMark,
  waitForHeld,
  type ApiCall,
  type Reply,
} from "./stubbedApi";

// Opening an audiobook whose saved place cannot be read shows an error
// screen with Retry and "Play from the beginning"; until the saved place is
// known nothing can play or save. Playing from the beginning saves no place
// for the rest of that session.

const LOAD_ERROR = "Could not load this book.";
const DETAIL = "Your saved place could not be loaded.";
const NOTE =
  "Your saved place could not be loaded. Progress is not being saved this time.";
const FROM_START = "Play from the beginning";
const PLAY_FAILED = "Could not play this audiobook. Try reloading the page.";

const retry = (page: Page) => page.getByRole("button", { name: "Retry" });
const fromStart = (page: Page) =>
  page.getByRole("button", { name: FROM_START });
const note = (page: Page) => page.getByText(NOTE, { exact: true });

const prompt = (label: string) =>
  ok({ format: "ebook", position: "12.5", label });

/** Item 1 with two chapters, a resume prompt and bookmark writes that succeed. */
async function openPlayer(
  page: Page,
  setup: PlayerSetup,
  path = "/listen/1?workId=1",
) {
  const player = await stubPlayer(page, {
    chapters: () => TWO_CHAPTERS,
    prompt: () => prompt("Chapter 2"),
    other: (call) =>
      call.method === "POST" && /^\/workfile\/\d+\/bookmarks$/.test(call.path)
        ? ok({ id: 50 })
        : undefined,
    ...setup,
  });
  if (path) await page.goto(path);
  return player;
}

async function expectSavedPlaceError(page: Page) {
  await expect(page.getByText(LOAD_ERROR)).toBeVisible({ timeout: 10_000 });
  await expect(page.getByText(DETAIL, { exact: true })).toBeVisible();
  await expect(retry(page)).toBeVisible();
  await expect(fromStart(page)).toBeVisible();
  await expectNoControls(page);
}

async function expectWaiting(page: Page) {
  await expect(page.getByText("Loading...", { exact: true })).toBeVisible();
  await expect(page.locator('button[title="Back"]')).toBeVisible();
  await expectNoControls(page);
}

/** The player is open: its play button and seek bar show, and no error. */
async function expectOpen(page: Page) {
  await expect(playButton(page)).toBeVisible({ timeout: 10_000 });
  await expect(controls(page)["seek bar"]).toBeVisible();
  await expect(page.getByText(LOAD_ERROR)).toHaveCount(0);
}

/** Once item `id`'s metadata has loaded, the element's time is `time`. */
async function expectOpensAt(page: Page, id: number, time: number) {
  await expectOpen(page);
  await waitForItemMetadata(page, id);
  await expect
    .poll(async () => (await media(page)).time, {
      message: `the element's time is ${time}`,
    })
    .toBeCloseTo(time, 0);
}

async function blurAll(page: Page) {
  await page.evaluate(() =>
    (document.activeElement as HTMLElement | null)?.blur(),
  );
}

/** Space, ArrowLeft and ArrowRight with focus on the page body. */
async function pressPlayerKeys(page: Page) {
  await blurAll(page);
  for (const key of [" ", "ArrowLeft", "ArrowRight"]) {
    await page.keyboard.press(key);
    await page.waitForTimeout(300);
  }
}

/**
 * The element is paused at time 0, or there is no element. Waits briefly
 * for any metadata first, so a key that would start or move playback could
 * have done so.
 */
async function expectStillAtZero(page: Page) {
  await page.waitForTimeout(1_000);
  const state = await page.evaluate(() => {
    const audio = document.querySelector("audio");
    return audio
      ? { present: true, paused: audio.paused, time: audio.currentTime }
      : { present: false };
  });
  if (state.present)
    expect(state, "the element stays paused at 0").toEqual({
      present: true,
      paused: true,
      time: 0,
    });
}

/**
 * From a paused element and a button showing play, one press of the play
 * button starts playback.
 */
async function startPlaying(page: Page) {
  await expect
    .poll(() => buttonShows(page), {
      message: "the button shows play before the press",
    })
    .toBe("play");
  expect((await media(page)).paused, "the element is paused before the press").toBe(
    true,
  );
  await playButton(page).click();
  await expectPlaying(page);
}

/** Pauses with the play button, and checks the element paused. */
async function pauseWithButton(page: Page) {
  await playButton(page).click();
  await expect
    .poll(async () => (await media(page)).paused, {
      message: "the element pauses",
    })
    .toBe(true);
}

/** Plays for `seconds` of real time and checks the element's time moved by most of it. */
async function listen(page: Page, seconds: number) {
  const before = (await media(page)).time;
  await page.waitForTimeout(seconds * 1_000);
  const after = await media(page);
  expect(after.paused, "still playing").toBe(false);
  expect(after.time - before, `played about ${seconds} s`).toBeGreaterThan(
    seconds - 2,
  );
}

/**
 * One press starts playback (see `startPlaying`), and the save sent while it
 * plays reaches item `id`: the first save to that item after the press is
 * sent about ten seconds later, while the element still plays, and carries
 * the time playback has advanced to. Saves recorded before the press do not
 * count.
 */
async function playAndExpectPeriodicSave(
  page: Page,
  calls: ApiCall[],
  id: number,
) {
  const from = calls.length;
  const startTime = (await media(page)).time;
  const pressedAt = Date.now();
  await startPlaying(page);
  await expect
    .poll(() => saves(calls.slice(from), id).length, {
      message: `a save to /workfile/${id}/progress while playing`,
      timeout: 12_000,
    })
    .toBeGreaterThan(0);
  const save = saves(calls.slice(from), id)[0]!;
  expect(
    (await media(page)).paused,
    "the element still plays after the save",
  ).toBe(false);
  expect(
    save.at - pressedAt,
    "the save comes from the listening interval, not at the press",
  ).toBeGreaterThanOrEqual(8_000);
  expect(
    Number((save.body as { position: string }).position),
    "the save carries the time playback advanced to",
  ).toBeGreaterThan(startTime + 5);
}

function describeSaves(calls: ApiCall[]) {
  return saves(calls).map((c) => `${c.path} ${JSON.stringify(c.body)}`);
}

const failedReads: Array<[string, Reply]> = [
  ["a 500", serverError("progress read failed")],
  ["a network failure", "network-error"],
  ["a 200 with no body", emptyOk],
  ['the position "abc"', savedAt("abc")],
  ["a 403", clientError(403, "forbidden")],
  ["a 200 with null", ok(null)],
  ["a 200 with {}", ok({})],
  ['a 200 with { "position": 60 }', ok({ position: 60 })],
  ['a 200 with { "position": null }', ok({ position: null })],
  ['the position "12abc"', savedAt("12abc")],
  ['the position "-5"', savedAt("-5")],
  ['the position "Infinity"', savedAt("Infinity")],
  ['the position "NaN"', savedAt("NaN")],
];

/** Opens item 1 with a failed read, and chooses "Play from the beginning". */
async function playFromStart(page: Page, setup: PlayerSetup = {}) {
  const player = await openPlayer(page, {
    progress: () => serverError("progress read failed"),
    ...setup,
  });
  await expectSavedPlaceError(page);
  await waitForItemMetadata(page, 1).catch(() => undefined);
  await fromStart(page).click();
  await expectOpen(page);
  return player;
}

test.describe("Opening an audiobook whose saved place cannot be read", () => {
  // AC-034 (500, network failure, no body, "abc") and AC-055 (the rest).
  for (const [label, failure] of failedReads) {
    test(`${label}: the error screen, no controls, and no save or prompt request`, async ({
      page,
    }) => {
      const { calls } = await openPlayer(page, { progress: () => failure });
      await expect
        .poll(() => progressReads(calls, 1).length)
        .toBeGreaterThan(0);
      await expectSavedPlaceError(page);
      await page.waitForTimeout(12_000);
      expect(describeSaves(calls), "no save").toEqual([]);
      expect(
        promptRequests(calls).map((c) => c.path),
        "no prompt request",
      ).toEqual([]);
      await expectSavedPlaceError(page);
    });
  }

  // AC-035 (from a 500) and AC-055 (from {}).
  for (const [label, failure] of [
    ["a 500", serverError("progress read failed")],
    ["a 200 with {}", ok({})],
  ] as Array<[string, Reply]>) {
    test(`Retry after ${label} opens at the saved place, asks for the prompt there, and saves as usual`, async ({
      page,
    }) => {
      let progress: Reply = failure;
      const { calls } = await openPlayer(page, { progress: () => progress });
      await expectSavedPlaceError(page);
      progress = savedAt("60");
      await retry(page).click();
      await expectOpensAt(page, 1, 60);
      await page.waitForTimeout(1_000);
      expect(progressReads(calls, 1), "one read per attempt").toHaveLength(2);
      await expect
        .poll(() => promptRequests(calls, 1).map((c) => c.path))
        .toEqual(["/workfile/1/cross-format/prompt?current_ts=60"]);
      await playAndExpectPeriodicSave(page, calls, 1);
    });
  }

  // AC-036.
  test("a held Retry shows Loading..., then fails back to the error screen", async ({
    page,
  }) => {
    const { calls } = await openPlayer(page, {
      progress: () => serverError("progress read failed"),
    });
    await expectSavedPlaceError(page);
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await retry(page).click();
    await waitForHeld(held, 1, "the second saved-place read");
    await expectWaiting(page);
    await page.waitForTimeout(2_000);
    await expectWaiting(page);
    await held[0]!.release(serverError("progress read failed"));
    await expectSavedPlaceError(page);
    await page.waitForTimeout(3_000);
    expect(describeSaves(calls), "no save").toEqual([]);
  });

  // AC-057.
  test("on the error screen and during a held Retry, the keys neither play nor move, and nothing is sent", async ({
    page,
  }) => {
    const { calls } = await openPlayer(page, {
      progress: () => serverError("progress read failed"),
    });
    await expect.poll(() => progressReads(calls, 1).length).toBe(1);
    await waitForItemMetadata(page, 1, 5_000).catch(() => undefined);
    await pressPlayerKeys(page);
    await expectStillAtZero(page);
    await page.waitForTimeout(2_000);
    expect(describeSaves(calls), "no save").toEqual([]);
    expect(
      promptRequests(calls).map((c) => c.path),
      "no prompt request",
    ).toEqual([]);

    await expectSavedPlaceError(page);
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await retry(page).click();
    await waitForHeld(held, 1, "the second saved-place read");
    await pressPlayerKeys(page);
    await expectStillAtZero(page);
    await page.waitForTimeout(11_000);
    await expectWaiting(page);
    expect(describeSaves(calls), "no save").toEqual([]);
    expect(
      promptRequests(calls).map((c) => c.path),
      "no prompt request",
    ).toEqual([]);
  });

  // AC-043.
  test(`"${FROM_START}" while the stream answers 404: the button stays on play, one toast, and the note`, async ({
    page,
  }) => {
    const { streams } = await openPlayer(page, {
      progress: () => serverError("progress read failed"),
      stream: () => notFound("no such file"),
    });
    await expectSavedPlaceError(page);
    await expect
      .poll(() => new Set(streams.map((s) => s.token)).size)
      .toBeGreaterThanOrEqual(1);
    await page.waitForTimeout(2_000);
    const mark = await toastMark(page);
    await fromStart(page).click();
    await expect.poll(() => buttonShows(page), { timeout: 2_000 }).toBe("play");
    await expectOnlyToast(page, PLAY_FAILED, "error", mark);
    await expect(note(page)).toBeVisible();
  });
});

test.describe(`A "${FROM_START}" session`, () => {
  // AC-037 (from a 500) and AC-055 (from "12abc").
  for (const [label, failure] of [
    ["a 500", serverError("progress read failed")],
    ['the position "12abc"', savedAt("12abc")],
  ] as Array<[string, Reply]>) {
    test(`after ${label}: it plays from 0 with the note, and no door saves`, async ({
      page,
    }) => {
      const { calls } = await playFromStart(page, { progress: () => failure });
      await expectPlaying(page);
      expect((await media(page)).time, "playing from 0").toBeLessThan(3);
      await expect.poll(() => buttonShows(page)).toBe("pause");
      await expect(note(page)).toBeVisible();
      await expect
        .poll(() => promptRequests(calls, 1).length)
        .toBeGreaterThan(0);
      expect(promptRequests(calls, 1).map((c) => c.path)).toEqual(
        promptRequests(calls, 1).map(
          () => "/workfile/1/cross-format/prompt?current_ts=0",
        ),
      );

      await listen(page, 12);
      await pauseWithButton(page);
      await page.waitForTimeout(3_000);
      expect(describeSaves(calls), "no save after playing and pausing").toEqual(
        [],
      );
      if (label !== "a 500") return;

      await blurAll(page);
      await page.keyboard.press(" ");
      await expectPlaying(page);
      await page.keyboard.press(" ");
      await expect.poll(async () => (await media(page)).paused).toBe(true);
      await page.waitForTimeout(3_000);
      expect(describeSaves(calls), "no save after Space").toEqual([]);

      const beforeSeek = await media(page);
      expect(beforeSeek.paused, "paused before the seek bar moves").toBe(true);
      expect(
        Math.abs(beforeSeek.time - 50),
        "the seek bar moves the element somewhere new",
      ).toBeGreaterThan(5);
      await controls(page)["seek bar"].fill("50");
      await expect
        .poll(async () => (await media(page)).time, {
          message: "the seek bar moves the element to 50 s",
        })
        .toBeCloseTo(50, 0);
      await page.waitForTimeout(3_000);
      expect(await media(page), "the element stays paused at 50 s").toMatchObject(
        { paused: true, time: expect.closeTo(50, 0) },
      );
      expect(describeSaves(calls), "no save after the seek bar").toEqual([]);

      await controls(page)["next chapter"].click();
      await expect
        .poll(async () => (await media(page)).time)
        .toBeGreaterThanOrEqual(200);
      await controls(page)["chapter list"].click();
      await page
        .locator("div.fixed button")
        .filter({ hasText: "1. Opening" })
        .click();
      await expect
        .poll(async () => (await media(page)).time, {
          message: "the chapter-list entry returns to the first chapter's start",
        })
        .toBeLessThan(3);
      await expectPlaying(page);
      expect(
        (await media(page)).time,
        "playing inside the first chapter",
      ).toBeLessThan(10);
      await pauseWithButton(page);
      await page.waitForTimeout(3_000);
      expect(describeSaves(calls), "no save at any point").toEqual([]);
      await expect(note(page)).toBeVisible();
    });
  }

  // AC-038.
  test("Add bookmark and the sleep timer's bookmark save, and no place is saved", async ({
    page,
  }) => {
    const { calls } = await playFromStart(page);
    const bookmarkPosts = () =>
      calls.filter(
        (c) => c.method === "POST" && c.path === "/workfile/1/bookmarks",
      ).length;
    await controls(page)["add bookmark"].click();
    await expect.poll(bookmarkPosts).toBe(1);
    const mark = await toastMark(page);
    await controls(page)["sleep timer"].click();
    await page.getByRole("button", { name: "5 minutes", exact: true }).click();
    await expect.poll(bookmarkPosts).toBe(2);
    await expectOnlyToast(page, "Bookmark saved", null, mark);
    await page.waitForTimeout(3_000);
    expect(describeSaves(calls), "no save").toEqual([]);
  });

  // AC-039.
  test("the resume banner's Jump moves the player and saves nothing", async ({
    page,
  }) => {
    const { calls } = await playFromStart(page, {
      prompt: () => prompt("Chapter 2"),
    });
    await page.getByRole("button", { name: "Jump" }).click();
    await expect
      .poll(async () => (await media(page)).time, {
        message: "the element moves to 12.5",
      })
      .toBeGreaterThanOrEqual(12.5);
    expect((await media(page)).time).toBeLessThan(14.5);
    await page.waitForTimeout(3_000);
    expect(describeSaves(calls), "no save").toEqual([]);
  });

  // AC-058.
  test("the sleep timer's chapter-end stop pauses and saves nothing", async ({
    page,
  }) => {
    const { calls } = await playFromStart(page, {
      chapters: () => [
        {
          id: 11,
          chapterIndex: 0,
          title: "Opening",
          startTimeSecs: 0,
          endTimeSecs: 3,
        },
        {
          id: 12,
          chapterIndex: 1,
          title: "Second part",
          startTimeSecs: 3,
          endTimeSecs: 400,
        },
      ],
    });
    await expectPlaying(page);
    await controls(page)["sleep timer"].click();
    await page
      .getByRole("button", { name: "End of chapter", exact: true })
      .click();
    expect(
      (await media(page)).time,
      "chosen before the chapter ends",
    ).toBeLessThan(3);
    await expect
      .poll(async () => (await media(page)).paused, {
        message: "the element pauses",
        timeout: 8_000,
      })
      .toBe(true);
    expect(
      (await media(page)).time,
      "paused at the chapter end",
    ).toBeGreaterThanOrEqual(3);
    await page.waitForTimeout(3_000);
    expect(describeSaves(calls), "no save").toEqual([]);
  });

  // AC-059.
  test("the sleep timer's end pauses and saves nothing", async ({ page }) => {
    await page.clock.install();
    const { calls } = await playFromStart(page);
    await expectPlaying(page);
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 500));
    await controls(page)["sleep timer"].click();
    await page.getByRole("button", { name: "5 minutes", exact: true }).click();
    await page.keyboard.press("Escape");
    await page.clock.runFor(4 * 60_000 + 55_000);
    await expectPlaying(page);
    await page.clock.runFor(6_000);
    await expect
      .poll(async () => (await media(page)).paused, {
        message: "the element pauses",
      })
      .toBe(true);
    await expect(controls(page)["sleep timer"]).toHaveText("");
    await page.clock.runFor(3_000);
    await page.waitForTimeout(500);
    expect(describeSaves(calls), "no save").toEqual([]);
  });

  // AC-040.
  test("opening another audiobook in the same page ends it", async ({
    page,
  }) => {
    const { calls } = await playFromStart(page, {
      progress: (id) =>
        id === 1
          ? serverError("progress read failed")
          : notFound("no progress"),
    });
    await expect(note(page)).toBeVisible();
    await openItemInPage(page, 2);
    await expectOpensAt(page, 2, 0);
    await expect(note(page)).toHaveCount(0);
    await playAndExpectPeriodicSave(page, calls, 2);
  });

  test("a reload ends it", async ({ page }) => {
    let progress: Reply = serverError("progress read failed");
    const { calls } = await playFromStart(page, { progress: () => progress });
    await expect(note(page)).toBeVisible();
    progress = savedAt("60");
    await page.reload();
    await expectOpensAt(page, 1, 60);
    await expect(note(page)).toHaveCount(0);
    await playAndExpectPeriodicSave(page, calls, 1);
  });
});

test.describe("Opening an audiobook whose saved place reads slowly or normally", () => {
  // AC-041.
  test("a slow read: Loading..., no controls, keys do nothing and nothing is sent, then it opens there", async ({
    page,
  }) => {
    const { calls } = await openPlayer(
      page,
      { progress: () => savedAt("60") },
      "",
    );
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "the saved-place read");
    await expectWaiting(page);
    await waitForItemMetadata(page, 1, 5_000).catch(() => undefined);
    await pressPlayerKeys(page);
    await expectStillAtZero(page);
    await page.waitForTimeout(10_000);
    await expectWaiting(page);
    await expectStillAtZero(page);
    expect(describeSaves(calls), "no save while the read is held").toEqual([]);
    expect(
      promptRequests(calls).map((c) => c.path),
      "no prompt request while it is held",
    ).toEqual([]);
    await held[0]!.release();
    await expectOpensAt(page, 1, 60);
  });

  // AC-042, AC-044, AC-056.
  for (const [label, reply, check] of [
    ["a 404", notFound("no progress"), "none"],
    ['the position "0"', savedAt("0"), "saves"],
    ['the position ""', savedAt(""), "saves and prompt"],
  ] as Array<[string, Reply, string]>) {
    test(`${label} opens at 0:00 with no message`, async ({ page }) => {
      const { calls } = await openPlayer(page, { progress: () => reply });
      await expectOpensAt(page, 1, 0);
      await expect(page.getByText("0:00").first()).toBeVisible();
      await expect(page.getByText(LOAD_ERROR)).toHaveCount(0);
      await expect(page.getByText(DETAIL)).toHaveCount(0);
      await expectNoToast(page, 0);
      if (check.includes("prompt"))
        expect(promptRequests(calls, 1).map((c) => c.path)).toContain(
          "/workfile/1/cross-format/prompt?current_ts=0",
        );
      if (check.includes("saves")) {
        await playAndExpectPeriodicSave(page, calls, 1);
      }
    });
  }

  // AC-045.
  test("the saved place is read once per open", async ({ page }) => {
    const { calls } = await openPlayer(page, { progress: () => savedAt("60") });
    await expectOpensAt(page, 1, 60);
    await page.waitForTimeout(1_000);
    expect(progressReads(calls, 1), "one read").toHaveLength(1);
  });
});

test.describe("Changing audiobook in the same page", () => {
  // AC-060: the sleep timer started on item 1, its 10-second save and the
  // chapter-end stop carry over to item 2 while item 2's saved place is
  // unknown. Two timings: the timer ends together with item 2's first
  // 10-second save, and the 10-second save comes due first.
  for (const [steppedBeforeChange, after] of [
    [4 * 60_000 + 50_000, 20_000],
    [4 * 60_000 + 40_000, 30_000],
  ] as Array<[number, number]>) {
    test(`carried-over doors send nothing (timer stepped ${steppedBeforeChange / 1000} s before the change)`, async ({
      page,
    }) => {
      await page.clock.install();
      const { calls } = await openPlayer(
        page,
        { progress: (id) => (id === 1 ? savedAt("300") : savedAt("30")) },
        "",
      );
      const { held } = await holdRequests(
        page,
        (url, method) =>
          method === "GET" && url.pathname === "/api/v1/workfile/2/progress",
      );
      await page.goto("/listen/1?workId=1");
      await expectOpensAt(page, 1, 300);
      await page.clock.pauseAt(await page.evaluate(() => Date.now() + 500));
      await startPlaying(page);
      expect(
        (await media(page)).time,
        "item 1 plays from its saved place",
      ).toBeGreaterThanOrEqual(300);
      await controls(page)["sleep timer"].click();
      await page
        .getByRole("button", { name: "5 minutes", exact: true })
        .click();
      await page.keyboard.press("Escape");
      await page.clock.runFor(steppedBeforeChange);

      const from = calls.length;
      await openItemInPage(page, 2);
      await waitForHeld(held, 1, "item 2's saved-place read");
      await waitForItemMetadata(page, 2);
      await page.clock.runFor(after);
      await page.clock.runFor(3_000);
      await page.waitForTimeout(500);
      const sent = saves(calls.slice(from)).map(
        (c) => `${c.path} ${JSON.stringify(c.body)}`,
      );
      expect(sent, "no save to either item after the change").toEqual([]);
      expect(held, "item 2's read is still held").toHaveLength(1);
    });
  }

  // A chapter-end save queued on item 1 just before the change still
  // carries item 1's own place, never item 2's time.
  test("a save queued on item 1 just before the change carries item 1's own place", async ({
    page,
  }) => {
    await page.clock.install();
    const { calls } = await openPlayer(
      page,
      {
        progress: (id) => (id === 1 ? savedAt("300") : savedAt("30")),
        chapters: (id) =>
          id === 1
            ? [
                {
                  id: 11,
                  chapterIndex: 0,
                  title: "Opening",
                  startTimeSecs: 0,
                  endTimeSecs: 303,
                },
                {
                  id: 12,
                  chapterIndex: 1,
                  title: "Second part",
                  startTimeSecs: 303,
                  endTimeSecs: 400,
                },
              ]
            : TWO_CHAPTERS,
      },
      "",
    );
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/2/progress",
    );
    await page.goto("/listen/1?workId=1");
    await expectOpensAt(page, 1, 300);
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 500));
    await startPlaying(page);
    await controls(page)["sleep timer"].click();
    await page
      .getByRole("button", { name: "End of chapter", exact: true })
      .click();
    await page.keyboard.press("Escape");
    await expect
      .poll(async () => (await media(page)).paused, {
        message: "item 1 pauses at its chapter end",
        timeout: 8_000,
      })
      .toBe(true);
    const stoppedAt = (await media(page)).time;
    expect(stoppedAt).toBeGreaterThanOrEqual(303);

    const from = calls.length;
    await openItemInPage(page, 2);
    await waitForHeld(held, 1, "item 2's saved-place read");
    await waitForItemMetadata(page, 2);
    await page.clock.runFor(3_000);
    await page.waitForTimeout(500);
    const after = saves(calls.slice(from));
    expect(
      saves(after, 2).map((c) => c.body),
      "no save to item 2",
    ).toEqual([]);
    for (const save of after)
      expect(
        Number((save.body as { position: string }).position),
        "item 1's own place",
      ).toBeGreaterThanOrEqual(300);
  });

  // AC-061.
  test(`a late success for the previous audiobook changes nothing in a "${FROM_START}" session`, async ({
    page,
  }) => {
    const { calls } = await openPlayer(
      page,
      {
        progress: (id) =>
          id === 1 ? savedAt("60") : serverError("progress read failed"),
        prompt: (id) => prompt(id === 2 ? "Item two place" : "Item one place"),
      },
      "",
    );
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "item 1's saved-place read");
    const from = calls.length;
    await openItemInPage(page, 2);
    await expectSavedPlaceError(page);
    await fromStart(page).click();
    await expectPlaying(page);
    await expect(page.getByText("Item two place")).toBeVisible();
    expect(promptRequests(calls, 2).map((c) => c.path)).toEqual([
      "/workfile/2/cross-format/prompt?current_ts=0",
    ]);

    await held[0]!.release();
    await page.waitForTimeout(1_500);
    expect(
      (await media(page)).time,
      "not moved to item 1's place",
    ).toBeLessThan(30);
    await expect(note(page)).toBeVisible();
    await expect(page.getByText("Item two place")).toBeVisible();
    await expect(page.getByText("Item one place")).toHaveCount(0);
    expect(
      promptRequests(calls.slice(from), 1),
      "no prompt request for item 1",
    ).toEqual([]);

    await listen(page, 12);
    await pauseWithButton(page);
    await page.waitForTimeout(3_000);
    expect(describeSaves(calls), "no save to either item").toEqual([]);
    await expect(page.getByText("Item two place")).toBeVisible();
  });

  // AC-062.
  test("a late failure for the previous audiobook changes nothing once the new one has opened", async ({
    page,
  }) => {
    const { calls } = await openPlayer(
      page,
      {
        progress: (id) =>
          id === 1 ? serverError("progress read failed") : savedAt("30"),
      },
      "",
    );
    const { held } = await holdRequests(
      page,
      (url, method) =>
        method === "GET" && url.pathname === "/api/v1/workfile/1/progress",
    );
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "item 1's saved-place read");
    await openItemInPage(page, 2);
    await expectOpensAt(page, 2, 30);
    await held[0]!.release();
    await page.waitForTimeout(2_000);
    await expect(page.getByText(LOAD_ERROR)).toHaveCount(0);
    await expectOpen(page);
    await playAndExpectPeriodicSave(page, calls, 2);
  });
});

test.describe("The saved place when the first stream is not the one that loads", () => {
  // F1, code review r1: the saved place is known before any stream metadata,
  // and the first usable metadata comes from a stream the player re-mints or
  // refreshes. That metadata must still start the player at the saved place.
  const firstStreamHeld = (url: URL, method: string) =>
    method === "GET" &&
    url.pathname === "/api/v1/stream/1" &&
    url.searchParams.get("token") === "token-1-1";

  test("a stream re-minted after the first one fails starts at the saved place, and saves from there", async ({
    page,
  }) => {
    const { calls, streams } = await openPlayer(
      page,
      {
        progress: () => savedAt("60"),
        stream: (_id, token) =>
          token === "token-1-1" ? notFound("no such file") : "wav",
      },
      "",
    );
    const { held } = await holdRequests(page, firstStreamHeld);
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "the first stream request");
    await expectOpen(page);
    expect(progressReads(calls, 1), "the saved place is read").toHaveLength(1);
    await held[0]!.release(notFound("no such file"));
    await expect
      .poll(() => streams.some((s) => s.token === "token-1-2"), {
        message: "the stream is re-minted after the first one fails",
      })
      .toBe(true);
    await expectOpensAt(page, 1, 60);
    await page.waitForTimeout(1_000);
    expect(progressReads(calls, 1), "one saved-place read").toHaveLength(1);
    await playAndExpectPeriodicSave(page, calls, 1);
  });

  test("a stream refreshed before the first one loads starts at the saved place, and saves from there", async ({
    page,
  }) => {
    const { calls, streams } = await openPlayer(
      page,
      {
        progress: () => savedAt("60"),
        token: (id, n) =>
          ok({
            token: `token-${id}-${n}`,
            exp: Math.floor(Date.now() / 1000) + (n === 1 ? 303 : 86_400),
          }),
      },
      "",
    );
    const { held } = await holdRequests(page, firstStreamHeld);
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "the first stream request");
    await expectOpen(page);
    expect(progressReads(calls, 1), "the saved place is read").toHaveLength(1);
    await expect
      .poll(() => streams.some((s) => s.token === "token-1-2"), {
        message: "the refreshed address is requested",
        timeout: 15_000,
      })
      .toBe(true);
    await expectOpensAt(page, 1, 60);
    await page.waitForTimeout(1_000);
    expect(progressReads(calls, 1), "one saved-place read").toHaveLength(1);
    await playAndExpectPeriodicSave(page, calls, 1);
  });
});

test.describe("The saved place when a refresh started before it was applied", () => {
  // F1, code review r2: a refresh whose position snapshot was taken before
  // the saved place reached the element, and whose address arrives after it
  // did, must not put the element back to that snapshot's 0.
  test("a refresh asked for before the saved place was applied keeps the saved place, and saves from there", async ({
    page,
  }) => {
    let mints = 0;
    const { calls, streams } = await openPlayer(
      page,
      {
        progress: () => savedAt("60"),
        token: (id, n) =>
          ok({
            token: `token-${id}-${n}`,
            exp: Math.floor(Date.now() / 1000) + (n === 1 ? 303 : 86_400),
          }),
      },
      "",
    );
    const firstStream = await holdRequests(
      page,
      (url, method) =>
        method === "GET" &&
        url.pathname === "/api/v1/stream/1" &&
        url.searchParams.get("token") === "token-1-1",
    );
    const refreshMint = await holdRequests(page, (url, method) => {
      if (method !== "POST" || url.pathname !== "/api/v1/workfile/1/stream-token")
        return false;
      mints += 1;
      return mints === 2;
    });
    await page.goto("/listen/1?workId=1");
    await waitForHeld(firstStream.held, 1, "the first stream request");
    await expectOpen(page);
    await waitForHeld(refreshMint.held, 1, "the refresh's token request");

    await firstStream.held[0]!.release();
    await expectOpensAt(page, 1, 60);

    await refreshMint.held[0]!.release();
    await expect
      .poll(
        () =>
          page.evaluate(() => {
            const audio = document.querySelector("audio");
            return !!audio &&
              audio.currentSrc.includes("token=token-1-2") &&
              audio.readyState >= 1
              ? "loaded"
              : "waiting";
          }),
        { message: "the refreshed stream's metadata loads", timeout: 15_000 },
      )
      .toBe("loaded");
    expect(streams.some((s) => s.token === "token-1-2")).toBe(true);
    await page.waitForTimeout(500);
    expect((await media(page)).time, "the element stays at the saved place").toBeCloseTo(60, 0);
    expect(progressReads(calls, 1), "one saved-place read").toHaveLength(1);
    await playAndExpectPeriodicSave(page, calls, 1);
  });
});
