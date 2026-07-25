import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { IdentityEditModal } from "./IdentityEditModal";
import type { IdentityPreviewResponse } from "@/types/api";

/**
 * Design: docs/design-identity-edit.md r4 §Frontend, F6 in
 * docs/design-identity-edit-fixes.md.
 *
 * These drive the real modal against the real @/api seam — only the network boundary is
 * stubbed, exactly as HistoryPage.test.tsx does. The properties under test are the ones
 * the user's safety depends on: a collision must never offer a confirm button, an
 * unverifiable provider must never look certifiable, and a stale preview must send the
 * user back to a fresh preview rather than silently failing.
 */

const previewIdentityEdit = vi.fn();
const commitIdentityEdit = vi.fn();

vi.mock("@/api", () => ({
  previewIdentityEdit: (...args: unknown[]) => previewIdentityEdit(...args),
  commitIdentityEdit: (...args: unknown[]) => commitIdentityEdit(...args),
  clearIdentitySlot: vi.fn(),
  getWork: vi.fn().mockResolvedValue({}),
}));

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

function certifiablePreview(): IdentityPreviewResponse {
  return {
    previewId: "token-1",
    resolved: {
      slot: "gr_work",
      canonicalValue: "12345",
      title: "The Right Book",
      author: "Case Writer",
      year: 2019,
    },
    collision: null,
    siblings: [],
    bridgeWarnings: [],
    conflictWarning: false,
    reason: null,
  } as unknown as IdentityPreviewResponse;
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  previewIdentityEdit.mockReset();
  commitIdentityEdit.mockReset();
  container = document.createElement("div");
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(slot: "gr_work" | null = "gr_work") {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  act(() => {
    root.render(
      <QueryClientProvider client={queryClient}>
        <IdentityEditModal workId={7} slot={slot} onClose={() => {}} />
      </QueryClientProvider>,
    );
  });
}

/** Type into the identifier box and press Preview. */
async function preview(value: string) {
  const input = container.querySelector("input") as HTMLInputElement;
  const setter = Object.getOwnPropertyDescriptor(
    window.HTMLInputElement.prototype,
    "value",
  )!.set!;
  await act(async () => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
  const previewButton = [...container.querySelectorAll("button")].find((b) =>
    b.textContent?.includes("Preview"),
  )!;
  await act(async () => {
    previewButton.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function confirmButton(): HTMLButtonElement {
  return [...container.querySelectorAll("button")].find((b) =>
    b.textContent?.includes("This is the right book"),
  )! as HTMLButtonElement;
}

describe("IdentityEditModal", () => {
  it("enables confirm only once the provider certified the identifier", async () => {
    previewIdentityEdit.mockResolvedValue(certifiablePreview());
    render();

    expect(confirmButton().disabled).toBe(true);

    await preview("12345");

    expect(container.textContent).toContain("The Right Book");
    expect(container.textContent).toContain("12345");
    expect(confirmButton().disabled).toBe(false);
  });

  it("offers no way to confirm an identifier that belongs to another book", async () => {
    previewIdentityEdit.mockResolvedValue({
      ...certifiablePreview(),
      previewId: null,
      collision: { owningWorkId: 42, owningWorkTitle: "The Other Book" },
    });
    render();

    await preview("12345");

    expect(container.textContent).toContain("The Other Book");
    expect(confirmButton().disabled).toBe(true);
    expect(commitIdentityEdit).not.toHaveBeenCalled();
  });

  it("will not confirm a resolved book the server issued no preview token for", async () => {
    // The server resolved the identifier but declined to store a snapshot, so there is
    // nothing for commit to consume. Without the token check the modal would happily
    // send a commit that can only fail — the confirm button must stay dead.
    previewIdentityEdit.mockResolvedValue({
      ...certifiablePreview(),
      previewId: null,
    });
    render();

    await preview("12345");

    expect(container.textContent).toContain("The Right Book");
    expect(confirmButton().disabled).toBe(true);
    expect(commitIdentityEdit).not.toHaveBeenCalled();
  });

  it("says nothing is certifiable when the provider could not be reached", async () => {
    previewIdentityEdit.mockResolvedValue({
      previewId: null,
      resolved: null,
      collision: null,
      siblings: [],
      bridgeWarnings: [],
      conflictWarning: false,
      reason: "unavailable",
    } as unknown as IdentityPreviewResponse);
    render();

    await preview("12345");

    expect(container.textContent).toContain("couldn't be reached");
    expect(confirmButton().disabled).toBe(true);
  });

  it("distinguishes a not-found identifier from an unreachable provider", async () => {
    previewIdentityEdit.mockResolvedValue({
      previewId: null,
      resolved: null,
      collision: null,
      siblings: [],
      bridgeWarnings: [],
      conflictWarning: false,
      reason: "not_found",
    } as unknown as IdentityPreviewResponse);
    render();

    await preview("12345");

    expect(container.textContent).toContain("No book was found");
    expect(confirmButton().disabled).toBe(true);
  });

  it("sends the user back to a fresh preview when the snapshot went stale", async () => {
    previewIdentityEdit.mockResolvedValue(certifiablePreview());
    const stale = Object.assign(new Error("stale"), {
      details: { code: "preview_required" },
    });
    Object.setPrototypeOf(stale, (await import("@/api/client")).ApiError.prototype);
    commitIdentityEdit.mockRejectedValue(stale);
    render();

    await preview("12345");
    await act(async () => {
      confirmButton().dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(container.textContent).toContain("changed while you were looking");
    // The certified card is withdrawn — confirming again without a new preview
    // must not be possible.
    expect(confirmButton().disabled).toBe(true);
  });

  it("warns that dropped identifiers are re-matched, not destroyed", async () => {
    previewIdentityEdit.mockResolvedValue({
      ...certifiablePreview(),
      siblings: [
        { slot: "ol_work", value: "OL1W", action: "drop", cause: "unverifiable" },
      ],
    });
    render();

    await preview("12345");

    expect(container.textContent).toContain("aren't destroyed");
  });
});
