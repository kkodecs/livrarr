import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { toast, Toaster } from "sonner";
import WorkDetailPage from "./WorkDetailPage";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import type { WorkDetailResponse } from "@/types/api";

// The book page is mounted for real with the real `deleteWork` and `apiFetch`;
// only the network is stubbed. A sonner Toaster is mounted beside the page so
// the delete outcome is read from the rendered notifications.

const BOX_LABEL = "Also delete files from disk";
const TITLE = "The Doomed Book";
const UNTICKED = `Delete "${TITLE}" from your library? Its files stay on disk.`;
const TICKED = `Delete "${TITLE}" from your library? Its files will be permanently deleted from disk. This cannot be undone.`;

function makeWork(): WorkDetailResponse {
  return {
    id: 7,
    title: TITLE,
    authorName: "Case Writer",
    identityStatus: "confirmed",
    enrichmentStatus: "enriched",
    enriching: false,
    parkedByConflicts: false,
    olKey: "OL7W",
    hcKey: null,
    grKey: null,
    isbn13: null,
    asin: null,
    libraryItems: [],
    coverManual: false,
    coverSource: null,
    coverMtime: null,
    audiobookCoverUrl: null,
    audiobookCoverSource: null,
    audiobookCoverMtime: null,
    coverUiState: {
      formatNeeded: null,
      ebook: { state: "NowhereToLook" },
      audiobook: { state: "NowhereToLook" },
    },
  } as unknown as WorkDetailResponse;
}

function installBookRoutes(deleteReply: StubReply = { status: 200, body: { warnings: [] } }) {
  return installApiStub((call: ApiCall): StubReply => {
    if (call.method === "GET" && call.path === "/work/7") {
      return { status: 200, body: makeWork() };
    }
    if (call.method === "GET" && call.path === "/work/7/pending-anchors") {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path.startsWith("/queue")) {
      return { status: 200, body: { items: [], total: 0, page: 1, pageSize: 50 } };
    }
    if (call.method === "DELETE" && call.path.startsWith("/work/7")) {
      return deleteReply;
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

async function mountBookPage() {
  const mounted = mountWith(newTestClient(), <WorkDetailPage />, {
    path: "/work/7?tab=metadata",
    route: "/work/:id",
  });
  await vi.waitFor(() =>
    expect(mounted.container.querySelector("h1")?.textContent).toBe(TITLE),
  );
  return mounted;
}

async function openDeleteDialog(container: HTMLElement): Promise<HTMLElement> {
  await click(buttonWithText(container, "Delete"));
  return openDialog();
}

function deleteCalls(calls: ApiCall[]): string[] {
  return calls.filter((c) => c.method === "DELETE").map((c) => `${c.method} ${c.path}`);
}

describe("Book page delete dialog", () => {
  let cleanups: Array<() => void> = [];

  afterEach(() => {
    act(() => {
      toast.dismiss();
    });
    for (const cleanup of cleanups.reverse()) cleanup();
    cleanups = [];
  });

  it("opens unticked with the stay-on-disk text, and ticking or unticking switches the text", async () => {
    const api = installBookRoutes();
    cleanups.push(api.restore);
    const mounted = await mountBookPage();
    cleanups.push(mounted.cleanup);

    const dialog = await openDeleteDialog(mounted.container);
    expect(descriptionText(dialog)).toBe(UNTICKED);
    const box = deleteFilesBox(dialog);
    expect(isTicked(box)).toBe(false);

    await click(box);
    expect(isTicked(deleteFilesBox(openDialog()))).toBe(true);
    expect(descriptionText(openDialog())).toBe(TICKED);

    await click(deleteFilesBox(openDialog()));
    expect(isTicked(deleteFilesBox(openDialog()))).toBe(false);
    expect(descriptionText(openDialog())).toBe(UNTICKED);
  });

  it("cancelling while ticked and reopening shows the box unticked, and confirming sends no parameter", async () => {
    const api = installBookRoutes();
    cleanups.push(api.restore);
    const mounted = await mountBookPage();
    cleanups.push(mounted.cleanup);

    const dialog = await openDeleteDialog(mounted.container);
    await click(deleteFilesBox(dialog));
    expect(isTicked(deleteFilesBox(openDialog()))).toBe(true);
    await click(buttonWithText(openDialog(), "Cancel"));
    expect(document.querySelector('[role="dialog"]')).toBeNull();

    const reopened = await openDeleteDialog(mounted.container);
    expect(isTicked(deleteFilesBox(reopened))).toBe(false);
    expect(descriptionText(reopened)).toBe(UNTICKED);

    await click(buttonWithText(reopened, "Delete"));
    await vi.waitFor(() => expect(deleteCalls(api.calls)).toEqual(["DELETE /work/7"]));
  });

  it("confirming ticked sends deleteFiles=true", async () => {
    const api = installBookRoutes();
    cleanups.push(api.restore);
    const mounted = await mountBookPage();
    cleanups.push(mounted.cleanup);

    const dialog = await openDeleteDialog(mounted.container);
    await click(deleteFilesBox(dialog));
    await click(buttonWithText(openDialog(), "Delete"));
    await vi.waitFor(() =>
      expect(deleteCalls(api.calls)).toEqual(["DELETE /work/7?deleteFiles=true"]),
    );
  });

  it("a reply with one warning shows one warning toast naming the file", async () => {
    const warning = "Case Writer/The Doomed Book.epub: is a link, not a regular file";
    const api = installBookRoutes({ status: 200, body: { warnings: [warning] } });
    cleanups.push(api.restore);
    cleanups.push(mountToaster());
    const mounted = await mountBookPage();
    cleanups.push(mounted.cleanup);

    const dialog = await openDeleteDialog(mounted.container);
    await click(deleteFilesBox(dialog));
    await click(buttonWithText(openDialog(), "Delete"));
    await vi.waitFor(() =>
      expect(deleteCalls(api.calls)).toEqual(["DELETE /work/7?deleteFiles=true"]),
    );
    await vi.waitFor(() =>
      expect(document.querySelectorAll("[data-sonner-toast]").length).toBeGreaterThan(0),
    );
    // Let any second pop-up for the same delete render before counting.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    const shown = document.querySelectorAll("[data-sonner-toast]");
    expect(shown).toHaveLength(1);
    expect(shown[0]!.getAttribute("data-type")).toBe("warning");
    expect(shown[0]!.textContent).toContain("Case Writer/The Doomed Book.epub");
  });
});
