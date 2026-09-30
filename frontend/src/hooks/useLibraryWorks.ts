import { useQuery } from "@tanstack/react-query";
import { listWorks } from "@/api";
import { computeTotalPages } from "@/utils/pagination";
import type { WorkDetailResponse } from "@/types/api";

/** `listWorks` filters and ordering; paging belongs to the walk. */
export type WorkListParams = Omit<
  NonNullable<Parameters<typeof listWorks>[0]>,
  "page" | "pageSize"
>;

const PAGE_SIZE = 1000;

/**
 * Every work matching `params`. GET /work caps a page at 1000, so one response
 * is not the library: this walks every page, taking the page count from each
 * response because the library can grow mid-walk (a list import running while
 * the page is open). A superseded walk stops between pages.
 */
export async function listAllWorks(
  params: WorkListParams,
  signal: AbortSignal,
): Promise<WorkDetailResponse[]> {
  const first = await listWorks({ ...params, pageSize: PAGE_SIZE, page: 1 });
  const items = [...first.items];
  let pages = computeTotalPages(first.total, PAGE_SIZE);
  for (let p = 2; p <= pages; p++) {
    if (signal.aborted) throw new DOMException("aborted", "AbortError");
    const next = await listWorks({ ...params, pageSize: PAGE_SIZE, page: p });
    items.push(...next.items);
    pages = computeTotalPages(next.total, PAGE_SIZE);
  }
  return items;
}

/**
 * The whole library, shared by Search, Queue and History. "works"-prefixed so
 * every invalidateQueries({ queryKey: ["works"] }) site refreshes it.
 */
export function useLibraryWorks() {
  return useQuery({
    queryKey: ["works", "library"],
    queryFn: ({ signal }) => listAllWorks({}, signal),
  });
}
