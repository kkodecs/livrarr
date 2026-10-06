import { afterEach, describe, expect, it, vi } from "vitest";
import UnmappedPage from "@/pages/unmapped/UnmappedPage";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";

let cleanup: () => void = () => {};
afterEach(() => cleanup());

describe("Unmapped Files with no root folder", () => {
  it("Go to Settings points to /settings/mediamanagement", async () => {
    const api = installApiStub((call: ApiCall) =>
      call.method === "GET" && call.path === "/rootfolder"
        ? { status: 200, body: [] }
        : {
            status: 404,
            body: { status: 404, error: "not_found", message: `unstubbed ${call.path}` },
          },
    );
    const mounted = mountWith(newTestClient(), <UnmappedPage />);
    cleanup = () => {
      mounted.cleanup();
      api.restore();
    };
    await vi.waitFor(
      () => expect(mounted.container.textContent).toContain("Unmapped Files"),
      { timeout: 5000 },
    );
    await clickButton(mounted.container, "Root Folder");
    // Control: the no-root-folder state shows its link.
    const link = Array.from(mounted.container.querySelectorAll("a")).find(
      (a) => a.textContent?.trim() === "Go to Settings",
    );
    expect(link).toBeDefined();
    expect(link!.getAttribute("href")).toBe("/settings/mediamanagement");
  });
});
