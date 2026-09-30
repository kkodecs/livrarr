import { expect, test, type Page, type Route } from "@playwright/test";
import {
  expectNoToast,
  expectOnlyToast,
  liveToasts,
  ok,
  silentWav,
  stubApi,
  toastMark,
} from "./stubbedApi";

// The audio player's ten-second save and its seek-bar save can finish close
// together. Here a success clears the warning and the next save fails while
// that warning is still leaving the screen. The page clock is paused, so the
// leaving warning stays mid-exit until the test advances time, and each save
// is held until the test answers it.

const SAVE_MESSAGE = "Could not save your place.";
const PROGRESS_PATH = /^\/api\/v1\/workfile\/\d+\/progress$/;
const WAV = silentWav(120);

type Answer = "ok" | "network-error";

interface HeldSave {
  path: string;
  body: Record<string, unknown>;
  /** Answers the request and resolves once the browser has the answer. */
  answer: (reply: Answer) => Promise<void>;
}

/**
 * Holds every position save until the test answers it. Registered after
 * `stubApi`, so it takes the PUT and leaves every other request to the stub.
 */
async function holdSaves(page: Page): Promise<HeldSave[]> {
  const held: HeldSave[] = [];
  await page.route(
    (url) => PROGRESS_PATH.test(url.pathname),
    async (route: Route) => {
      const request = route.request();
      if (request.method() !== "PUT") {
        await route.fallback();
        return;
      }
      let answered!: () => void;
      const done = new Promise<void>((resolve) => {
        answered = resolve;
      });
      let reply!: (value: Answer) => void;
      const replied = new Promise<Answer>((resolve) => {
        reply = resolve;
      });
      held.push({
        path: new URL(request.url()).pathname.replace(/^\/api\/v1/, ""),
        body: request.postDataJSON() as Record<string, unknown>,
        answer: async (value) => {
          reply(value);
          await done;
        },
      });
      if ((await replied) === "network-error") {
        await route.abort("failed");
      } else {
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ success: true }),
        });
      }
      answered();
    },
  );
  return held;
}

/**
 * Counts position-save fetches that have settled. The count is bumped in the
 * first reaction to the fetch's promise, so by the time a later task reads it
 * the rejection has run through the page's own handlers.
 */
async function countSettledSaves(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __savesSettled: number };
    w.__savesSettled = 0;
    const original = window.fetch.bind(window);
    window.fetch = (input, init) => {
      const pending = original(input, init);
      if (init?.method === "PUT" && /\/progress$/.test(String(input))) {
        const settled = () => {
          w.__savesSettled += 1;
        };
        pending.then(settled, settled);
      }
      return pending;
    };
  });
}

async function settledSaves(page: Page): Promise<number> {
  return page.evaluate(
    () => (window as unknown as { __savesSettled: number }).__savesSettled,
  );
}

/** Answers a held save with a network failure and waits until the page has handled it. */
async function failSave(page: Page, save: HeldSave) {
  const before = await settledSaves(page);
  await save.answer("network-error");
  await expect.poll(() => settledSaves(page)).toBe(before + 1);
}

async function waitForHeld(held: HeldSave[], count: number) {
  await expect
    .poll(() => held.length, { message: `save ${count} is sent` })
    .toBe(count);
}

/** Advances the page clock in frame-sized steps until `done` holds. */
async function advanceUntil(
  page: Page,
  done: () => Promise<boolean>,
  what: string,
) {
  for (let elapsed = 0; elapsed <= 1_000; elapsed += 16) {
    if (await done()) return;
    await page.clock.runFor(16);
  }
  throw new Error(`${what} did not happen within one second of page time`);
}

const leavingToasts = (page: Page) =>
  page.locator('[data-sonner-toast][data-removed="true"]');

test("a save failure right after a success still warns, while the old warning is leaving", async ({
  page,
}) => {
  await countSettledSaves(page);
  await stubApi(page, (call) => {
    const bare = call.path.split("?")[0]!;
    if (call.method === "POST" && /^\/workfile\/\d+\/stream-token$/.test(bare))
      return ok({
        token: "stream-token",
        exp: Math.floor(Date.now() / 1000) + 86_400,
      });
    if (call.method !== "GET") return undefined;
    if (bare === "/work/1")
      return ok({ id: 1, title: "Sample Audiobook", authorName: "Sample Author" });
    if (/^\/stream\/\d+$/.test(bare))
      return { status: 200, bytes: WAV, contentType: "audio/wav" };
    if (/^\/workfile\/\d+\/chapters$/.test(bare)) return ok([]);
    if (/^\/workfile\/\d+\/bookmarks$/.test(bare)) return ok([]);
    if (/^\/workfile\/\d+\/progress$/.test(bare))
      return {
        status: 404,
        json: { status: 404, error: "not_found", message: "no progress" },
      };
    if (/^\/workfile\/\d+\/cross-format\/prompt$/.test(bare)) return ok(null);
    return undefined;
  });
  const held = await holdSaves(page);

  await page.clock.install();
  await page.goto("/listen/1?workId=1");
  await expect
    .poll(() =>
      page.evaluate(() => document.querySelector("audio")?.duration ?? NaN),
    )
    .toBeGreaterThan(100);

  // From here page time moves only when the test advances it.
  const pageNow = await page.evaluate(() => Date.now());
  await page.clock.pauseAt(pageNow + 1_000);
  await page.locator("button:has(svg.lucide-play)").click();
  await expect
    .poll(() => page.evaluate(() => document.querySelector("audio")?.currentTime ?? 0))
    .toBeGreaterThan(0.5);
  const seekBar = page.locator('input[type="range"][step="0.1"]');

  // Failure 1: the ten-second save while playing (page time 10 s).
  await page.clock.runFor(10_000);
  await waitForHeld(held, 1);
  await failSave(page, held[0]!);
  await page.clock.runFor(50);
  await expectOnlyToast(page, SAVE_MESSAGE, "error", 0);

  // Failure 2: moving the seek bar; its save goes two seconds later (12 s).
  let mark = await toastMark(page);
  await seekBar.fill("20");
  await page.clock.runFor(2_000);
  await waitForHeld(held, 2);
  await failSave(page, held[1]!);
  await page.clock.runFor(50);
  await expectOnlyToast(page, SAVE_MESSAGE, "error", mark);
  expect(await toastMark(page), "no second warning").toBe(mark);

  // Two saves in flight at once: a seek save (14 s) and the next
  // ten-second save (20 s).
  await seekBar.fill("40");
  await page.clock.runFor(2_000);
  await waitForHeld(held, 3);
  await page.clock.runFor(5_900);
  await waitForHeld(held, 4);

  // The seek save succeeds, and the warning starts to leave.
  await held[2]!.answer("ok");
  await advanceUntil(
    page,
    async () => (await leavingToasts(page).count()) === 1,
    "the warning starting to leave",
  );
  await expect(liveToasts(page)).toHaveCount(0);

  // The ten-second save fails while that warning is still leaving.
  mark = await toastMark(page);
  await failSave(page, held[3]!);

  // Once the old warning has gone, exactly one warning is on screen.
  await page.clock.runFor(1_000);
  await expectOnlyToast(page, SAVE_MESSAGE, "error", mark);

  // It stays through an idle spell that ends before the next ten-second save.
  await page.clock.runFor(8_000);
  await expectOnlyToast(page, SAVE_MESSAGE, "error", mark);
  expect(held, "no save while idle").toHaveLength(4);

  // Another failure (30 s) keeps that one warning and adds none.
  await page.clock.runFor(1_000);
  await waitForHeld(held, 5);
  mark = await toastMark(page);
  await failSave(page, held[4]!);
  await page.clock.runFor(50);
  await expectOnlyToast(page, SAVE_MESSAGE, "error", mark);
  expect(await toastMark(page), "no second warning").toBe(mark);

  // A later success clears it.
  mark = await toastMark(page);
  await seekBar.fill("60");
  await page.clock.runFor(2_000);
  await waitForHeld(held, 6);
  await held[5]!.answer("ok");
  await advanceUntil(
    page,
    async () => (await liveToasts(page).count()) === 0,
    "the warning leaving",
  );
  await page.clock.runFor(500);
  await expectNoToast(page, mark);
  await expect(page.locator("[data-sonner-toast]")).toHaveCount(0);

  // The saves carry what each control sent, and there were no others.
  expect(held).toHaveLength(6);
  const bodies = held.map((s) => s.body);
  for (const save of held) expect(save.path).toBe("/workfile/1/progress");
  for (const i of [1, 2, 5]) {
    const position = ["", "20", "40", "", "", "60"][i]!;
    expect(bodies[i]).toEqual({
      position,
      progress_pct: expect.closeTo(Number(position) / 120, 3),
      kind: "seek",
      cross_format_ts: Number(position),
    });
  }
  // A ten-second save sends the element's own time; with this stubbed
  // stream that time can be back near zero just after a seek.
  for (const i of [0, 3, 4]) {
    const body = bodies[i]!;
    const position = Number(body.position);
    expect(Number.isFinite(position)).toBe(true);
    expect(body).toEqual({
      position: String(position),
      progress_pct: expect.closeTo(position / 120, 3),
      kind: "progress",
      cross_format_ts: position,
    });
  }
});
