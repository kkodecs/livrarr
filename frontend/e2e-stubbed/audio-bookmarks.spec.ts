import { expect, test, type Page } from "@playwright/test";
import {
  media,
  ONE_CHAPTER,
  stubPlayer,
  waitForItemMetadata,
  type PlayerSetup,
} from "./audioPlayer";
import {
  clientError,
  expectOnlyToast,
  holdRequests,
  liveToasts,
  notFound,
  ok,
  serverError,
  toastMark,
  toastSnapshot,
  waitForHeld,
  type ApiCall,
  type Reply,
} from "./stubbedApi";

// Bookmark actions in the audio player whose request fails each show one
// error toast with the EPUB reader's wording; the sleep timer's bookmark
// says "Bookmark saved" only once its create has succeeded.

const ADD = "Could not add the bookmark";
const RENAME = "Could not rename the bookmark";
const DELETE = "Could not delete the bookmark";
const SAVED = "Bookmark saved";

const bookmark = {
  id: 7,
  libraryItemId: 1,
  mediaType: "audiobook",
  position: "30",
  sortKey: 30,
  name: "Half a minute",
  chapterTitle: null,
  pairedBookmarkId: null,
  createdAt: "2026-09-01T00:00:00Z",
};

type Write = (call: ApiCall) => Reply | undefined;

const onCreate =
  (reply: Reply): Write =>
  (call) =>
    call.method === "POST" && call.path === "/workfile/1/bookmarks"
      ? reply
      : undefined;
const onRename =
  (reply: Reply): Write =>
  (call) =>
    call.method === "PATCH" && call.path === "/bookmarks/7" ? reply : undefined;
const onDelete =
  (reply: Reply): Write =>
  (call) =>
    call.method === "DELETE" && call.path === "/bookmarks/7"
      ? reply
      : undefined;

function sent(calls: ApiCall[], method: string, path: string) {
  return calls.filter((c) => c.method === method && c.path === path).length;
}

async function openPlayer(page: Page, write: Write, setup: PlayerSetup = {}) {
  const { calls } = await stubPlayer(page, {
    bookmarks: () => [bookmark],
    other: write,
    ...setup,
  });
  await page.goto("/listen/1?workId=1");
  await waitForItemMetadata(page, 1);
  await expect(page.getByText("1 bookmarks")).toBeVisible();
  await expect(liveToasts(page)).toHaveCount(0);
  return calls;
}

async function startRename(page: Page) {
  await page.locator('button[title="Bookmarks"]').click();
  await page.locator('button[title="Rename"]').click();
  const input = page.locator("form input");
  await input.fill("Chapter two");
  return input;
}

function deleteButton(page: Page) {
  return page
    .locator("div.group")
    .filter({ hasText: "Half a minute" })
    .locator("button")
    .last();
}

async function chooseSleep(page: Page, option: "5 minutes" | "End of chapter") {
  await page.locator('button[title="Sleep timer"]').click();
  await page.getByRole("button", { name: option, exact: true }).click();
}

/** The sleep timer runs: its remaining time, or the chapter-end flag, shows. */
async function expectTimerRunning(
  page: Page,
  option: "5 minutes" | "End of chapter",
) {
  if (option === "5 minutes") {
    await expect(page.locator('button[title="Sleep timer"]')).toHaveText(
      /^\d+:\d\d$/,
    );
  } else {
    await expect(
      page.getByRole("button", { name: "Sleeping at chapter end" }),
    ).toBeVisible();
  }
}

/** Waits for the request, lets a second toast render, then checks the one toast. */
async function expectOneAfter(
  page: Page,
  calls: ApiCall[],
  method: string,
  path: string,
  text: string,
  mark: number,
) {
  await expect.poll(() => sent(calls, method, path)).toBe(1);
  await page.waitForTimeout(500);
  await expectOnlyToast(page, text, "error", mark);
}

const failures: Array<[string, Reply]> = [
  ["a 500", serverError("bookmark write failed")],
  ["a network failure", "network-error"],
  ["a 400", clientError(400, "rejected")],
  ["a 404", notFound("bookmark gone")],
];
const reply = (label: string) => failures.find(([l]) => l === label)![1];

test.describe("A rejected audiobook bookmark action", () => {
  // Add bookmark: a 500 and a network failure (AC-007), a 400 (AC-047).
  for (const label of ["a 500", "a network failure", "a 400"]) {
    test(`adding, answered with ${label}, shows one error toast`, async ({
      page,
    }) => {
      const calls = await openPlayer(page, onCreate(reply(label)));
      const mark = await toastMark(page);
      await page.locator('button[title="Add bookmark"]').click();
      await expectOneAfter(
        page,
        calls,
        "POST",
        "/workfile/1/bookmarks",
        ADD,
        mark,
      );
    });
  }

  // Rename: a 500 (AC-008), a network failure and a 404 (AC-047).
  for (const label of ["a 500", "a network failure", "a 404"]) {
    test(`renaming, answered with ${label}, shows one error toast`, async ({
      page,
    }) => {
      const calls = await openPlayer(page, onRename(reply(label)));
      const input = await startRename(page);
      const mark = await toastMark(page);
      await input.press("Enter");
      await expectOneAfter(page, calls, "PATCH", "/bookmarks/7", RENAME, mark);
    });
  }

  // Delete: a 500 and a 404 (AC-009), a network failure (AC-047).
  for (const label of ["a 500", "a 404", "a network failure"]) {
    test(`deleting, answered with ${label}, shows one error toast`, async ({
      page,
    }) => {
      const calls = await openPlayer(page, onDelete(reply(label)));
      await page.locator('button[title="Bookmarks"]').click();
      const mark = await toastMark(page);
      await deleteButton(page).click();
      await expectOneAfter(page, calls, "DELETE", "/bookmarks/7", DELETE, mark);
    });
  }

  // The sleep timer's bookmark: "5 minutes" with a 500 (AC-010), a network
  // failure and a 400 (AC-047); "End of chapter" with a 500 (AC-010).
  const sleepCases: Array<["5 minutes" | "End of chapter", string]> = [
    ["5 minutes", "a 500"],
    ["5 minutes", "a network failure"],
    ["5 minutes", "a 400"],
    ["End of chapter", "a 500"],
  ];
  for (const [option, label] of sleepCases) {
    test(`"${option}" whose bookmark is answered with ${label} shows the add error and no "${SAVED}"; the timer runs`, async ({
      page,
    }) => {
      const calls = await openPlayer(page, onCreate(reply(label)), {
        chapters: () => ONE_CHAPTER,
      });
      const mark = await toastMark(page);
      await chooseSleep(page, option);
      await expectOneAfter(
        page,
        calls,
        "POST",
        "/workfile/1/bookmarks",
        ADD,
        mark,
      );
      await expectTimerRunning(page, option);
    });
  }

  // AC-048: the reply arrives after the bookmark panel has closed.
  for (const action of ["rename", "delete"] as const) {
    test(`a held ${action} answered with a 500 after the panel closes shows one error toast`, async ({
      page,
    }) => {
      await openPlayer(page, () => undefined);
      const { held } = await holdRequests(
        page,
        (url, method) =>
          url.pathname === "/api/v1/bookmarks/7" &&
          method === (action === "rename" ? "PATCH" : "DELETE"),
      );
      const mark = await toastMark(page);
      if (action === "rename") {
        const input = await startRename(page);
        await input.press("Enter");
      } else {
        await page.locator('button[title="Bookmarks"]').click();
        await deleteButton(page).click();
      }
      await waitForHeld(held, 1, `the ${action}`);
      await page
        .locator("div.fixed")
        .filter({ hasText: "Bookmarks" })
        .locator("button:has(svg.lucide-x)")
        .first()
        .click();
      await expect(
        page.locator("div.fixed").filter({ hasText: "Half a minute" }),
      ).toHaveCount(0);
      await held[0]!.release(serverError(`bookmark ${action} failed`));
      await page.waitForTimeout(500);
      await expectOnlyToast(
        page,
        action === "rename" ? RENAME : DELETE,
        "error",
        mark,
      );
    });
  }
});

test.describe("The sleep timer's bookmark succeeds", () => {
  // AC-011.
  test(`"5 minutes" with the create succeeding shows one "${SAVED}"`, async ({
    page,
  }) => {
    const calls = await openPlayer(page, onCreate(ok(bookmark)));
    const mark = await toastMark(page);
    await chooseSleep(page, "5 minutes");
    await expect
      .poll(() => sent(calls, "POST", "/workfile/1/bookmarks"))
      .toBe(1);
    await page.waitForTimeout(500);
    await expectOnlyToast(page, SAVED, null, mark);
    await expectTimerRunning(page, "5 minutes");
  });

  // AC-012: a bookmark added while the sleep bookmark's create is held does
  // not take over the "Bookmark saved" for it.
  test(`a held sleep bookmark, then Add bookmark: "${SAVED}" shows once, after the sleep create succeeds`, async ({
    page,
  }) => {
    let creates = 0;
    const calls = await openPlayer(page, onCreate(ok(bookmark)));
    const { held } = await holdRequests(page, (url, method) => {
      if (method !== "POST" || url.pathname !== "/api/v1/workfile/1/bookmarks")
        return false;
      creates += 1;
      return creates === 1;
    });
    const mark = await toastMark(page);
    await chooseSleep(page, "5 minutes");
    await waitForHeld(held, 1, "the sleep bookmark's create");
    await page.keyboard.press("Escape");
    await page.locator('button[title="Add bookmark"]').click();
    await expect
      .poll(() => sent(calls, "POST", "/workfile/1/bookmarks"))
      .toBe(1);
    await page.waitForTimeout(1_000);
    const before = await page.evaluate(
      (from) =>
        (window as unknown as { __toastsAdded: Element[] }).__toastsAdded
          .slice(from)
          .map((el) => el.textContent ?? ""),
      mark,
    );
    expect(
      before.filter((text) => text.includes(SAVED)),
      `no "${SAVED}" while the sleep bookmark's create is unanswered`,
    ).toEqual([]);
    await held[0]!.release();
    await page.waitForTimeout(500);
    await expectOnlyToast(page, SAVED, null, mark);
  });
});

/**
 * Runs the paused page clock in small steps until `count` toasts have been
 * added since `since`, or a second of page time has passed, then a little
 * longer so a further toast could render.
 */
async function runUntilAdded(page: Page, since: number, count: number) {
  for (let i = 0; i < 20; i++) {
    if ((await toastMark(page)) - since >= count) break;
    await page.waitForTimeout(100);
    await page.clock.runFor(50);
  }
  await page.waitForTimeout(200);
  await page.clock.runFor(50);
}

/** Advances the page clock in small steps until `done` holds. */
async function advanceUntil(
  page: Page,
  done: () => Promise<boolean>,
  what: string,
  limitMs: number,
  stepMs: number,
) {
  for (let elapsed = 0; elapsed <= limitMs; elapsed += stepMs) {
    if (await done()) return;
    await page.clock.runFor(stepMs);
  }
  throw new Error(`${what} did not happen within ${limitMs} ms of page time`);
}

/** The toast added `index`-th in the page's record, and whether it is still on screen. */
async function recordedToast(page: Page, index: number) {
  return page.evaluate((i) => {
    const el = (window as unknown as { __toastsAdded: Element[] })
      .__toastsAdded[i];
    if (!el) return null;
    return {
      type: el.getAttribute("data-type"),
      text: el.textContent ?? "",
      onScreen: el.isConnected && el.getAttribute("data-removed") !== "true",
    };
  }, index);
}

const leavingToasts = (page: Page) =>
  page.locator('[data-sonner-toast][data-removed="true"]');

/** Moves the player to `seconds` with its seek bar. */
async function seekTo(page: Page, seconds: number) {
  await page.locator('input[type="range"][step="0.1"]').fill(String(seconds));
  await expect
    .poll(async () => (await media(page)).time, {
      message: `the player moves to ${seconds} s`,
    })
    .toBeCloseTo(seconds, 0);
}

/** Chooses a sleep option, checks the timer runs, and closes the menu. */
async function sleepAndClose(
  page: Page,
  option: "5 minutes" | "End of chapter",
) {
  await chooseSleep(page, option);
  await expectTimerRunning(page, option);
  await page.keyboard.press("Escape");
  await expect(
    page.getByRole("button", { name: "5 minutes", exact: true }),
  ).toHaveCount(0);
}

interface Repeated {
  name: string;
  text: string;
  hold: (url: URL, method: string) => boolean;
  setup?: PlayerSetup;
  /** Performs the action for the `n`-th time, from 1. */
  act: (page: Page, n: number) => Promise<void>;
}

const createHeld = (url: URL, method: string) =>
  method === "POST" && url.pathname === "/api/v1/workfile/1/bookmarks";

const repeated: Repeated[] = [
  {
    name: "Add bookmark",
    text: ADD,
    hold: createHeld,
    act: async (page) => {
      await page.locator('button[title="Add bookmark"]').click();
    },
  },
  {
    name: "rename",
    text: RENAME,
    hold: (url, method) =>
      method === "PATCH" && url.pathname === "/api/v1/bookmarks/7",
    act: async (page, n) => {
      if (n === 1) await page.locator('button[title="Bookmarks"]').click();
      await page.locator('button[title="Rename"]').click();
      const input = page.locator("form input");
      await input.fill(`Renamed ${n}`);
      await input.press("Enter");
    },
  },
  {
    name: "delete",
    text: DELETE,
    hold: (url, method) =>
      method === "DELETE" && url.pathname === "/api/v1/bookmarks/7",
    act: async (page, n) => {
      if (n === 1) await page.locator('button[title="Bookmarks"]').click();
      await deleteButton(page).click();
    },
  },
  {
    // Each sleep bookmark is made more than 60 s from the last, so the
    // player's duplicate guard lets it through.
    name: "the sleep timer's bookmark",
    text: ADD,
    hold: createHeld,
    setup: { chapters: () => ONE_CHAPTER },
    act: async (page, n) => {
      if (n > 1) await seekTo(page, n * 100);
      await sleepAndClose(page, n === 3 ? "End of chapter" : "5 minutes");
    },
  },
];

test.describe("Repeated bookmark failures in one page", () => {
  // REQ-002: no fixed id, so each failure shows its own toast (AC-007 to
  // AC-010). The page clock is paused so toasts leave only when it is run.
  for (const r of repeated) {
    test(`${r.name} failing three times: each failure adds its own toast, including one answered while an earlier toast leaves`, async ({
      page,
    }) => {
      await page.clock.install();
      await openPlayer(page, () => undefined, r.setup);
      const { held } = await holdRequests(page, r.hold);
      await page.clock.pauseAt(await page.evaluate(() => Date.now() + 1_000));
      const mark = await toastMark(page);
      const failure = serverError(`${r.name} failed`);
      const error = { type: "error", text: expect.stringContaining(r.text) };

      // A first failure.
      await r.act(page, 1);
      await waitForHeld(held, 1, `the first ${r.name}`);
      await held[0]!.release(failure);
      await runUntilAdded(page, mark, 1);
      let seen = await toastSnapshot(page, mark);
      expect(seen.added, "toasts added by the first failure").toEqual([error]);
      expect(seen.live, "toasts on screen after the first failure").toEqual([
        error,
      ]);

      // A second failure while the first toast is on screen.
      await r.act(page, 2);
      await waitForHeld(held, 2, `the second ${r.name}`);
      await held[1]!.release(failure);
      await runUntilAdded(page, mark, 2);
      seen = await toastSnapshot(page, mark);
      expect(seen.added, "toasts added by two failures").toEqual([error, error]);
      expect(seen.live, "both toasts on screen").toEqual([error, error]);

      // A third failure, answered while an earlier toast is leaving.
      await r.act(page, 3);
      await waitForHeld(held, 3, `the third ${r.name}`);
      await page.mouse.move(0, 0);
      await advanceUntil(
        page,
        async () => (await leavingToasts(page).count()) > 0,
        "an earlier toast starting to leave",
        10_000,
        50,
      );
      await held[2]!.release(failure);
      await runUntilAdded(page, mark, 3);
      await page.clock.runFor(300);
      seen = await toastSnapshot(page, mark);
      expect(seen.added, "toasts added by three failures").toEqual([
        error,
        error,
        error,
      ]);
      expect(
        await recordedToast(page, mark + 2),
        "the third failure's toast outlasts the one that was leaving",
      ).toEqual({ ...error, onScreen: true });
    });
  }
});
