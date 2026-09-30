import { expect, test, type Page } from "@playwright/test";
import {
  ok,
  sampleBook,
  serverError,
  servePdfWorker,
  stubApi,
  expectOnlyToast,
  liveToasts,
  toastMark,
  type ApiCall,
  type Reply,
} from "./stubbedApi";

const bookmark = {
  id: 7,
  libraryItemId: 1,
  mediaType: "ebook",
  position: "epubcfi(/6/4!/4/2/1:0)",
  sortKey: 0.12,
  name: "Twelve percent",
  chapterTitle: null,
  pairedBookmarkId: null,
  createdAt: "2026-09-01T00:00:00Z",
};

function libraryItem(id: number, file: string) {
  return {
    id,
    path: `Sample Author/${file}`,
    mediaType: "ebook",
    fileSize: 1000,
    importedAt: "2026-09-01T00:00:00Z",
    progressPct: null,
    durationSeconds: null,
    finishedAt: null,
  };
}

/** The reads a reader makes when it opens item 1; `download` serves the file. */
function readerReads(
  file: "sample.epub" | "sample.pdf",
  download: () => Reply,
): (call: ApiCall) => Reply | undefined {
  return (call) => {
    const bare = call.path.split("?")[0];
    if (call.method !== "GET") return undefined;
    if (bare === "/workfile/1") return ok(libraryItem(1, file));
    if (bare === "/workfile/1/download") return download();
    if (bare === "/workfile/1/bookmarks") return ok([bookmark]);
    if (bare === "/workfile/1/progress")
      return { status: 404, json: { status: 404, error: "not_found", message: "none" } };
    if (bare === "/workfile/1/cross-format/anchors")
      return { status: 404, json: { status: 404, error: "not_found", message: "none" } };
    return undefined;
  };
}

function downloads(calls: ApiCall[]) {
  return calls.filter((c) => c.path === "/workfile/1/download").length;
}

const failures: Array<[string, Reply]> = [
  ["a 500", serverError("download failed")],
  ["a network rejection", "network-error"],
];

test.describe("Opening a book whose download fails", () => {
  for (const [label, failure] of failures) {
    test(`EPUB: ${label} shows the load error and Retry, and Retry opens the book`, async ({
      page,
    }) => {
      let download: Reply = failure;
      const calls = await stubApi(
        page,
        readerReads("sample.epub", () => download),
      );
      await page.goto("/read/1");
      await expect.poll(() => downloads(calls)).toBeGreaterThan(0);

      await expect(page.getByText("Could not load this book.")).toBeVisible({
        timeout: 5_000,
      });
      await expect(page.getByText("Loading...")).toHaveCount(0);

      download = {
        status: 200,
        bytes: sampleBook("sample.epub"),
        contentType: "application/epub+zip",
      };
      const before = downloads(calls);
      await page.getByRole("button", { name: "Retry" }).click();
      await expect.poll(() => downloads(calls)).toBeGreaterThan(before);
      await expect(page.locator('button[title="Add bookmark"]')).toBeVisible();
      await expect(page.getByText("Could not load this book.")).toHaveCount(0);
    });

    // Download-error and refetch only: whether the PDF then opens is outside
    // this test.
    test(`PDF: ${label} shows the load error and Retry, and Retry refetches the download and clears the error`, async ({
      page,
    }) => {
      let download: Reply = failure;
      await servePdfWorker(page);
      const calls = await stubApi(
        page,
        readerReads("sample.pdf", () => download),
      );
      await page.goto("/read/1");
      await expect.poll(() => downloads(calls)).toBeGreaterThan(0);

      await expect(page.getByText("Could not load this book.")).toBeVisible({
        timeout: 5_000,
      });
      await expect(page.getByText("Failed to load PDF.")).toHaveCount(0);
      await expect(page.getByText("No PDF file specified.")).toHaveCount(0);

      download = {
        status: 200,
        bytes: sampleBook("sample.pdf"),
        contentType: "application/pdf",
      };
      const before = downloads(calls);
      await page.getByRole("button", { name: "Retry" }).click();
      await expect.poll(() => downloads(calls)).toBeGreaterThan(before);
      await expect(page.getByText("Could not load this book.")).toHaveCount(0);
    });
  }
});

test.describe("A rejected EPUB bookmark action", () => {
  async function openBook(page: Page, write: (call: ApiCall) => Reply | undefined) {
    const reads = readerReads("sample.epub", () => ({
      status: 200,
      bytes: sampleBook("sample.epub"),
      contentType: "application/epub+zip",
    }));
    const calls = await stubApi(page, (call) => reads(call) ?? write(call));
    await page.goto("/read/1");
    await expect(page.locator('button[title="Add bookmark"]')).toBeVisible();
    await expect(page.getByText("1 bookmarks")).toBeVisible();
    await expect(liveToasts(page)).toHaveCount(0);
    return calls;
  }

  function sent(calls: ApiCall[], method: string, path: string) {
    return calls.filter((c) => c.method === method && c.path === path).length;
  }

  test("adding shows one error toast", async ({ page }) => {
    const calls = await openBook(page, (call) =>
      call.method === "POST" && call.path === "/workfile/1/bookmarks"
        ? serverError("bookmark write failed")
        : undefined,
    );
    const mark = await toastMark(page);
    await page.locator('button[title="Add bookmark"]').click();
    await expect.poll(() => sent(calls, "POST", "/workfile/1/bookmarks")).toBe(1);
    await page.waitForTimeout(500);
    await expectOnlyToast(page, "Could not add the bookmark", "error", mark);
  });

  test("renaming shows one error toast", async ({ page }) => {
    const calls = await openBook(page, (call) =>
      call.method === "PATCH" && call.path === "/bookmarks/7"
        ? serverError("bookmark rename failed")
        : undefined,
    );
    await page.locator('button[title="Bookmarks"]').click();
    await page.locator('button[title="Rename"]').click();
    const input = page.locator("form input");
    await input.fill("Chapter two");
    const mark = await toastMark(page);
    await input.press("Enter");
    await expect.poll(() => sent(calls, "PATCH", "/bookmarks/7")).toBe(1);
    await page.waitForTimeout(500);
    await expectOnlyToast(page, "Could not rename the bookmark", "error", mark);
  });

  test("deleting shows one error toast", async ({ page }) => {
    const calls = await openBook(page, (call) =>
      call.method === "DELETE" && call.path === "/bookmarks/7"
        ? serverError("bookmark delete failed")
        : undefined,
    );
    await page.locator('button[title="Bookmarks"]').click();
    const row = page
      .locator("div.group")
      .filter({ hasText: "Twelve percent" });
    const mark = await toastMark(page);
    await row.locator("button").last().click();
    await expect.poll(() => sent(calls, "DELETE", "/bookmarks/7")).toBe(1);
    await page.waitForTimeout(500);
    await expectOnlyToast(page, "Could not delete the bookmark", "error", mark);
  });
});
