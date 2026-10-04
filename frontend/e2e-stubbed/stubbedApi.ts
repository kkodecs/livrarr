import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, type Locator, type Page, type Route } from "@playwright/test";

/**
 * The network boundary for browser tests of the working-tree UI. Every request
 * under /api/ is answered here; the page's own code (apiFetch, the readers'
 * raw download fetch, the <audio> stream) runs unchanged.
 */

export interface ApiCall {
  method: string;
  /** Path with the `/api/v1` prefix stripped, query string kept. */
  path: string;
  body: unknown;
  /** When the request reached the network boundary (ms since epoch). */
  at: number;
}

export type Reply =
  | { status: number; json?: unknown; bytes?: Buffer; contentType?: string }
  | "network-error";

export type Handler = (call: ApiCall) => Reply | undefined;

const here = path.dirname(fileURLToPath(import.meta.url));

export function sampleBook(name: "sample.epub" | "sample.pdf"): Buffer {
  return readFileSync(path.join(here, "sample-books", name));
}

/**
 * The PDF reader points pdf.js at a worker URL relative to its own module
 * (`assets/pdfjs-dist/build/pdf.worker.min.mjs`), which the Vite build does
 * not emit. Answer that request with the worker file of the pdf.js copy
 * react-pdf uses, so the reader can open a PDF.
 */
export async function servePdfWorker(page: Page) {
  const fromReactPdf = createRequire(
    createRequire(import.meta.url).resolve("react-pdf/package.json"),
  );
  const worker = readFileSync(
    fromReactPdf.resolve("pdfjs-dist/build/pdf.worker.min.mjs"),
  );
  await page.route("**/pdfjs-dist/build/pdf.worker.min.mjs", (route) =>
    route.fulfill({ status: 200, body: worker, contentType: "text/javascript" }),
  );
}

/** A silent mono 8-bit PCM WAV of `seconds` length. */
export function silentWav(seconds: number): Buffer {
  const rate = 8000;
  const samples = rate * seconds;
  const header = Buffer.alloc(44);
  header.write("RIFF", 0, "ascii");
  header.writeUInt32LE(36 + samples, 4);
  header.write("WAVE", 8, "ascii");
  header.write("fmt ", 12, "ascii");
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate, 28);
  header.writeUInt16LE(1, 32);
  header.writeUInt16LE(8, 34);
  header.write("data", 36, "ascii");
  header.writeUInt32LE(samples, 40);
  return Buffer.concat([header, Buffer.alloc(samples, 0x80)]);
}

export const ok = (json: unknown): Reply => ({ status: 200, json });

export const serverError = (message: string): Reply => ({
  status: 500,
  json: { status: 500, error: "internal", message },
});

export const notFound = (message: string): Reply => ({
  status: 404,
  json: { status: 404, error: "not_found", message },
});

export const clientError = (status: number, message: string): Reply => ({
  status,
  json: { status, error: "bad_request", message },
});

/** A 200 whose body is empty. */
export const emptyOk: Reply = {
  status: 200,
  bytes: Buffer.alloc(0),
  contentType: "application/json",
};

/** A saved-place reply carrying `position` as given. */
export const savedAt = (position: unknown): Reply =>
  ok({
    library_item_id: 1,
    position,
    progress_pct: 0.1,
    updated_at: "2026-09-01T00:00:00Z",
  });

/** Replies every authenticated page needs, used when the handler has none. */
function sessionReply(call: ApiCall): Reply | undefined {
  const bare = call.path.split("?")[0];
  if (call.method === "GET" && bare === "/auth/me") {
    return ok({
      user: {
        id: 1,
        username: "admin",
        role: "admin",
        createdAt: "2026-01-01T00:00:00Z",
        updatedAt: "2026-01-01T00:00:00Z",
      },
      authType: "session",
    });
  }
  if (call.method === "GET" && bare === "/setup/status") {
    return ok({ setupRequired: false });
  }
  return undefined;
}

export async function stubApi(page: Page, handler: Handler) {
  const calls: ApiCall[] = [];
  await page.addInitScript(() => {
    localStorage.setItem("livrarr_token", "session-token");
    localStorage.setItem("livrarr-tour-completed", "true");
    // Record every toast element as it is added, so a toast that appears and
    // expires between two looks is still seen.
    const added: Element[] = [];
    (window as unknown as { __toastsAdded: Element[] }).__toastsAdded = added;
    new MutationObserver((mutations) => {
      for (const mutation of mutations) {
        for (const node of Array.from(mutation.addedNodes)) {
          if (!(node instanceof Element)) continue;
          if (node.matches("[data-sonner-toast]")) added.push(node);
          added.push(...Array.from(node.querySelectorAll("[data-sonner-toast]")));
        }
      }
    }).observe(document, { childList: true, subtree: true });
  });
  await page.route(
    (url) => url.pathname.startsWith("/api/"),
    async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      let body: unknown;
      try {
        body = request.postDataJSON();
      } catch {
        body = request.postData();
      }
      const call: ApiCall = {
        method: request.method(),
        path: url.pathname.replace(/^\/api\/v1/, "") + url.search,
        body,
        at: Date.now(),
      };
      calls.push(call);
      const reply =
        handler(call) ??
        sessionReply(call) ??
        notFound(`unstubbed ${call.method} ${call.path}`);
      await answerWith(route, reply);
    },
  );
  return calls;
}

export async function answerWith(route: Route, reply: Reply) {
  if (reply === "network-error") {
    await route.abort("failed");
    return;
  }
  if (reply.bytes) {
    await route.fulfill({
      status: reply.status,
      body: reply.bytes,
      contentType: reply.contentType ?? "application/octet-stream",
    });
    return;
  }
  await route.fulfill({
    status: reply.status,
    contentType: "application/json",
    body: JSON.stringify(reply.json ?? null),
  });
}

export interface HeldRequest {
  path: string;
  /** Answers the request with `reply`, or passes it on to the stubs. */
  release: (reply?: Reply) => Promise<void>;
}

/**
 * Holds every /api request that `match` accepts until the test releases it.
 * Registered after the stubs, so it takes the request first; a release
 * without a reply passes the request on to them.
 */
export async function holdRequests(
  page: Page,
  match: (url: URL, method: string) => boolean,
): Promise<{ held: HeldRequest[] }> {
  const held: HeldRequest[] = [];
  await page.route(
    (url) => url.pathname.startsWith("/api/"),
    async (route) => {
      const request = route.request();
      if (!match(new URL(request.url()), request.method())) {
        await route.fallback();
        return;
      }
      let decide!: (reply: Reply | undefined) => void;
      const decided = new Promise<Reply | undefined>((resolve) => {
        decide = resolve;
      });
      let finish!: () => void;
      const finished = new Promise<void>((resolve) => {
        finish = resolve;
      });
      held.push({
        path: new URL(request.url()).pathname.replace(/^\/api\/v1/, ""),
        release: async (reply) => {
          decide(reply);
          await finished;
        },
      });
      const reply = await decided;
      try {
        if (reply === undefined) await route.fallback();
        else await answerWith(route, reply);
      } finally {
        finish();
      }
    },
  );
  return { held };
}

export async function waitForHeld(held: HeldRequest[], count: number, what: string) {
  await expect
    .poll(() => held.length, { message: `${what} is sent`, timeout: 15_000 })
    .toBeGreaterThanOrEqual(count);
}

/** Every toast on screen, whatever its text or type; leaving toasts excluded. */
export function liveToasts(page: Page): Locator {
  return page.locator('[data-sonner-toast]:not([data-removed="true"])');
}

interface ToastSeen {
  type: string | null;
  text: string;
}

/** A position in the record of added toasts; pass it to the checks below. */
export async function toastMark(page: Page): Promise<number> {
  return page.evaluate(
    () => (window as unknown as { __toastsAdded: Element[] }).__toastsAdded.length,
  );
}

/** One look at the page: toasts on screen now, and toasts added since `since`. */
export async function toastSnapshot(
  page: Page,
  since: number,
): Promise<{ live: ToastSeen[]; added: ToastSeen[] }> {
  return page.evaluate((from) => {
    const seen = (el: Element) => ({
      type: el.getAttribute("data-type"),
      text: el.textContent ?? "",
    });
    const record = (window as unknown as { __toastsAdded: Element[] })
      .__toastsAdded;
    return {
      live: Array.from(
        document.querySelectorAll('[data-sonner-toast]:not([data-removed="true"])'),
      ).map(seen),
      added: record.slice(from).map(seen),
    };
  }, since);
}

/**
 * Exactly one toast is on screen, it carries `text` and is of `type`
 * (Sonner's `data-type`; `null` for a plain toast), and no other toast was
 * added since `since`. Waits
 * only for a first toast to appear; the counts come from a single look, so a
 * second toast cannot be waited out.
 */
export async function expectOnlyToast(
  page: Page,
  text: string,
  type: "error" | "warning" | null,
  since: number,
) {
  await expect
    .poll(async () => (await toastSnapshot(page, since)).live.length, {
      message: `a "${text}" toast appears`,
      timeout: 5_000,
    })
    .toBeGreaterThan(0);
  await page.waitForTimeout(300);
  const { live, added } = await toastSnapshot(page, since);
  const expected = { type, text: expect.stringContaining(text) };
  expect(live, "toasts on screen").toEqual([expected]);
  expect(added.length, "toasts added since the action").toBeLessThanOrEqual(1);
  for (const toast of added) expect(toast, "toast added since the action").toEqual(expected);
}

/** No toast is on screen, and none was added since `since`. */
export async function expectNoToast(page: Page, since: number) {
  await expect
    .poll(async () => (await toastSnapshot(page, since)).live.length, {
      message: "every toast has gone",
      timeout: 5_000,
    })
    .toBe(0);
  const { added } = await toastSnapshot(page, since);
  expect(added, "toasts added since the action").toEqual([]);
}
