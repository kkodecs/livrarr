import { expect, test, type Page } from "@playwright/test";
import {
  ok,
  sampleBook,
  serverError,
  servePdfWorker,
  silentWav,
  expectNoToast,
  expectOnlyToast,
  liveToasts,
  stubApi,
  toastMark,
  type ApiCall,
  type Reply,
} from "./stubbedApi";
import { cfiAfter, renditionStartCfi } from "./epubReader";

// Each place that saves the reading or listening position runs the same
// failure run through its real control: failure, failure, success, failure.
// The saves themselves are answered by `saveReply`, set before each step.

const SAVE_MESSAGE = "Could not save your place.";
const PROGRESS_PATH = /^\/workfile\/(\d+)\/progress$/;

/** Longer than a one-second retry, shorter than the ten-second playing save. */
const SETTLE_MS = 2_500;

const noProgress: Reply = {
  status: 404,
  json: { status: 404, error: "not_found", message: "no progress" },
};

const steps: Array<{ reply: Reply; toastsAfter: 0 | 1; idle?: boolean }> = [
  { reply: serverError("progress write failed"), toastsAfter: 1 },
  { reply: "network-error", toastsAfter: 1, idle: true },
  { reply: ok({ success: true }), toastsAfter: 0 },
  { reply: serverError("progress write failed"), toastsAfter: 1 },
];

function saves(calls: ApiCall[]) {
  return calls.filter(
    (c) => c.method === "PUT" && PROGRESS_PATH.test(c.path),
  );
}

async function expectSaveToasts(page: Page, count: 0 | 1, since: number) {
  if (count === 0) {
    await expectNoToast(page, since);
  } else {
    await expectOnlyToast(page, SAVE_MESSAGE, "error", since);
  }
}

/**
 * Drive the four steps. `trigger(i)` performs the user action for step i.
 * Each step causes exactly one save and no other save while its reply
 * settles (a failed save is not retried); then the whole toast container is
 * checked. `observe(i)` runs as soon as the step's save is seen, to read what
 * the page itself reached.
 */
async function runFailureRun(
  page: Page,
  calls: ApiCall[],
  setReply: (reply: Reply) => void,
  trigger: (step: number) => Promise<void>,
  options: {
    saveTimeout?: number;
    observe?: (step: number) => Promise<void>;
  } = {},
) {
  await expect(liveToasts(page)).toHaveCount(0);
  for (const [i, step] of steps.entries()) {
    setReply(step.reply);
    const before = saves(calls).length;
    const mark = await toastMark(page);
    await trigger(i);
    await expect
      .poll(() => saves(calls).length, {
        timeout: options.saveTimeout ?? 10_000,
        intervals: [100],
      })
      .toBeGreaterThan(before);
    await options.observe?.(i);
    await page.waitForTimeout(SETTLE_MS);
    expect(saves(calls)).toHaveLength(before + 1);
    await expectSaveToasts(page, step.toastsAfter, mark);
    if (step.idle) {
      // Sit idle past the save pause and a normal toast's lifetime.
      await page.waitForTimeout(5_000);
      expect(saves(calls)).toHaveLength(before + 1);
      await expectSaveToasts(page, 1, mark);
    }
  }
}

function libraryItem(id: number, file: string, mediaType: string) {
  return {
    id,
    path: `Sample Author/${file}`,
    mediaType,
    fileSize: 1000,
    importedAt: "2026-09-01T00:00:00Z",
    progressPct: null,
    durationSeconds: null,
    finishedAt: null,
  };
}

test.describe("Saving your place fails", () => {
  test("EPUB reader: turning the page", async ({ page }) => {
    let saveReply: Reply = ok({ success: true });
    const calls = await stubApi(page, (call) => {
      const bare = call.path.split("?")[0]!;
      if (call.method === "PUT" && PROGRESS_PATH.test(bare)) return saveReply;
      if (call.method !== "GET") return undefined;
      if (bare === "/workfile/1")
        return ok(libraryItem(1, "Sample.epub", "ebook"));
      if (bare === "/workfile/1/download")
        return {
          status: 200,
          bytes: sampleBook("sample.epub"),
          contentType: "application/epub+zip",
        };
      if (bare === "/workfile/1/bookmarks") return ok([]);
      if (bare === "/workfile/1/progress") return noProgress;
      if (bare === "/workfile/1/cross-format/anchors")
        return ok([{ cfi: "epubcfi(/6/2!/4/2/1:0)", ts: 42 }]);
      if (bare === "/workfile/1/cross-format/prompt") return ok(null);
      return undefined;
    });
    await page.goto("/read/1");
    await expect(page.locator('button[title="Add bookmark"]')).toBeVisible();
    await expect
      .poll(
        () =>
          page.evaluate(() => localStorage.getItem("livrarr-locations-1")),
        { timeout: 30_000 },
      )
      .not.toBeNull();
    // Past the reader's opening window, a page turn is ordinary progress.
    await page.waitForTimeout(3_500);

    // After each turn: the percentage the toolbar shows, and the start CFI the
    // live epub.js rendition reports (read-only, through the reader's React
    // component that holds it).
    const shown: number[] = [];
    const reached: Array<string | null> = [];
    await runFailureRun(
      page,
      calls,
      (reply) => {
        saveReply = reply;
      },
      async () => {
        await page.keyboard.press("ArrowRight");
      },
      {
        observe: async () => {
          const text = await page.locator("span.tabular-nums").innerText();
          shown.push(Number(text.replace("%", "")));
          reached.push(await renditionStartCfi(page));
        },
      },
    );

    const sent = saves(calls);
    expect(sent).toHaveLength(4);
    const bodies = sent.map((s) => s.body as Record<string, unknown>);
    for (const [i, body] of bodies.entries()) {
      expect(sent[i]!.path).toBe("/workfile/1/progress");
      expect(String(body.position)).toMatch(/^epubcfi\(/);
      expect(body.position, `turn ${i + 1} saves the location it reached`).toBe(
        reached[i],
      );
      expect(Math.round(Number(body.progress_pct) * 100)).toBe(shown[i]);
      expect(body.kind).toBe("progress");
      expect(body.cross_format_ts).toBe(42);
      if (i > 0) {
        const previous = bodies[i - 1]!;
        expect(
          cfiAfter(String(body.position), String(previous.position)),
          `turn ${i + 1} saves a later position than turn ${i}`,
        ).toBe(true);
        expect(Number(body.progress_pct)).toBeGreaterThanOrEqual(
          Number(previous.progress_pct),
        );
      }
    }
    expect(Number(bodies[3]!.progress_pct)).toBeGreaterThan(
      Number(bodies[0]!.progress_pct),
    );
  });

  // A PDF that downloads cannot stay open in the built app: the reader passes
  // pdf.js a new file object on every render, pdf.js has already detached the
  // buffer, and the page throws "Cannot perform Construct on a detached
  // ArrayBuffer". The page-turn control is unreachable until that is fixed.
  test.fixme("PDF reader: moving to the next page", async ({ page }) => {
    let saveReply: Reply = ok({ success: true });
    await servePdfWorker(page);
    const calls = await stubApi(page, (call) => {
      const bare = call.path.split("?")[0]!;
      if (call.method === "PUT" && PROGRESS_PATH.test(bare)) return saveReply;
      if (call.method !== "GET") return undefined;
      if (bare === "/workfile/1")
        return ok(libraryItem(1, "Sample.pdf", "ebook"));
      if (bare === "/workfile/1/download")
        return {
          status: 200,
          bytes: sampleBook("sample.pdf"),
          contentType: "application/pdf",
        };
      if (bare === "/workfile/1/progress") return noProgress;
      return undefined;
    });
    await page.goto("/read/1");
    await expect(page.getByText("/ 5")).toBeVisible();

    const next = page.locator("button:has(svg.lucide-chevron-right)");
    await runFailureRun(
      page,
      calls,
      (reply) => {
        saveReply = reply;
      },
      async () => {
        await next.click();
      },
    );

    expect(saves(calls).map((s) => s.body)).toEqual([
      { position: "2", progress_pct: 2 / 5 },
      { position: "3", progress_pct: 3 / 5 },
      { position: "4", progress_pct: 4 / 5 },
      { position: "5", progress_pct: 5 / 5 },
    ]);
  });
});

test.describe("Saving your place in the audio player fails", () => {
  const WAV = silentWav(120);

  function playerApi(
    page: Page,
    options: { prompt: unknown; saveReply: () => Reply },
  ) {
    return stubApi(page, (call) => {
      const bare = call.path.split("?")[0]!;
      if (call.method === "PUT" && PROGRESS_PATH.test(bare))
        return options.saveReply();
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
      if (/^\/workfile\/\d+\/progress$/.test(bare)) return noProgress;
      if (/^\/workfile\/\d+\/cross-format\/prompt$/.test(bare))
        return ok(options.prompt);
      return undefined;
    });
  }

  async function waitForDuration(page: Page) {
    await expect
      .poll(() =>
        page.evaluate(() => document.querySelector("audio")?.duration ?? NaN),
      )
      .toBeGreaterThan(100);
  }

  /** The player has loaded item `id`'s audio and knows its duration. */
  async function waitForItemMetadata(page: Page, id: number) {
    await expect
      .poll(() =>
        page.evaluate((itemId) => {
          const audio = document.querySelector("audio");
          return !!audio &&
            audio.currentSrc.includes(`/stream/${itemId}?`) &&
            audio.duration > 100
            ? "loaded"
            : "waiting";
        }, id),
      )
      .toBe("loaded");
  }

  test("moving the seek bar", async ({ page }) => {
    let saveReply: Reply = ok({ success: true });
    const calls = await playerApi(page, {
      prompt: null,
      saveReply: () => saveReply,
    });
    await page.goto("/listen/1?workId=1");
    await waitForDuration(page);

    const seekBar = page.locator('input[type="range"][step="0.1"]');
    const targets = ["10", "20", "30", "40"];
    await runFailureRun(
      page,
      calls,
      (reply) => {
        saveReply = reply;
      },
      async (i) => {
        await seekBar.fill(targets[i]!);
      },
    );

    const bodies = saves(calls).map((s) => s.body as Record<string, unknown>);
    expect(bodies.map((b) => b.position)).toEqual(targets);
    for (const body of bodies) {
      expect(body.kind).toBe("seek");
      expect(body.cross_format_ts).toBe(Number(body.position));
      expect(body.progress_pct).toBeCloseTo(Number(body.position) / 120, 3);
    }
  });

  test("the ten-second save while playing", async ({ page }) => {
    let saveReply: Reply = ok({ success: true });
    const calls = await playerApi(page, {
      prompt: null,
      saveReply: () => saveReply,
    });
    await page.goto("/listen/1?workId=1");
    await waitForDuration(page);

    // The media element's own time, read as soon as each save is seen.
    const mediaTime: number[] = [];
    await runFailureRun(
      page,
      calls,
      (reply) => {
        saveReply = reply;
      },
      async (i) => {
        if (i === 0) await page.locator("button:has(svg.lucide-play)").click();
      },
      {
        saveTimeout: 15_000,
        observe: async () => {
          mediaTime.push(
            await page.evaluate(
              () => document.querySelector("audio")?.currentTime ?? NaN,
            ),
          );
        },
      },
    );

    const sent = saves(calls);
    expect(sent).toHaveLength(4);
    const positions = sent.map((s) =>
      Number((s.body as Record<string, unknown>).position),
    );
    for (const [i, save] of sent.entries()) {
      const body = save.body as Record<string, unknown>;
      expect(save.path).toBe("/workfile/1/progress");
      expect(body.kind).toBe("progress");
      expect(body.cross_format_ts).toBe(positions[i]);
      expect(body.progress_pct).toBeCloseTo(positions[i]! / 120, 3);
      // Saved time is the playing element's time at the tick.
      expect(positions[i]).toBeGreaterThan(0);
      expect(positions[i]).toBeLessThanOrEqual(mediaTime[i]! + 0.05);
      expect(positions[i]).toBeGreaterThan(mediaTime[i]! - 2);
      if (i > 0) {
        expect(positions[i]! - positions[i - 1]!).toBeGreaterThan(5);
        // Ten-second cadence: nothing between ticks, so no retry slipped in.
        const gap = save.at - sent[i - 1]!.at;
        expect(gap).toBeGreaterThan(9_000);
        expect(gap).toBeLessThan(11_000);
      }
    }
  });

  test("jumping from the resume banner", async ({ page }) => {
    let saveReply: Reply = ok({ success: true });
    const calls = await playerApi(page, {
      prompt: { format: "ebook", position: "12.5", label: "Chapter 2" },
      saveReply: () => saveReply,
    });
    await page.goto("/listen/1?workId=1");
    await waitForDuration(page);

    await runFailureRun(
      page,
      calls,
      (reply) => {
        saveReply = reply;
      },
      async (i) => {
        if (i > 0) {
          // Opening the next audiobook in the same session offers the banner again.
          await page.evaluate((id) => {
            window.history.pushState(null, "", `/listen/${id}?workId=1`);
            window.dispatchEvent(new PopStateEvent("popstate"));
          }, i + 1);
        }
        await waitForItemMetadata(page, i + 1);
        await page.getByRole("button", { name: "Jump" }).click();
      },
    );

    const sent = saves(calls);
    expect(sent.map((s) => s.path)).toEqual([
      "/workfile/1/progress",
      "/workfile/2/progress",
      "/workfile/3/progress",
      "/workfile/4/progress",
    ]);
    for (const save of sent) {
      expect(save.body).toEqual({
        position: "12.5",
        progress_pct: expect.closeTo(12.5 / 120, 6),
        kind: "seek",
        cross_format_ts: 12.5,
      });
    }
  });
});
