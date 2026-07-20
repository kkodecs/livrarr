import { describe, it, expect, vi } from "vitest";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { MemoryRouter } from "react-router";
import HistoryPage from "./HistoryPage";
import type { HistoryResponse } from "@/types/api";

vi.mock("@/api", () => ({
  getHistory: vi.fn().mockResolvedValue({ items: [] }),
  listWorks: vi.fn().mockResolvedValue({ items: [] }),
}));

describe("HistoryPage", () => {
  it("renders a row with an unrecognized event kind generically, without throwing", () => {
    const row = {
      id: 1,
      workId: null,
      eventType: "somethingFromTheFuture",
      data: {},
      date: "2026-07-20T00:00:00.000Z",
    } as unknown as HistoryResponse;

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: Infinity } },
    });
    queryClient.setQueryData(["history", "", false], { items: [row] });
    queryClient.setQueryData(["works"], { items: [] });

    const container = document.createElement("div");
    document.body.appendChild(container);
    const root = createRoot(container);

    act(() => {
      root.render(
        <MemoryRouter>
          <QueryClientProvider client={queryClient}>
            <HistoryPage />
          </QueryClientProvider>
        </MemoryRouter>,
      );
    });

    expect(container.textContent).toContain("somethingFromTheFuture");
    expect(container.querySelector("tbody tr svg")).not.toBeNull();

    act(() => {
      root.unmount();
    });
    container.remove();
  });
});
