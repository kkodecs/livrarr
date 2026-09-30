import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { toast, Toaster } from "sonner";
import { LibraryFilesTab } from "./LibraryFilesTab";
import {
  clickTitled,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import type { WorkDetailResponse } from "@/types/api";

// The real files tab runs the real `deleteLibraryFile` and `apiFetch`; only
// the network is stubbed. A sonner Toaster is mounted beside the tab so the
// delete outcome is read from the rendered notifications.

const ITEM_PATH = "Tab Writer/Tab Book.epub";

function makeWork(): WorkDetailResponse {
  return {
    id: 7,
    title: "Tab Book",
    authorName: "Tab Writer",
    libraryItems: [
      {
        id: 21,
        path: ITEM_PATH,
        mediaType: "ebook",
        fileSize: 2048,
        importedAt: "2026-09-01T00:00:00Z",
        progressPct: null,
        durationSeconds: null,
        finishedAt: null,
      },
    ],
  } as unknown as WorkDetailResponse;
}

function installFileRoutes(deleteReply: StubReply) {
  return installApiStub((call: ApiCall): StubReply => {
    if (call.method === "DELETE" && call.path.startsWith("/workfile/")) {
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

function openDialog(): HTMLElement {
  const dialog = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!dialog) throw new Error("the delete dialog is not open");
  return dialog;
}

function descriptionText(dialog: HTMLElement): string | null {
  const id = dialog.getAttribute("aria-describedby");
  return id ? (document.getElementById(id)?.textContent ?? null) : null;
}

async function confirmDelete(dialog: HTMLElement) {
  const confirm = Array.from(dialog.querySelectorAll("button")).find(
    (b) => b.textContent?.trim() === "Delete",
  );
  if (!confirm) throw new Error("no Delete button in the dialog");
  await click(confirm);
}

function toasts(type: string): Element[] {
  return Array.from(
    document.querySelectorAll(`[data-sonner-toast][data-type="${type}"]`),
  );
}

describe("Files tab Delete File", () => {
  let cleanups: Array<() => void> = [];

  afterEach(() => {
    act(() => {
      toast.dismiss();
    });
    for (const cleanup of cleanups.reverse()) cleanup();
    cleanups = [];
  });

  function mountTab() {
    const mounted = mountWith(newTestClient(), <LibraryFilesTab work={makeWork()} />);
    cleanups.push(mounted.cleanup);
    return mounted;
  }

  it("says the file is deleted from disk and sends exactly DELETE /workfile/21", async () => {
    const api = installFileRoutes({ status: 200 });
    cleanups.push(api.restore);
    const mounted = mountTab();

    await clickTitled(mounted.container, "Delete file");
    expect(descriptionText(openDialog())).toBe(
      "Delete this file from disk? This cannot be undone.",
    );
    await confirmDelete(openDialog());
    await vi.waitFor(() =>
      expect(
        api.calls.filter((c) => c.method !== "GET").map((c) => `${c.method} ${c.path}`),
      ).toEqual(["DELETE /workfile/21"]),
    );
  });

  it("a 409 reply shows exactly one error pop-up with the server's message", async () => {
    const message = `${ITEM_PATH}: is a link, not a regular file`;
    const api = installFileRoutes({
      status: 409,
      body: { status: 409, error: "conflict", message },
    });
    cleanups.push(api.restore);
    cleanups.push(mountToaster());
    const mounted = mountTab();

    await clickTitled(mounted.container, "Delete file");
    await confirmDelete(openDialog());
    await vi.waitFor(() => expect(toasts("error").length).toBeGreaterThan(0));
    // Let any second pop-up for the same failure render before counting.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    const errors = toasts("error");
    expect(errors).toHaveLength(1);
    expect(errors[0]!.textContent).toContain(message);
  });

  it("a 200 reply shows File deleted", async () => {
    const api = installFileRoutes({ status: 200 });
    cleanups.push(api.restore);
    cleanups.push(mountToaster());
    const mounted = mountTab();

    await clickTitled(mounted.container, "Delete file");
    await confirmDelete(openDialog());
    await vi.waitFor(() => {
      const shown = toasts("success");
      expect(shown).toHaveLength(1);
      expect(shown[0]!.textContent).toContain("File deleted");
    });
  });
});
