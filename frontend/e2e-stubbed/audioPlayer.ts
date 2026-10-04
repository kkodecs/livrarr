import { expect, type Locator, type Page, type Route } from "@playwright/test";
import {
  answerWith,
  notFound,
  ok,
  silentWav,
  stubApi,
  type ApiCall,
  type Reply,
} from "./stubbedApi";

/**
 * Helpers for browser tests of the audio player at `/listen/{id}?workId=1`.
 * Every /api request is answered by `stubApi`; the stream is answered by a
 * route of its own that honours byte ranges, so the element can seek.
 */

export const PROGRESS_PATH = /^\/workfile\/(\d+)\/progress$/;
const STREAM_PATH = /^\/api\/v1\/stream\/(\d+)$/;

/** What the stream route serves: the item's WAV, a reply, or a network failure. */
export type StreamAnswer = "wav" | Reply;

export interface PlayerSetup {
  /** The saved-place read for item `id`. Default: 404 (no saved place). */
  progress?: (id: number) => Reply;
  /** The stream-token mint; `n` counts mints for that item from 1. Default: a distinct token, far expiry. */
  token?: (id: number, n: number) => Reply;
  /** The stream for item `id` under `token`. Default: the WAV. */
  stream?: (id: number, token: string) => StreamAnswer;
  chapters?: (id: number) => unknown[];
  bookmarks?: (id: number) => unknown[];
  /** The cross-format prompt for item `id`. Default: none. */
  prompt?: (id: number, currentTs: string | null) => Reply;
  /** Any other request (bookmark writes, saves). Default: saves succeed. */
  other?: (call: ApiCall) => Reply | undefined;
  /** WAV length in seconds. */
  wavSeconds?: number;
}

export const farExpiry = () => Math.floor(Date.now() / 1000) + 86_400;

export interface StreamRequest {
  id: number;
  token: string;
  at: number;
}

export interface Player {
  calls: ApiCall[];
  streams: StreamRequest[];
}

/** Answers a WAV request, honouring a `Range` header. */
async function serveWav(route: Route, wav: Buffer) {
  const range = route.request().headers()["range"];
  const match = range ? /bytes=(\d+)-(\d*)/.exec(range) : null;
  if (!match) {
    await route.fulfill({
      status: 200,
      body: wav,
      headers: { "content-type": "audio/wav", "accept-ranges": "bytes" },
    });
    return;
  }
  const start = Number(match[1]);
  const end = match[2]
    ? Math.min(Number(match[2]), wav.length - 1)
    : wav.length - 1;
  if (start >= wav.length) {
    await route.fulfill({
      status: 416,
      headers: { "content-range": `bytes */${wav.length}` },
    });
    return;
  }
  await route.fulfill({
    status: 206,
    body: wav.subarray(start, end + 1),
    headers: {
      "content-type": "audio/wav",
      "accept-ranges": "bytes",
      "content-range": `bytes ${start}-${end}/${wav.length}`,
    },
  });
}

export async function stubPlayer(
  page: Page,
  setup: PlayerSetup = {},
): Promise<Player> {
  const wav = silentWav(setup.wavSeconds ?? 400);
  const mints = new Map<number, number>();
  const calls = await stubApi(page, (call) => {
    const bare = call.path.split("?")[0]!;
    const query = new URLSearchParams(call.path.split("?")[1] ?? "");
    const other = setup.other?.(call);
    if (other) return other;
    if (call.method === "PUT" && PROGRESS_PATH.test(bare))
      return ok({ success: true });
    const mint = /^\/workfile\/(\d+)\/stream-token$/.exec(bare);
    if (call.method === "POST" && mint) {
      const id = Number(mint[1]);
      const n = (mints.get(id) ?? 0) + 1;
      mints.set(id, n);
      return (
        setup.token?.(id, n) ??
        ok({ token: `token-${id}-${n}`, exp: farExpiry() })
      );
    }
    if (call.method !== "GET") return undefined;
    if (bare === "/work/1")
      return ok({
        id: 1,
        title: "Sample Audiobook",
        authorName: "Sample Author",
      });
    let m = /^\/workfile\/(\d+)\/chapters$/.exec(bare);
    if (m) return ok(setup.chapters?.(Number(m[1])) ?? []);
    m = /^\/workfile\/(\d+)\/bookmarks$/.exec(bare);
    if (m) return ok(setup.bookmarks?.(Number(m[1])) ?? []);
    m = PROGRESS_PATH.exec(bare);
    if (m) return setup.progress?.(Number(m[1])) ?? notFound("no progress");
    m = /^\/workfile\/(\d+)\/cross-format\/prompt$/.exec(bare);
    if (m)
      return setup.prompt?.(Number(m[1]), query.get("current_ts")) ?? ok(null);
    return undefined;
  });
  const streams: StreamRequest[] = [];
  await page.route(
    (url) => STREAM_PATH.test(url.pathname),
    async (route) => {
      const url = new URL(route.request().url());
      const id = Number(STREAM_PATH.exec(url.pathname)![1]);
      const token = url.searchParams.get("token") ?? "";
      streams.push({ id, token, at: Date.now() });
      const answer = setup.stream?.(id, token) ?? "wav";
      if (answer === "wav") await serveWav(route, wav);
      else await answerWith(route, answer);
    },
  );
  return { calls, streams };
}

export function progressReads(calls: ApiCall[], id: number) {
  return calls.filter(
    (c) => c.method === "GET" && c.path === `/workfile/${id}/progress`,
  );
}

export function saves(calls: ApiCall[], id?: number) {
  return calls.filter(
    (c) =>
      c.method === "PUT" &&
      PROGRESS_PATH.test(c.path) &&
      (id === undefined || c.path === `/workfile/${id}/progress`),
  );
}

export function promptRequests(calls: ApiCall[], id?: number) {
  return calls.filter((c) =>
    id === undefined
      ? /^\/workfile\/\d+\/cross-format\/prompt/.test(c.path)
      : c.path.startsWith(`/workfile/${id}/cross-format/prompt`),
  );
}

/** The player's main play/pause button. */
export function playButton(page: Page): Locator {
  return page.locator(
    "button:has(> svg.lucide-play), button:has(> svg.lucide-pause)",
  );
}

/** "play" or "pause": the icon the main button shows. */
export async function buttonShows(
  page: Page,
): Promise<"play" | "pause" | "absent"> {
  return page.evaluate(() => {
    const play = document.querySelector("button > svg.lucide-play");
    const pause = document.querySelector("button > svg.lucide-pause");
    if (pause) return "pause";
    if (play) return "play";
    return "absent";
  });
}

export interface MediaState {
  time: number;
  paused: boolean;
  duration: number;
  src: string;
  readyState: number;
}

export async function media(page: Page): Promise<MediaState> {
  return page.evaluate(() => {
    const a = document.querySelector("audio");
    return {
      time: a?.currentTime ?? NaN,
      paused: a?.paused ?? true,
      duration: a?.duration ?? NaN,
      src: a?.currentSrc ?? "",
      readyState: a?.readyState ?? 0,
    };
  });
}

/** The element has loaded item `id`'s audio and knows its duration. */
export async function waitForItemMetadata(
  page: Page,
  id: number,
  timeout = 15_000,
) {
  await expect
    .poll(
      () =>
        page.evaluate((itemId) => {
          const audio = document.querySelector("audio");
          return !!audio &&
            audio.currentSrc.includes(`/stream/${itemId}?`) &&
            audio.duration > 100
            ? "loaded"
            : "waiting";
        }, id),
      { message: `item ${id}'s audio metadata loads`, timeout },
    )
    .toBe("loaded");
}

/** The element is playing: not paused and its time moves. */
export async function expectPlaying(page: Page) {
  const before = (await media(page)).time;
  await expect
    .poll(
      async () => {
        const m = await media(page);
        return !m.paused && m.time > before + 0.2;
      },
      { message: "the media element is playing", timeout: 5_000 },
    )
    .toBe(true);
}

/** Opens item `id` in the same page, as a link inside the app does. */
export async function openItemInPage(page: Page, id: number) {
  await page.evaluate((itemId) => {
    window.history.pushState(null, "", `/listen/${itemId}?workId=1`);
    window.dispatchEvent(new PopStateEvent("popstate"));
  }, id);
}

/** The controls a loaded player shows, by the names a user finds them by. */
export function controls(page: Page) {
  return {
    "play button": playButton(page),
    "previous chapter": page.locator('button[title="Previous chapter"]'),
    "next chapter": page.locator('button[title="Next chapter"]'),
    "chapter list": page.locator('button[title="Chapter list"]'),
    "seek bar": page.locator('input[type="range"][step="0.1"]'),
    "sleep timer": page.locator('button[title="Sleep timer"]'),
    "add bookmark": page.locator('button[title="Add bookmark"]'),
    "resume banner": page.getByRole("button", { name: "Jump" }),
  };
}

export async function expectNoControls(page: Page) {
  for (const [name, locator] of Object.entries(controls(page))) {
    await expect(locator, `${name} is not present`).toHaveCount(0);
  }
}

export const TWO_CHAPTERS = [
  {
    id: 11,
    chapterIndex: 0,
    title: "Opening",
    startTimeSecs: 0,
    endTimeSecs: 200,
  },
  {
    id: 12,
    chapterIndex: 1,
    title: "Second part",
    startTimeSecs: 200,
    endTimeSecs: 400,
  },
];

export const ONE_CHAPTER = [
  {
    id: 11,
    chapterIndex: 0,
    title: "Opening",
    startTimeSecs: 0,
    endTimeSecs: 400,
  },
];
