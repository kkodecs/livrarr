import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { toast, Toaster } from "sonner";
import { Route, Routes } from "react-router";
import WorkDetailPage from "./WorkDetailPage";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import {
  AppToaster,
  recordAddedToasts,
  toastSummary,
} from "@/test-support/toasts";
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

type DeleteReply = StubReply | "network-error" | Promise<StubReply>;

function installBookRoutes(
  deleteReply: () => DeleteReply = () => ({ status: 200, body: { warnings: [] } }),
) {
  return installApiStub(async (call: ApiCall): Promise<StubReply> => {
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
      const reply = deleteReply();
      if (reply === "network-error") throw new TypeError("Failed to fetch");
      return reply;
    }
    return {
      status: 404,
      body: { error: "not_found", message: `unstubbed ${call.path}`, status: 404 },
    };
  });
}

function mountToaster(toaster: ReactNode = <Toaster />) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(toaster);
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

const HOME = "home page";

async function mountBookPage() {
  const mounted = mountWith(
    newTestClient(),
    <Routes>
      <Route path="/work/:id" element={<WorkDetailPage />} />
      <Route path="/" element={<p data-testid="home">{HOME}</p>} />
    </Routes>,
    { path: "/work/7?tab=metadata", route: "*" },
  );
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
    const api = installBookRoutes(() => ({ status: 200, body: { warnings: [warning] } }));
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

const serverError = (status: number, error: string, message: string): StubReply => ({
  status,
  body: { status, error, message },
});

/** A reply the stub never answers by itself; the test answers it later. */
function heldReply() {
  let answer: (reply: StubReply) => void = () => {};
  const reply = new Promise<StubReply>((resolve) => {
    answer = resolve;
  });
  return { reply, answer };
}

function isHome(container: HTMLElement): boolean {
  return container.querySelector('[data-testid="home"]') !== null;
}

/**
 * Waits for a first toast, lets a second one for the same delete render,
 * then returns what is on screen and everything added since `recorder` began.
 */
async function toastsAfterDelete(recorder: ReturnType<typeof recordAddedToasts>) {
  await vi.waitFor(() => expect(recorder.added().length).toBeGreaterThan(0));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 300));
  });
  return {
    live: toastSummary(),
    added: recorder.added().map((el) => ({
      type: el.getAttribute("data-type"),
      text: el.textContent ?? "",
    })),
  };
}

describe("Book page delete: outcome pop-ups", () => {
  let cleanups: Array<() => void> = [];

  afterEach(() => {
    act(() => {
      toast.dismiss();
    });
    for (const cleanup of cleanups.reverse()) cleanup();
    cleanups = [];
  });

  /** Mounts the page and the app's toaster, opens the dialog and presses Delete. */
  async function pressDelete(deleteReply: () => DeleteReply) {
    const api = installBookRoutes(deleteReply);
    cleanups.push(api.restore);
    cleanups.push(mountToaster(<AppToaster />));
    const mounted = await mountBookPage();
    cleanups.push(mounted.cleanup);
    const recorder = recordAddedToasts();
    cleanups.push(recorder.stop);
    const dialog = await openDeleteDialog(mounted.container);
    await click(buttonWithText(dialog, "Delete"));
    await vi.waitFor(() => expect(deleteCalls(api.calls)).toEqual(["DELETE /work/7"]));
    return { api, mounted, recorder };
  }

  const onlyError = (text: string) => [{ type: "error", text: expect.stringContaining(text) }];

  const failures: Array<[string, () => DeleteReply, string]> = [
    ["a server error", () => serverError(500, "internal", "Something went wrong"), "Something went wrong"],
    ["a network failure", () => "network-error", "Unable to reach Livrarr"],
    ["a 404", () => serverError(404, "not_found", "not found"), "not found"],
  ];

  for (const [shape, reply, message] of failures) {
    it(`${shape}: exactly one error pop-up with the server's message, and the dialog stays open`, async () => {
      const { mounted, recorder } = await pressDelete(reply);
      const { live, added } = await toastsAfterDelete(recorder);
      expect(live).toEqual(onlyError(message));
      expect(added).toEqual(onlyError(message));
      expect(document.body.textContent).not.toContain("Failed to delete work");
      expect(document.querySelector('[role="dialog"]')).not.toBeNull();
      expect(isHome(mounted.container)).toBe(false);
    });
  }

  it("a slow reply that fails after the user cancelled the dialog: exactly one error pop-up", async () => {
    const held = heldReply();
    const { recorder } = await pressDelete(() => held.reply);
    await click(buttonWithText(openDialog(), "Cancel"));
    expect(document.querySelector('[role="dialog"]')).toBeNull();
    expect(recorder.added()).toEqual([]);

    await act(async () => {
      held.answer(serverError(500, "internal", "Something went wrong"));
    });
    const { live, added } = await toastsAfterDelete(recorder);
    expect(live).toEqual(onlyError("Something went wrong"));
    expect(added).toEqual(onlyError("Something went wrong"));
  });

  const deleted: Array<[string, StubReply]> = [
    ["{ warnings: [] }", { status: 200, body: { warnings: [] } }],
    ["no body", { status: 200 }],
    ["{}", { status: 200, body: {} }],
    ["null", { status: 200, body: null }],
    ['{ warnings: "x" }', { status: 200, body: { warnings: "x" } }],
    ["a body that is not JSON", { status: 200, rawBody: "<html>deleted</html>" }],
  ];

  for (const [shape, reply] of deleted) {
    it(`a 200 answering ${shape}: one "Work deleted", no error, and the page moves home`, async () => {
      const { mounted, recorder } = await pressDelete(() => reply);
      const { live, added } = await toastsAfterDelete(recorder);
      const deletedToast = [{ type: "success", text: expect.stringContaining("Work deleted") }];
      expect(live).toEqual(deletedToast);
      expect(added).toEqual(deletedToast);
      await vi.waitFor(() => expect(isHome(mounted.container)).toBe(true));
    });
  }
});
