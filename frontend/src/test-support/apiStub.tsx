import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter, Route, Routes } from "react-router";

/**
 * Test support for driving pages through the real API layer.
 *
 * Nothing under `@/api` is mocked here. The stub sits at the network boundary
 * — the one seam a browser test cannot cross — so every test still runs the
 * real `apiFetch`: the real URL, the real method, the real 202/204 handling
 * and the real error envelope. A page that builds the wrong path or misreads
 * a status fails these tests, which is the point.
 */

// React only flushes effects inside act() when it knows it is under test.
declare global {
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

export interface ApiCall {
  method: string;
  /** Path with the `/api/v1` prefix stripped, e.g. `/author/7/resolve`. */
  path: string;
  body: unknown;
}

export interface StubReply {
  status: number;
  /** Omit for a bodiless reply (202 Accepted, 204 No Content). */
  body?: unknown;
}

export type StubHandler = (call: ApiCall) => StubReply | Promise<StubReply>;

function toResponse(reply: StubReply): Response {
  if (reply.body === undefined) {
    return new Response(null, { status: reply.status });
  }
  return new Response(JSON.stringify(reply.body), {
    status: reply.status,
    headers: { "content-type": "application/json" },
  });
}

export function installApiStub(handler: StubHandler): {
  calls: ApiCall[];
  restore: () => void;
} {
  const calls: ApiCall[] = [];
  const original = globalThis.fetch;

  globalThis.fetch = (async (
    input: RequestInfo | URL,
    init?: RequestInit,
  ): Promise<Response> => {
    const url =
      typeof input === "string"
        ? input
        : input instanceof URL
          ? input.toString()
          : input.url;
    const method = (init?.method ?? "GET").toUpperCase();
    let body: unknown;
    if (typeof init?.body === "string") {
      try {
        body = JSON.parse(init.body);
      } catch {
        body = init.body;
      }
    }
    const call: ApiCall = {
      method,
      path: url.replace(/^\/api\/v1/, ""),
      body,
    };
    calls.push(call);
    return toResponse(await handler(call));
  }) as typeof globalThis.fetch;

  return {
    calls,
    restore: () => {
      globalThis.fetch = original;
    },
  };
}

export function newTestClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
}

/** Mount a page against a caller-supplied client, so several can share a cache. */
export function mountWith(
  queryClient: QueryClient,
  ui: ReactNode,
  options: { path?: string; route?: string } = {},
) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  const entry = options.path ?? "/";
  const routePath = options.route ?? "*";
  act(() => {
    root.render(
      <MemoryRouter initialEntries={[entry]}>
        <QueryClientProvider client={queryClient}>
          <Routes>
            <Route path={routePath} element={ui} />
          </Routes>
        </QueryClientProvider>
      </MemoryRouter>,
    );
  });
  return {
    container,
    cleanup: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/** Click a button by its visible label, inside the container or a portal. */
export async function clickButton(scope: HTMLElement, label: string) {
  const button = Array.from(scope.querySelectorAll("button")).find((b) =>
    b.textContent?.includes(label),
  );
  if (!button) throw new Error(`no button labelled "${label}"`);
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

/** Click a button by its `title`, for icon-only controls. */
export async function clickTitled(scope: HTMLElement, title: string) {
  const button = scope.querySelector<HTMLButtonElement>(
    `button[title="${title}"]`,
  );
  if (!button) throw new Error(`no button titled "${title}"`);
  await act(async () => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}
