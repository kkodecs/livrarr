import { act } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth";
import {
  clickButton,
  installApiStub,
  mountWith,
  newTestClient,
  type ApiCall,
} from "@/test-support/apiStub";
import ReadarrImportPage from "./ReadarrImportPage";

function baseReply(call: ApiCall) {
  if (call.method === "GET" && call.path === "/rootfolder") {
    return { status: 200, body: [] };
  }
  if (call.method === "GET" && call.path === "/import/readarr/history") {
    return { status: 200, body: { imports: [] } };
  }
  throw new Error(`unexpected call ${call.method} ${call.path}`);
}

function changeInput(input: HTMLInputElement, value: string) {
  act(() => {
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

afterEach(() => {
  useAuthStore.setState({ isAdmin: false });
});

describe("Readarr approved origins", () => {
  it("lists, adds, and removes origins through the real API wrapper", async () => {
    useAuthStore.setState({ isAdmin: true });
    const origins = [
      {
        id: 1,
        origin: "http://existing-readarr.internal:8787",
        createdAt: "2026-08-16T23:40:00Z",
      },
    ];
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/import/readarr/origin") {
        return { status: 200, body: origins };
      }
      if (call.method === "POST" && call.path === "/import/readarr/origin") {
        expect(call.body).toEqual({
          url: "http://new-readarr.internal:8787/path",
        });
        origins.push({
          id: 2,
          origin: "http://new-readarr.internal:8787",
          createdAt: "2026-08-16T23:45:00Z",
        });
        return { status: 200, body: origins[1] };
      }
      if (
        call.method === "DELETE" &&
        call.path === "/import/readarr/origin/2"
      ) {
        origins.splice(1, 1);
        return { status: 204 };
      }
      return baseReply(call);
    });
    const mounted = mountWith(newTestClient(), <ReadarrImportPage />);
    try {
      await act(async () => {
        await Promise.resolve();
      });
      await Promise.resolve();
      await act(async () => {
        await Promise.resolve();
      });
      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "http://existing-readarr.internal:8787",
        ),
      );

      const input = mounted.container.querySelector<HTMLInputElement>(
        'input[placeholder="readarr.internal:8787"]',
      );
      expect(input).not.toBeNull();
      changeInput(input!, "new-readarr.internal:8787/path");
      await clickButton(mounted.container, "Approve origin");

      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "http://new-readarr.internal:8787",
        ),
      );
      const remove = mounted.container.querySelector<HTMLButtonElement>(
        'button[aria-label="Remove http://new-readarr.internal:8787"]',
      );
      expect(remove).not.toBeNull();
      await act(async () => {
        remove!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
      await vi.waitFor(() =>
        expect(mounted.container.textContent).not.toContain(
          "http://new-readarr.internal:8787",
        ),
      );

      expect(api.calls.map((call) => `${call.method} ${call.path}`)).toEqual(
        expect.arrayContaining([
          "GET /import/readarr/origin",
          "POST /import/readarr/origin",
          "DELETE /import/readarr/origin/2",
        ]),
      );
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });

  it("shows the private-origin approval hint after a connect failure", async () => {
    useAuthStore.setState({ isAdmin: true });
    const api = installApiStub((call) => {
      if (call.method === "GET" && call.path === "/import/readarr/origin") {
        return { status: 200, body: [] };
      }
      if (call.method === "POST" && call.path === "/import/readarr/connect") {
        return {
          status: 500,
          body: {
            status: 500,
            error: "internal",
            message: "unable to connect to the Readarr instance",
          },
        };
      }
      return baseReply(call);
    });
    const mounted = mountWith(newTestClient(), <ReadarrImportPage />);
    try {
      const inputs =
        mounted.container.querySelectorAll<HTMLInputElement>("input");
      changeInput(inputs[0]!, "private-readarr.internal:8787");
      changeInput(inputs[1]!, "secret-key");
      await clickButton(mounted.container, "Connect");

      await vi.waitFor(() =>
        expect(mounted.container.textContent).toContain(
          "Private or local Readarr addresses must be approved",
        ),
      );
      expect(api.calls).toContainEqual(
        expect.objectContaining({
          method: "POST",
          path: "/import/readarr/connect",
        }),
      );
    } finally {
      mounted.cleanup();
      api.restore();
    }
  });
});
