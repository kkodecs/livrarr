import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import ManualImportPage from "@/pages/manual-import/ManualImportPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";

const sourcePath = "/incoming/Case Author - Book Tail.epub";
const ISBN_FIXTURE = "9780306406157";
const singularDefer =
  'Not imported: you already have "Book" by Case Author. Choose Edit title and author for this row. To add this file to that book, enter exactly "Book" and "Case Author", then retry. To add it as a different book, enter a different main title (a different subtitle alone is not enough).';

afterEach(() => {
  vi.restoreAllMocks();
});

function scanFile(match: Record<string, unknown> | null = null) {
  return {
    path: sourcePath,
    filename: "Case Author - Book Tail.epub",
    relPath: "Case Author - Book Tail.epub",
    mediaType: "ebook",
    size: 4096,
    parsed: {
      author: "Case Author",
      title: "Book: Tail",
      series: null,
      seriesPosition: null,
    },
    match,
    existingWorkId:
      match && typeof match.existingWorkId === "number"
        ? match.existingWorkId
        : null,
    hasExistingMediaType: false,
    routable: true,
  };
}

function scanReply(match: Record<string, unknown> | null = null) {
  return {
    scanId: "u2-scan",
    files: [scanFile(match)],
    warnings: [],
    olTotal: 0,
    olCompleted: 0,
  };
}

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )?.set;
  if (!setter) throw new Error("HTMLInputElement value setter missing");
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function inputByLabel(scope: HTMLElement, label: string): HTMLInputElement {
  const input = scope.querySelector<HTMLInputElement>(
    `input[aria-label="${label}"]`,
  );
  if (!input) throw new Error(`no input labelled "${label}"`);
  return input;
}

async function scan(scope: HTMLElement) {
  setInput(inputByLabel(scope, "Path to scan"), "/incoming");
  await clickButton(scope, "Scan");
  await vi.waitFor(
    () => expect(scope.textContent).toContain("Case Author - Book Tail.epub"),
    { timeout: 5000 },
  );
}

function expectMinimumOnlyBody(
  call: ApiCall,
  title: string,
  author = "Case Author",
) {
  expect(call.body).toEqual({
    items: [
      {
        path: sourcePath,
        olKey: "",
        title,
        author,
        deleteExisting: false,
      },
    ],
  });
  const item = (call.body as { items: Record<string, unknown>[] }).items[0]!;
  for (const providerField of [
    "authorOlKey",
    "candidateId",
    "hcKey",
    "grKey",
    "asin",
    "coverUrl",
    "isbn",
    "year",
  ]) {
    expect(item).not.toHaveProperty(providerField);
  }
}

async function runDeferredRecovery(retryTitle: string, importedWorkId: number) {
  let imports = 0;
  const api = installApiStub((call: ApiCall) => {
    if (call.method === "POST" && call.path === "/manualimport/scan") {
      expect(call.body).toEqual({ path: "/incoming" });
      return { status: 200, body: scanReply() };
    }
    if (call.method === "POST" && call.path === "/manualimport/import") {
      imports += 1;
      if (imports === 1) {
        expectMinimumOnlyBody(call, "Book: Tail");
        return {
          status: 200,
          body: {
            results: [
              {
                path: sourcePath,
                status: "failed",
                workId: null,
                error: singularDefer,
                mediaType: "ebook",
              },
            ],
          },
        };
      }
      expectMinimumOnlyBody(call, retryTitle);
      return {
        status: 200,
        body: {
          results: [
            {
              path: sourcePath,
              status: "imported",
              workId: importedWorkId,
              error: null,
              mediaType: "ebook",
            },
          ],
        },
      };
    }
    throw new Error(`unexpected call ${call.method} ${call.path}`);
  });

  const mounted = mountWith(newTestClient(), <ManualImportPage />);
  try {
    await scan(mounted.container);
    await clickButton(mounted.container, "Import Selected");
    await vi.waitFor(
      () => expect(mounted.container.textContent).toContain(singularDefer),
      { timeout: 5000 },
    );

    await clickButton(mounted.container, "Edit title and author");
    const title = inputByLabel(mounted.container, "Title");
    const author = inputByLabel(mounted.container, "Author");
    expect(title.value).toBe("Book: Tail");
    expect(author.value).toBe("Case Author");
    setInput(title, "   ");
    await clickButton(mounted.container, "Save");
    expect(inputByLabel(mounted.container, "Title").value.trim()).toBe("");
    expect(mounted.container.textContent).toContain(singularDefer);
    setInput(title, retryTitle);
    setInput(author, "   ");
    await clickButton(mounted.container, "Save");
    expect(inputByLabel(mounted.container, "Author").value.trim()).toBe("");
    expect(mounted.container.textContent).toContain(singularDefer);
    setInput(author, "Case Author");
    await clickButton(mounted.container, "Save");

    expect(mounted.container.textContent).not.toContain(singularDefer);
    expect(mounted.container.textContent).toContain(
      `${retryTitle} — Case Author`,
    );
    await clickButton(mounted.container, "Import Selected");
    await vi.waitFor(
      () => expect(mounted.container.textContent).toContain("Imported"),
      { timeout: 5000 },
    );
    expect(imports).toBe(2);
    expect(
      api.calls.some(
        (call) =>
          call.method === "POST" && call.path === "/manualimport/search",
      ),
    ).toBe(false);
  } finally {
    mounted.cleanup();
    api.restore();
  }
}

describe("ManualImportPage — REQ-003 deferred recovery", () => {
  // RED-UNTIL-U2: today the singular failure is visible but ManualImportPage has no title/author editor, so the stored-pair recovery cannot be submitted.
  it("edits the displayed deferred row to the stored pair and retries minimum-only", async () => {
    await runDeferredRecovery("Book", 41);
  });

  // RED-UNTIL-U2: today the singular failure is visible but ManualImportPage has no editor for a different-main create retry.
  it("edits a fresh deferred row to a different main title and retries minimum-only", async () => {
    await runDeferredRecovery("Another Book", 42);
  });

  // RED-UNTIL-U2: today a row's effective provider match can only be replaced by provider search; there is no manual override that clears its badge, result, and match-derived keys.
  it("saving a manual override clears a provider match and its existing-work badge", async () => {
    const providerMatch = {
      olKey: "OL123W",
      title: "Provider Book",
      author: "Provider Author",
      coverUrl: "https://covers.invalid/provider.jpg",
      existingWorkId: 77,
      candidateId: "openlibrary:OL123W",
      hcKey: "456",
      grKey: "789",
      asin: "B000U2",
      isbn13: ISBN_FIXTURE,
      year: 2026,
      language: "fr",
      source: "openlibrary",
    };
    const api = installApiStub((call: ApiCall) => {
      if (call.method === "POST" && call.path === "/manualimport/scan") {
        return { status: 200, body: scanReply(providerMatch) };
      }
      if (call.method === "POST" && call.path === "/manualimport/import") {
        expectMinimumOnlyBody(call, "Manual Identity", "Manual Author");
        return {
          status: 200,
          body: {
            results: [
              {
                path: sourcePath,
                status: "imported",
                workId: 78,
                error: null,
                mediaType: "ebook",
              },
            ],
          },
        };
      }
      throw new Error(`unexpected call ${call.method} ${call.path}`);
    });

    const mounted = mountWith(newTestClient(), <ManualImportPage />);
    try {
      await scan(mounted.container);
      expect(mounted.container.textContent).toContain("Provider Book");
      expect(mounted.container.textContent).toContain("in library");

      await clickButton(mounted.container, "Edit title and author");
      const title = inputByLabel(mounted.container, "Title");
      const author = inputByLabel(mounted.container, "Author");
      expect(title.value).toBe("Provider Book");
      expect(author.value).toBe("Provider Author");
      setInput(title, "Manual Identity");
      setInput(author, "Manual Author");
      await clickButton(mounted.container, "Save");

      expect(mounted.container.textContent).not.toContain("Provider Book");
      expect(mounted.container.textContent).not.toContain("in library");
      expect(mounted.container.textContent).toContain(
        "Manual Identity — Manual Author",
      );
      const checkbox = inputByLabel(
        mounted.container,
        "Select Case Author - Book Tail.epub",
      );
      expect(checkbox.disabled).toBe(false);
      if (!checkbox.checked) act(() => checkbox.click());
      expect(checkbox.checked).toBe(true);
      await clickButton(mounted.container, "Import Selected");
      await vi.waitFor(
        () => expect(mounted.container.textContent).toContain("Imported"),
        { timeout: 5000 },
      );
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});
