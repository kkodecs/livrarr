import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import WorkDetailPage from "../WorkDetailPage";
import {
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
  type StubReply,
} from "@/test-support/apiStub";
import type { WorkDetailResponse } from "@/types/api";

afterEach(() => vi.restoreAllMocks());

const work = {
  id: 7,
  title: "The Current Book",
  authorName: "Case Writer",
  seriesName: null,
  seriesPosition: null,
  monitorEbook: true,
  monitorAudiobook: false,
  identityStatus: "confirmed",
  enrichmentStatus: "enriched",
  enriching: false,
  parkedByConflicts: false,
  olKey: null,
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

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  if (!setter) throw new Error("HTMLInputElement value setter missing");
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function field(name: string): HTMLInputElement {
  const input = document.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  if (!input) throw new Error(`no input named ${name}`);
  return input;
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(document.querySelectorAll("button")).find(
    (candidate) => candidate.textContent?.trim() === label,
  );
  if (!found) throw new Error(`no button labelled ${label}`);
  return found;
}

async function openEditor(putReply: StubReply) {
  const api = installApiStub((call: ApiCall) => {
    if (call.method === "GET" && call.path === "/work/7") {
      return { status: 200, body: work };
    }
    if (call.method === "GET" && /\/work\/\d+\/pending-anchors$/.test(call.path)) {
      return { status: 200, body: [] };
    }
    if (call.method === "GET" && call.path.startsWith("/queue")) {
      return { status: 200, body: { items: [], total: 0, page: 1, pageSize: 50 } };
    }
    if (call.method === "PUT" && call.path === "/work/7") {
      return putReply;
    }
    throw new Error(`unexpected call ${call.method} ${call.path}`);
  });
  const mounted = mountWith(newTestClient(), <WorkDetailPage />, {
    path: "/work/7?tab=metadata",
    route: "/work/:id",
  });
  await vi.waitFor(() => expect(mounted.container.textContent).toContain("The Current Book"));
  await act(async () => button("Edit").click());
  return { api, mounted };
}

describe("Edit dialog title and author", () => {
  it("edits title and author and saves every changed field in one request", async () => {
    const { api, mounted } = await openEditor({ status: 200, body: work });
    try {
      expect(field("title").readOnly).toBe(false);
      expect(field("authorName").readOnly).toBe(false);
      expect(document.body.textContent).not.toContain("temporarily unavailable");

      setInput(field("title"), "The Renamed Book");
      setInput(field("seriesName"), "The Saga");
      await act(async () => field("monitorEbook").click());
      await act(async () => button("Save").click());

      await vi.waitFor(() =>
        expect(api.calls.some((call) => call.method === "PUT")).toBe(true),
      );
      const put = api.calls.find((call) => call.method === "PUT");
      expect(put?.path).toBe("/work/7");
      expect(put?.body).toMatchObject({
        title: "The Renamed Book",
        authorName: "Case Writer",
        seriesName: "The Saga",
        monitorEbook: false,
      });
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("shows a duplicate-identity refusal in the dialog and stays open", async () => {
    const message = "Another book already has this title and author.";
    const { api, mounted } = await openEditor({
      status: 409,
      body: { error: "conflict", message, status: 409 },
    });
    try {
      setInput(field("title"), "Another Book");
      await act(async () => button("Save").click());

      await vi.waitFor(() => expect(document.body.textContent).toContain(message));
      expect(field("title").value).toBe("Another Book");
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});
