import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { toast, Toaster } from "sonner";
import { WorksPage } from "./WorksPage";
import {
  clickTitled,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import type { WorkDetailResponse } from "@/types/api";

// The Books page is mounted for real with the real `deleteWork` and
// `apiFetch`; only the network is stubbed. Books are selected through the
// page's own editor mode and row checkboxes. A sonner Toaster is mounted beside
// the page so the bulk-delete summary is read from the rendered notifications.

const BOX_LABEL = "Also delete files from disk";
const unticked = (n: number) => `Delete ${n} works from your library? Their files stay on disk.`;
const ticked = (n: number) =>
  `Delete ${n} works from your library? Their files will be permanently deleted from disk. This cannot be undone.`;

const BOOKS: Array<[number, string]> = [
  [11, "First Shelf Book"],
  [12, "Second Shelf Book"],
  [13, "Third Shelf Book"],
  [14, "Fourth Shelf Book"],
];

function makeWork(id: number, title: string): WorkDetailResponse {
  return {
    id,
    title,
    authorName: "Shelf Writer",
    authorId: null,
    year: null,
    language: "en",
    monitorEbook: false,
    monitorAudiobook: false,
    enrichmentStatus: "enriched",
    enriching: false,
    libraryItems: [],
    coverMtime: null,
    addedAt: "2026-09-01T00:00:00Z",
  } as unknown as WorkDetailResponse;
}

type DeleteReplies = Record<number, StubReply>;

/** Serves the shelf; a book whose delete succeeded leaves later listings. */
function installShelfRoutes(replies: DeleteReplies = {}) {
  const deleted = new Set<number>();
  return installApiStub((call: ApiCall): StubReply => {
    const path = call.path.split("?")[0] ?? "";
    if (call.method === "GET" && path === "/work") {
      const shelf = BOOKS.filter(([id]) => !deleted.has(id));
      return {
        status: 200,
        body: {
          items: shelf.map(([id, title]) => makeWork(id, title)),
          total: shelf.length,
          page: 1,
          pageSize: 50,
        },
      };
    }
    if (call.method === "GET" && path === "/queue") {
      return { status: 200, body: { items: [], total: 0, page: 1, pageSize: 50 } };
    }
    const deleting = call.method === "DELETE" ? /^\/work\/(\d+)$/.exec(path) : null;
    if (deleting) {
      const id = Number(deleting[1]);
      const reply = replies[id] ?? { status: 200, body: { warnings: [] } };
      if (reply.status === 200) deleted.add(id);
      return reply;
    }
    return {
      status: 404,
      body: { error: "not_found", message: `unstubbed ${call.path}`, status: 404 },
    };
  });
}

function mountToaster() {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(<Toaster />);
  });
  return () => {
    act(() => root.unmount());
    container.remove();
  };
}

async function click(el: Element) {
  await act(async () => {
    el.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function buttonWithText(scope: ParentNode, text: string): HTMLButtonElement {
  const button = Array.from(scope.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === text,
  );
  if (!button) throw new Error(`no button with text "${text}"`);
  return button;
}

function openDialog(): HTMLElement {
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!dialog) throw new Error("the delete dialog is not open");
  return dialog;
}

/** The dialog's description element, read apart from the box and its label. */
function descriptionText(dialog: HTMLElement): string | null {
  const id = dialog.getAttribute("aria-describedby");
  return id ? (document.getElementById(id)?.textContent ?? null) : null;
}

function labelText(el: HTMLElement): string {
  const parts: string[] = [];
  parts.push(el.getAttribute("aria-label") ?? "");
  const labelledBy = el.getAttribute("aria-labelledby");
  if (labelledBy) {
    for (const id of labelledBy.split(/\s+/)) {
      parts.push(document.getElementById(id)?.textContent ?? "");
    }
  }
  if (el instanceof HTMLInputElement) {
    for (const label of Array.from(el.labels ?? [])) parts.push(label.textContent ?? "");
  }
  parts.push(el.closest("label")?.textContent ?? "");
  if (el.id) parts.push(document.querySelector(`label[for="${el.id}"]`)?.textContent ?? "");
  return parts.join(" ");
}

function deleteFilesBox(dialog: HTMLElement): HTMLElement {
  const candidates = Array.from(
    dialog.querySelectorAll<HTMLElement>('input[type="checkbox"], [role="checkbox"]'),
  ).filter((el) => labelText(el).includes(BOX_LABEL));
  if (candidates.length !== 1) {
    throw new Error(`expected one "${BOX_LABEL}" box in the dialog, found ${candidates.length}`);
  }
  return candidates[0]!;
}

function isTicked(box: HTMLElement): boolean {
  return box instanceof HTMLInputElement
    ? box.checked
    : box.getAttribute("aria-checked") === "true";
}

async function mountShelf() {
  const mounted = mountWith(newTestClient(), <WorksPage />, { path: "/" });
  await vi.waitFor(() =>
    expect(mounted.container.textContent).toContain("First Shelf Book"),
  );
  await clickTitled(mounted.container, "Table");
  await clickTitled(mounted.container, "Toggle editor mode");
  return mounted;
}

/** Ticks the row checkbox of the book with this title. */
async function selectBook(container: HTMLElement, title: string) {
  const row = Array.from(container.querySelectorAll("tbody tr")).find((tr) =>
    tr.textContent?.includes(title),
  );
  if (!row) throw new Error(`no row for "${title}"`);
  const checkbox = row.querySelector("td button");
  if (!checkbox) throw new Error(`no selection checkbox for "${title}"`);
  await click(checkbox);
}

async function openDeleteDialog(container: HTMLElement): Promise<HTMLElement> {
  await click(buttonWithText(container, "Delete Selected"));
  return openDialog();
}

function deleteCalls(calls: ApiCall[]): string[] {
  return calls
    .filter((c) => c.method === "DELETE")
    .map((c) => `${c.method} ${c.path}`)
    .sort();
}

describe("Books page bulk delete dialog", () => {
  let cleanups: Array<() => void> = [];

  beforeEach(() => {
    localStorage.clear();
  });

  afterEach(() => {
    act(() => {
      toast.dismiss();
    });
    for (const cleanup of cleanups.reverse()) cleanup();
    cleanups = [];
    localStorage.clear();
  });

  it("opens unticked with the stay-on-disk text, and ticking or unticking switches the text", async () => {
    const api = installShelfRoutes();
    cleanups.push(api.restore);
    const mounted = await mountShelf();
    cleanups.push(mounted.cleanup);
    await selectBook(mounted.container, "First Shelf Book");
    await selectBook(mounted.container, "Second Shelf Book");

    const dialog = await openDeleteDialog(mounted.container);
    expect(descriptionText(dialog)).toBe(unticked(2));
    expect(isTicked(deleteFilesBox(dialog))).toBe(false);

    await click(deleteFilesBox(openDialog()));
    expect(isTicked(deleteFilesBox(openDialog()))).toBe(true);
    expect(descriptionText(openDialog())).toBe(ticked(2));

    await click(deleteFilesBox(openDialog()));
    expect(isTicked(deleteFilesBox(openDialog()))).toBe(false);
    expect(descriptionText(openDialog())).toBe(unticked(2));
  });

  it("cancelling while ticked and reopening shows the box unticked, and confirming sends no parameter", async () => {
    const api = installShelfRoutes();
    cleanups.push(api.restore);
    const mounted = await mountShelf();
    cleanups.push(mounted.cleanup);
    await selectBook(mounted.container, "First Shelf Book");
    await selectBook(mounted.container, "Second Shelf Book");

    const dialog = await openDeleteDialog(mounted.container);
    await click(deleteFilesBox(dialog));
    expect(isTicked(deleteFilesBox(openDialog()))).toBe(true);
    await click(buttonWithText(openDialog(), "Cancel"));
    expect(document.querySelector('[role="dialog"]')).toBeNull();

    const reopened = await openDeleteDialog(mounted.container);
    expect(isTicked(deleteFilesBox(reopened))).toBe(false);
    expect(descriptionText(reopened)).toBe(unticked(2));

    await click(buttonWithText(reopened, "Delete"));
    await vi.waitFor(() =>
      expect(deleteCalls(api.calls)).toEqual(["DELETE /work/11", "DELETE /work/12"]),
    );
  });

  it("confirming ticked sends deleteFiles=true for every chosen book, and a new selection opens unticked", async () => {
    const api = installShelfRoutes();
    cleanups.push(api.restore);
    const mounted = await mountShelf();
    cleanups.push(mounted.cleanup);
    await selectBook(mounted.container, "First Shelf Book");
    await selectBook(mounted.container, "Second Shelf Book");

    const dialog = await openDeleteDialog(mounted.container);
    await click(deleteFilesBox(dialog));
    await click(buttonWithText(openDialog(), "Delete"));
    await vi.waitFor(() =>
      expect(deleteCalls(api.calls)).toEqual([
        "DELETE /work/11?deleteFiles=true",
        "DELETE /work/12?deleteFiles=true",
      ]),
    );
    await vi.waitFor(() => expect(document.querySelector('[role="dialog"]')).toBeNull());

    await vi.waitFor(() =>
      expect(mounted.container.textContent).not.toContain("First Shelf Book"),
    );
    await selectBook(mounted.container, "Third Shelf Book");
    await selectBook(mounted.container, "Fourth Shelf Book");
    const next = await openDeleteDialog(mounted.container);
    expect(isTicked(deleteFilesBox(next))).toBe(false);
    expect(descriptionText(next)).toBe(unticked(2));
  });

  it("gathers file warnings from the successful deletes into the one summary", async () => {
    const warnings11 = [
      "Shelf Writer/First/a.epub: is a link, not a regular file",
      "Shelf Writer/First/b.epub: is a link, not a regular file",
      "Shelf Writer/First/c.epub: is a link, not a regular file",
    ];
    const warnings12 = [
      "Shelf Writer/Second/d.epub: is a link, not a regular file",
      "Shelf Writer/Second/e.epub: is a link, not a regular file",
      "Shelf Writer/Second/f.epub: is a link, not a regular file",
      "Shelf Writer/Second/g.epub: is a link, not a regular file",
    ];
    const api = installShelfRoutes({
      11: { status: 200, body: { warnings: warnings11 } },
      12: { status: 200, body: { warnings: warnings12 } },
      13: {
        status: 500,
        body: { error: "internal", message: "delete failed", status: 500 },
      },
    });
    cleanups.push(api.restore);
    cleanups.push(mountToaster());
    const mounted = await mountShelf();
    cleanups.push(mounted.cleanup);
    for (const title of ["First Shelf Book", "Second Shelf Book", "Third Shelf Book"]) {
      await selectBook(mounted.container, title);
    }

    const dialog = await openDeleteDialog(mounted.container);
    await click(deleteFilesBox(dialog));
    await click(buttonWithText(openDialog(), "Delete"));
    await vi.waitFor(() => expect(deleteCalls(api.calls)).toHaveLength(3));

    await vi.waitFor(() => {
      const toasts = document.querySelectorAll("[data-sonner-toast]");
      expect(toasts).toHaveLength(1);
      const summary = toasts[0]!;
      expect(summary.getAttribute("data-type")).toBe("warning");
      const text = summary.textContent ?? "";
      expect(text).toContain("Deleted 2, failed 1 of 3 works");
      const listed = [...warnings11, ...warnings12].filter((w) => text.includes(w));
      expect(listed).toHaveLength(5);
      expect(text).toContain("and 2 more");
    });
  });
});
