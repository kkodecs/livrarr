import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

// Vite's build defines this constant; the test config does not, and the
// sidebar reads it when its module loads.
vi.hoisted(() => {
  (globalThis as Record<string, unknown>).__APP_VERSION__ = "0.0.0-test";
});

import { act } from "react";
import { createRoot } from "react-dom/client";
import { App } from "@/App";
import { useAuthStore } from "@/stores/auth";
import { installApiStub, type ApiCall, type StubReply } from "@/test-support/apiStub";

const FIRST_TOKEN = "6f1c2b9e0d4a7f3851c6e2b0a9d8f714";
const SECOND_TOKEN = "d41d8cd98f00b204e9800998ecf8427e";
const TOKENS = [FIRST_TOKEN, SECOND_TOKEN];
const REFUSAL =
  "The setup token is missing or wrong. Find it in Livrarr's startup output, or in the file setup-token in its data folder (/config/setup-token in Docker).";

// The whole app is mounted for real; only the network boundary is stubbed.
// The first setup attempt is held until the test releases it, then refused
// with the server's 403; the second succeeds.
function stubApi() {
  let setupAttempts = 0;
  let releaseFirst: () => void = () => {};
  const firstHeld = new Promise<void>((resolve) => {
    releaseFirst = resolve;
  });
  const stub = installApiStub(async (call: ApiCall): Promise<StubReply> => {
    const path = call.path.split("?")[0];
    if (path === "/setup/status") {
      return { status: 200, body: { setupRequired: setupAttempts < 2 } };
    }
    if (path === "/setup" && call.method === "POST") {
      setupAttempts += 1;
      if (setupAttempts === 1) {
        await firstHeld;
        return {
          status: 403,
          body: { status: 403, error: "forbidden", message: REFUSAL },
        };
      }
      return { status: 200, body: { token: "session-token", apiKey: "api-key" } };
    }
    if (path === "/auth/me") {
      return {
        status: 200,
        body: {
          user: {
            id: 1,
            username: "owner",
            role: "admin",
            createdAt: "2026-01-01T00:00:00Z",
            updatedAt: "2026-01-01T00:00:00Z",
          },
          authType: "session",
        },
      };
    }
    return {
      status: 404,
      body: { error: "not_found", message: `unstubbed ${path}`, status: 404 },
    };
  });
  return { ...stub, releaseFirst };
}

async function settle(ms = 20) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function waitFor(condition: () => boolean, what: string, timeoutMs = 3000) {
  const deadline = Date.now() + timeoutMs;
  while (!condition()) {
    if (Date.now() > deadline) throw new Error(`timed out waiting for ${what}`);
    await settle();
  }
}

function mountApp(path: string) {
  window.history.pushState({}, "", path);
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => {
    root.render(<App />);
  });
  return {
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  if (!setter) throw new Error("HTMLInputElement value setter missing");
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function namedInput(name: string): HTMLInputElement {
  const input = document.querySelector<HTMLInputElement>(`input[name="${name}"]`);
  if (!input) throw new Error(`no input named ${name}`);
  return input;
}

function setupTokenLabel(): HTMLLabelElement | undefined {
  return Array.from(document.querySelectorAll("label")).find((el) =>
    el.textContent?.trim().startsWith("Setup token"),
  );
}

/** The input labelled "Setup token". */
function setupTokenInput(): HTMLInputElement {
  const label = setupTokenLabel();
  const input = label
    ? ((label.control as HTMLInputElement | null) ?? label.querySelector("input"))
    : null;
  if (!input) throw new Error('no input labelled "Setup token" on the setup page');
  return input;
}

function submitButton(): HTMLButtonElement | undefined {
  return Array.from(document.querySelectorAll("button")).find((b) =>
    b.textContent?.includes("Create Account"),
  );
}

async function submit() {
  const button = submitButton();
  if (!button) throw new Error("no account submit button");
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function pageText(): string {
  return (document.body.textContent ?? "").replace(/\s+/g, " ");
}

function storageEntries(storage: Storage): string[] {
  const entries: string[] = [];
  for (let i = 0; i < storage.length; i += 1) {
    const key = storage.key(i);
    if (key !== null) entries.push(key, storage.getItem(key) ?? "");
  }
  return entries;
}

function carriesAToken(value: unknown): boolean {
  const text = String(value);
  return TOKENS.some((token) => text.includes(token));
}

function expectNoTokenAnywhereNow(calls: ApiCall[]) {
  expect(storageEntries(sessionStorage).some(carriesAToken)).toBe(false);
  expect(storageEntries(localStorage).some(carriesAToken)).toBe(false);
  expect(carriesAToken(window.location.href)).toBe(false);
  expect(calls.some((c) => carriesAToken(c.path))).toBe(false);
}

describe("Setup page token", () => {
  let restore: () => void = () => {};
  let cleanup: () => void = () => {};

  beforeEach(() => {
    localStorage.clear();
    sessionStorage.clear();
    localStorage.setItem("livrarr-tour-completed", "true");
    useAuthStore.setState({ status: "loading", user: null, token: null });
  });

  afterEach(async () => {
    await settle();
    vi.restoreAllMocks();
    cleanup();
    restore();
    localStorage.clear();
    sessionStorage.clear();
  });

  it("requires the token, sends it in the body, shows a refusal, and never stores it", async () => {
    const stub = stubApi();
    restore = stub.restore;
    cleanup = mountApp("/setup").cleanup;
    await waitFor(
      () => useAuthStore.getState().status === "setup_required" && submitButton() !== undefined,
      "the account step",
    );

    const tokenInput = setupTokenInput();
    expect(pageText()).toContain("Livrarr printed a one-time setup token when it started.");
    expect(pageText()).toContain("docker logs livrarr");
    expect(pageText()).toContain("setup-token in your config folder");

    // Every storage write and history change from here on is recorded and
    // still performed.
    const storageWrites = vi.spyOn(Storage.prototype, "setItem");
    const pushes = vi.spyOn(window.history, "pushState");
    const replaces = vi.spyOn(window.history, "replaceState");
    const setupCalls = () => stub.calls.filter((c) => c.method === "POST" && c.path === "/setup");

    setInput(namedInput("username"), "owner");
    setInput(namedInput("password"), "owner-password-1");
    setInput(namedInput("confirmPassword"), "owner-password-1");
    await submit();
    await waitFor(() => (setupTokenLabel()?.textContent ?? "").includes("Required"), "a required-field error");
    expect(setupCalls()).toHaveLength(0);

    setInput(tokenInput, FIRST_TOKEN);
    expectNoTokenAnywhereNow(stub.calls);
    await submit();
    await waitFor(() => setupCalls().length === 1, "the first setup request");
    expect(setupCalls()[0]?.body).toMatchObject({
      username: "owner",
      password: "owner-password-1",
      setupToken: FIRST_TOKEN,
    });
    expectNoTokenAnywhereNow(stub.calls);

    stub.releaseFirst();
    await waitFor(() => pageText().includes(REFUSAL), "the refusal message");
    expect(pageText()).not.toContain("Session expired");
    expect(useAuthStore.getState().status).toBe("setup_required");
    expect(window.location.pathname).toBe("/setup");
    expectNoTokenAnywhereNow(stub.calls);

    setInput(setupTokenInput(), SECOND_TOKEN);
    await submit();
    await waitFor(() => setupCalls().length === 2, "the second setup request");
    expect(setupCalls()[1]?.body).toMatchObject({ setupToken: SECOND_TOKEN });
    await waitFor(() => useAuthStore.getState().status === "authenticated", "authentication");
    expectNoTokenAnywhereNow(stub.calls);

    expect(storageWrites).toHaveBeenCalled();
    expect(storageWrites.mock.calls.some((args) => args.some(carriesAToken))).toBe(false);
    expect(pushes.mock.calls.some((args) => carriesAToken(args[2]))).toBe(false);
    expect(replaces.mock.calls.some((args) => carriesAToken(args[2]))).toBe(false);
  });
});
