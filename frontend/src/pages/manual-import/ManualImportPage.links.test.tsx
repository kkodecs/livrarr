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

const FILE = "Unroutable Author - Unroutable Book.m4b";

/** A scan that found one audiobook no root folder can take. */
const scanReply = {
  scanId: "links-scan",
  files: [
    {
      path: `/incoming/${FILE}`,
      filename: FILE,
      relPath: FILE,
      mediaType: "audiobook",
      size: 4096,
      parsed: {
        author: "Unroutable Author",
        title: "Unroutable Book",
        series: null,
        seriesPosition: null,
      },
      match: null,
      existingWorkId: null,
      hasExistingMediaType: false,
      routable: false,
    },
  ],
  warnings: [],
  olTotal: 0,
  olCompleted: 0,
};

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    "value",
  )!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

let cleanup: () => void = () => {};
afterEach(() => cleanup());

describe("Manual Import: a file no root folder can take", () => {
  it("Configure root folder points to /settings/mediamanagement in a new tab", async () => {
    const api = installApiStub((call: ApiCall) =>
      call.method === "POST" && call.path === "/manualimport/scan"
        ? { status: 200, body: scanReply }
        : {
            status: 404,
            body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
          },
    );
    const mounted = mountWith(newTestClient(), <ManualImportPage />);
    cleanup = () => {
      mounted.cleanup();
      api.restore();
    };
    setInput(
      mounted.container.querySelector<HTMLInputElement>(
        'input[aria-label="Path to scan"]',
      )!,
      "/incoming",
    );
    await clickButton(mounted.container, "Scan");
    await vi.waitFor(
      () => expect(mounted.container.textContent).toContain(FILE),
      { timeout: 5000 },
    );
    // Control: the unroutable row shows its link.
    const link = Array.from(mounted.container.querySelectorAll("a")).find(
      (a) => a.textContent?.trim() === "Configure root folder",
    );
    expect(link).toBeDefined();
    expect({
      href: link!.getAttribute("href"),
      target: link!.getAttribute("target"),
    }).toEqual({ href: "/settings/mediamanagement", target: "_blank" });
  });
});
