import { expect, test, type Page } from "@playwright/test";
import {
  buttonShows,
  expectPlaying,
  media,
  ONE_CHAPTER,
  playButton,
  stubPlayer,
  waitForItemMetadata,
  type StreamAnswer,
} from "./audioPlayer";
import {
  clientError,
  emptyOk,
  expectNoToast,
  expectOnlyToast,
  holdRequests,
  liveToasts,
  notFound,
  ok,
  serverError,
  toastMark,
  waitForHeld,
  type Reply,
} from "./stubbedApi";

// Pressing play on an audiobook the browser cannot play leaves the button on
// play and shows one "Could not play" toast; every door that reports a
// playback failure shares that one toast.

const PLAY_FAILED = "Could not play this audiobook. Try reloading the page.";

const notAudio: Reply = {
  status: 200,
  bytes: Buffer.from("this is not audio ".repeat(200)),
  contentType: "audio/wav",
};

const brokenStreams: Array<[string, StreamAnswer]> = [
  ["answers 404", notFound("no such file")],
  ["answers 500", serverError("stream failed")],
  ["fails at the network", "network-error"],
  ["answers bytes that are not audio", notAudio],
];

/** Records every `play()` call's outcome, without changing it. */
async function trackPlayCalls(page: Page) {
  await page.addInitScript(() => {
    const plays: Array<{ state: string; error: string }> = [];
    (window as unknown as { __plays: typeof plays }).__plays = plays;
    const original = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function play(this: HTMLMediaElement) {
      const record = { state: "pending", error: "" };
      plays.push(record);
      const promise = original.call(this);
      promise.then(
        () => {
          record.state = "resolved";
        },
        (error: unknown) => {
          record.state = "rejected";
          record.error = (error as Error)?.name ?? String(error);
        },
      );
      return promise;
    };
  });
}

async function playCalls(page: Page): Promise<number> {
  return page.evaluate(
    () => (window as unknown as { __plays: unknown[] }).__plays.length,
  );
}

async function lastPlay(
  page: Page,
): Promise<{ state: string; error: string } | null> {
  return page.evaluate(() => {
    const plays = (
      window as unknown as { __plays: Array<{ state: string; error: string }> }
    ).__plays;
    return plays[plays.length - 1] ?? null;
  });
}

async function expectButton(
  page: Page,
  shows: "play" | "pause",
  timeout = 2_000,
) {
  await expect
    .poll(() => buttonShows(page), {
      message: `the button shows ${shows}`,
      timeout,
    })
    .toBe(shows);
}

/** The player has tried both stream addresses and both have failed. */
async function waitForStreamFailures(page: Page, streams: { token: string }[]) {
  await expect
    .poll(() => new Set(streams.map((s) => s.token)).size, {
      message:
        "the stream is requested under a second address after the first fails",
      timeout: 10_000,
    })
    .toBeGreaterThanOrEqual(2);
  await expect
    .poll(
      () =>
        page.evaluate(() => document.querySelector("audio")?.error?.code ?? 0),
      {
        message: "the second stream address fails too",
      },
    )
    .toBeGreaterThan(0);
  await page.waitForTimeout(1_000);
}

async function pressSpace(page: Page) {
  await page.evaluate(() =>
    (document.activeElement as HTMLElement | null)?.blur(),
  );
  await page.keyboard.press(" ");
}

async function openChapterList(page: Page) {
  await page.locator('button[title="Chapter list"]').click();
}

function chapterEntry(page: Page) {
  return page.locator("div.fixed button").filter({ hasText: "1. Opening" });
}

const leavingToasts = (page: Page) =>
  page.locator('[data-sonner-toast][data-removed="true"]');
const liveFailureToasts = (page: Page) =>
  liveToasts(page).filter({ hasText: PLAY_FAILED });

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

test.describe("Pressing play on a stream that cannot play", () => {
  // AC-013.
  for (const [label, stream] of brokenStreams) {
    test(`the play button, when the stream ${label}: the button stays on play and one toast shows`, async ({
      page,
    }) => {
      const { streams } = await stubPlayer(page, { stream: () => stream });
      await page.goto("/listen/1?workId=1");
      await waitForStreamFailures(page, streams);
      const mark = await toastMark(page);
      await playButton(page).click();
      await expectButton(page, "play");
      await expectOnlyToast(page, PLAY_FAILED, "error", mark);
    });
  }

  // AC-015.
  test("a chapter-list entry, when the stream answers 404: the button stays on play and one toast shows", async ({
    page,
  }) => {
    const { streams } = await stubPlayer(page, {
      stream: () => notFound("no such file"),
      chapters: () => ONE_CHAPTER,
    });
    await page.goto("/listen/1?workId=1");
    await waitForStreamFailures(page, streams);
    await openChapterList(page);
    const mark = await toastMark(page);
    await chapterEntry(page).click();
    await expectButton(page, "play");
    await expectOnlyToast(page, PLAY_FAILED, "error", mark);
  });

  // AC-017.
  test("with no press, the stream's own failure report carries the shared text", async ({
    page,
  }) => {
    const { streams } = await stubPlayer(page, {
      stream: () => notFound("no such file"),
    });
    await page.goto("/listen/1?workId=1");
    await waitForStreamFailures(page, streams);
    await expectOnlyToast(page, PLAY_FAILED, "error", 0);
  });
});

test.describe("Pressing play with no stream address", () => {
  // AC-014 and REQ-003's chapter door with no address. A token reply with no
  // body reaches the controller outside its error handling; any resulting
  // page error is recorded, not asserted.
  const tokenFailures: Array<[string, Reply | "held"]> = [
    ["answers 500", serverError("token failed")],
    ["fails at the network", "network-error"],
    ["answers 404", clientError(404, "no such item")],
    ["answers 200 with no body", emptyOk],
    ["is held", "held"],
  ];
  const doors: Array<
    [string, (page: Page) => Promise<void>, (page: Page) => Promise<void>]
  > = [
    [
      "the play button",
      async () => undefined,
      async (page) => playButton(page).click(),
    ],
    [
      "a chapter-list entry",
      openChapterList,
      async (page) => chapterEntry(page).click(),
    ],
  ];
  for (const [door, prepare, press] of doors) {
    for (const [label, failure] of tokenFailures) {
      test(`${door}, when the token request ${label}: no native play call, the button stays on play and one toast shows`, async ({
        page,
      }) => {
        const pageErrors: string[] = [];
        page.on("pageerror", (error) => pageErrors.push(error.message));
        await trackPlayCalls(page);
        const { calls } = await stubPlayer(page, {
          token: failure === "held" ? undefined : () => failure,
          chapters: () => ONE_CHAPTER,
        });
        const { held } = await holdRequests(
          page,
          (url, method) =>
            failure === "held" &&
            method === "POST" &&
            url.pathname.endsWith("/stream-token"),
        );
        await page.goto("/listen/1?workId=1");
        await expect(playButton(page)).toBeVisible();
        if (failure === "held") {
          await waitForHeld(held, 1, "the token request");
        } else {
          await expect
            .poll(
              () =>
                calls.filter((c) => c.path.endsWith("/stream-token")).length,
            )
            .toBeGreaterThan(0);
        }
        await page.waitForTimeout(500);
        expect(await media(page), "no stream address").toMatchObject({
          src: "",
        });
        const recorded = pageErrors.length ? pageErrors.join(" | ") : "none";
        test
          .info()
          .annotations.push({ type: "page errors", description: recorded });
        console.log(
          `page errors with the token request ${label}: ${recorded}`,
        );
        await prepare(page);
        const mark = await toastMark(page);
        expect(mark, "the token failure alone shows no toast").toBe(0);
        const playsBefore = await playCalls(page);
        await press(page);
        await expectButton(page, "play");
        await expectOnlyToast(page, PLAY_FAILED, "error", mark);
        expect(
          await playCalls(page),
          "no native play call without a stream address",
        ).toBe(playsBefore);
        if (failure !== "held") return;

        // The token is answered and its stream loads: nothing starts until
        // the user presses play again.
        await held[0]!.release();
        await waitForItemMetadata(page, 1);
        await page.waitForTimeout(1_000);
        expect(
          (await media(page)).paused,
          "the element stays paused once the stream loads",
        ).toBe(true);
        expect(await playCalls(page), "still no native play call").toBe(
          playsBefore,
        );
        await expectButton(page, "play");
        await playButton(page).click();
        await expectButton(page, "pause");
        await expectPlaying(page);
      });
    }
  }
});

test.describe("A start that fails reports itself", () => {
  // AC-049: the stream fails only after the press, and the controller's
  // re-mint is held, so no other report can stand in for the start's own.
  const doors: Array<[string, (page: Page) => Promise<void>]> = [
    ["the play button", async (page) => playButton(page).click()],
    ["Space", pressSpace],
    [
      "a chapter-list entry",
      async (page) => {
        await openChapterList(page);
        await chapterEntry(page).click();
      },
    ],
  ];
  for (const [door, press] of doors) {
    test(`${door}: one toast before the re-mint is answered, and none added after it`, async ({
      page,
    }) => {
      await trackPlayCalls(page);
      let mints = 0;
      const { streams } = await stubPlayer(page, {
        stream: () => notFound("no such file"),
        chapters: () => ONE_CHAPTER,
      });
      const { held } = await holdRequests(page, (url, method) => {
        if (method === "GET" && url.pathname === "/api/v1/stream/1")
          return url.searchParams.get("token") === "token-1-1";
        if (
          method === "POST" &&
          url.pathname === "/api/v1/workfile/1/stream-token"
        ) {
          mints += 1;
          return mints === 2;
        }
        return false;
      });
      await page.goto("/listen/1?workId=1");
      await waitForHeld(held, 1, "the first stream request");
      await expect(playButton(page)).toBeVisible();
      await page.waitForTimeout(500);
      expect(await toastMark(page), "no toast before the press").toBe(0);

      await press(page);
      await expectButton(page, "pause");
      expect(await lastPlay(page), "the start is pending").toEqual({
        state: "pending",
        error: "",
      });

      await held[0]!.release(notFound("no such file"));
      await waitForHeld(held, 2, "the re-mint's token request");
      await expectButton(page, "play");
      await expectOnlyToast(page, PLAY_FAILED, "error", 0);

      const mark = await toastMark(page);
      await held[1]!.release();
      await expect
        .poll(() => streams.some((s) => s.token === "token-1-2"), {
          message: "the stream is requested under the re-minted address",
        })
        .toBe(true);
      await page.waitForTimeout(1_000);
      await expectOnlyToast(page, PLAY_FAILED, "error", mark);
      expect(
        await toastMark(page),
        "no toast added by the stream's own failure",
      ).toBe(mark);
    });
  }

  // AC-016: the shared toast's life, with the page clock paused.
  test("a failure while the toast is on screen, while it leaves, and after it has gone", async ({
    page,
  }) => {
    await page.clock.install();
    let mints = 0;
    await stubPlayer(page, { stream: () => notFound("no such file") });
    const { held } = await holdRequests(page, (url, method) => {
      if (method === "GET" && url.pathname === "/api/v1/stream/1")
        return url.searchParams.get("token") === "token-1-1";
      if (
        method === "POST" &&
        url.pathname === "/api/v1/workfile/1/stream-token"
      ) {
        mints += 1;
        return mints === 2;
      }
      return false;
    });
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "the first stream request");
    await expect(playButton(page)).toBeVisible();
    const pageNow = await page.evaluate(() => Date.now());
    await page.clock.pauseAt(pageNow + 1_000);

    await playButton(page).click();
    await held[0]!.release(notFound("no such file"));
    await waitForHeld(held, 2, "the re-mint's token request");
    await page.clock.runFor(50);
    await expectButton(page, "play");
    await expectOnlyToast(page, PLAY_FAILED, "error", 0);

    // Live: another failure replaces the toast in place.
    let mark = await toastMark(page);
    await playButton(page).click();
    await page.clock.runFor(50);
    await expectButton(page, "play");
    await expectOnlyToast(page, PLAY_FAILED, "error", mark);
    expect(await toastMark(page), "the second press adds no toast").toBe(mark);

    // Leaving: close it, and fail again before it has gone.
    await liveFailureToasts(page).locator("[data-close-button]").click();
    await advanceUntil(
      page,
      async () => (await leavingToasts(page).count()) === 1,
      "the toast starting to leave",
      100,
      10,
    );
    await playButton(page).click();
    await expectButton(page, "play");
    expect(
      await liveFailureToasts(page).count(),
      "at most one such toast on screen beside the leaving one",
    ).toBeLessThanOrEqual(1);

    // Gone: once nothing with the text remains, the next failure shows anew.
    await page.mouse.move(0, 0);
    await advanceUntil(
      page,
      async () =>
        (await page
          .locator("[data-sonner-toast]")
          .filter({ hasText: PLAY_FAILED })
          .count()) === 0,
      "every such toast leaving",
      8_000,
      50,
    );
    // Sonner's removal of the old toast finishes on the frames after its
    // element goes.
    await page.clock.runFor(100);
    mark = await toastMark(page);
    await playButton(page).click();
    await page.clock.runFor(50);
    await expectButton(page, "play");
    await expectOnlyToast(page, PLAY_FAILED, "error", mark);
    expect(await toastMark(page), "one new toast").toBe(mark + 1);
  });
});

test.describe("Playback that works", () => {
  // AC-018.
  test("the silent WAV plays: the button shows pause and no toast", async ({
    page,
  }) => {
    await stubPlayer(page);
    await page.goto("/listen/1?workId=1");
    await waitForItemMetadata(page, 1);
    const mark = await toastMark(page);
    await playButton(page).click();
    await expectButton(page, "pause");
    await expectPlaying(page);
    await page.waitForTimeout(500);
    await expectNoToast(page, mark);
  });

  test("a start still pending when the user pauses is not a failure", async ({
    page,
  }) => {
    await trackPlayCalls(page);
    await stubPlayer(page);
    const { held } = await holdRequests(
      page,
      (url, method) => method === "GET" && url.pathname === "/api/v1/stream/1",
    );
    await page.goto("/listen/1?workId=1");
    await waitForHeld(held, 1, "the stream request");
    await expect(playButton(page)).toBeVisible();
    const mark = await toastMark(page);
    await playButton(page).click();
    await expectButton(page, "pause");
    expect(await lastPlay(page), "the start is pending").toEqual({
      state: "pending",
      error: "",
    });
    await playButton(page).click();
    await expectButton(page, "play");
    await expect
      .poll(() => lastPlay(page))
      .toEqual({ state: "rejected", error: "AbortError" });
    await held[0]!.release();
    await waitForItemMetadata(page, 1);
    await page.waitForTimeout(1_000);
    expect((await media(page)).paused, "the element stays paused").toBe(true);
    await expectButton(page, "play");
    await expectNoToast(page, mark);
  });
});

test.describe("A stream-address refresh", () => {
  /**
   * The first token expires a few seconds past the controller's five-minute
   * margin, so it schedules a refresh a few seconds after minting. The page
   * clock is paused once the stream has loaded, so that refresh runs only
   * once playback is confirmed. Every token string is distinct, so each
   * address loads anew.
   */
  async function playThenRefresh(page: Page, refreshStream: StreamAnswer) {
    await page.clock.install();
    const setup = await stubPlayer(page, {
      token: (_id, n) =>
        ok({
          token: `token-1-${n}`,
          exp: Math.floor(Date.now() / 1000) + (n === 1 ? 305 : 86_400),
        }),
      stream: (_id, token) =>
        token === "token-1-1"
          ? "wav"
          : token === "token-1-2"
            ? refreshStream
            : notFound("gone"),
    });
    await page.goto("/listen/1?workId=1");
    await waitForItemMetadata(page, 1);
    await page.clock.pauseAt(await page.evaluate(() => Date.now() + 500));
    expect(
      setup.streams.every((s) => s.token === "token-1-1"),
      "no refresh yet",
    ).toBe(true);
    await playButton(page).click();
    await expectPlaying(page);
    const before = await media(page);
    await page.clock.runFor(6_000);
    await expect
      .poll(() => setup.streams.some((s) => s.token === "token-1-2"), {
        message: "the refreshed address is requested",
      })
      .toBe(true);
    return { ...setup, before };
  }

  // AC-050.
  test("whose streams fail after playback was running: one toast, and the button shows play", async ({
    page,
  }) => {
    const { streams } = await playThenRefresh(page, notFound("gone"));
    await expect
      .poll(() => streams.some((s) => s.token === "token-1-3"), {
        message: "the recovery address is requested",
      })
      .toBe(true);
    await expectButton(page, "play", 5_000);
    // Sonner adds a toast from a timer, which the paused page clock holds.
    await page.clock.runFor(50);
    await expectOnlyToast(page, PLAY_FAILED, "error", 0);
  });

  // AC-051.
  test("that loads restores the time and keeps playing, with no toast", async ({
    page,
  }) => {
    const { before } = await playThenRefresh(page, "wav");
    await expect
      .poll(async () => (await media(page)).src, {
        message: "the refreshed address loads",
      })
      .toContain("token=token-1-2");
    await expectPlaying(page);
    expect((await media(page)).time).toBeGreaterThanOrEqual(before.time);
    await expectButton(page, "pause");
    await expectNoToast(page, 0);
  });
});
