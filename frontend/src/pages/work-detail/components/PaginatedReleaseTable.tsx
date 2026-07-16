import { useState, useEffect } from "react";
import { Check, Loader2, Download, ChevronLeft, ChevronRight } from "lucide-react";
import { SortHeader } from "@/components/Page/SortHeader";
import { formatBytes, formatRelativeDate } from "@/utils/format";
import { computeTotalPages } from "@/utils/pagination";
import type { ReleaseResponse } from "@/types/api";

export type ReleaseSortField = "title" | "indexer" | "size" | "seeders" | "leechers" | "publishDate";

const PAGE_SIZE = 10;

export function PaginatedReleaseTable({
  items,
  sorting,
  grabbedGuids,
  grabbingGuid,
  grabMutation,
}: {
  items: ReleaseResponse[];
  sorting: { field: ReleaseSortField; dir: "asc" | "desc"; toggle: (f: ReleaseSortField) => void };
  grabbedGuids: Set<string>;
  grabbingGuid: string | null;
  grabMutation: { mutate: (r: ReleaseResponse) => void; isPending: boolean };
}) {
  const [page, setPage] = useState(0);
  const totalPages = computeTotalPages(items.length, PAGE_SIZE);
  const pageItems = items.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);

  // Reset to page 0 when items change (sort, filter).
  useEffect(() => {
    setPage(0);
  }, [items.length]);

  return (
    <div>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="border-b border-border">
            <tr>
              <SortHeader field="title" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Title</SortHeader>
              <SortHeader field="indexer" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Indexer</SortHeader>
              <SortHeader field="size" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle} className="text-right">Size</SortHeader>
              <SortHeader field="seeders" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle} className="text-right">S</SortHeader>
              <SortHeader field="leechers" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle} className="text-right">L</SortHeader>
              <SortHeader field="publishDate" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Age</SortHeader>
              <th className="w-10 px-3 py-2" />
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {pageItems.map((release) => (
              <tr key={release.guid} className="hover:bg-zinc-800/50">
                <td
                  className="max-w-sm truncate px-3 py-2 text-zinc-300"
                  title={release.title}
                >
                  {release.title}
                </td>
                <td className="px-3 py-2 text-muted">{release.indexer}</td>
                <td className="px-3 py-2 text-right text-muted">
                  {formatBytes(release.size)}
                </td>
                <td className="px-3 py-2 text-right text-muted">
                  {release.seeders ?? "—"}
                </td>
                <td className="px-3 py-2 text-right text-muted">
                  {release.leechers ?? "—"}
                </td>
                <td className="px-3 py-2 text-muted">
                  {release.publishDate
                    ? formatRelativeDate(release.publishDate)
                    : "—"}
                </td>
                <td className="px-3 py-2">
                  {grabbedGuids.has(release.guid) ? (
                    <span className="inline-flex rounded p-1 text-green-400" title="Grabbed">
                      <Check size={14} />
                    </span>
                  ) : grabbingGuid === release.guid ? (
                    <span className="inline-flex rounded p-1 text-brand">
                      <Loader2 size={14} className="animate-spin" />
                    </span>
                  ) : (
                    <button
                      onClick={() => grabMutation.mutate(release)}
                      disabled={grabMutation.isPending}
                      className="rounded p-1 text-muted hover:text-brand"
                      title="Grab"
                    >
                      <Download size={14} />
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {totalPages > 1 && (
        <div className="flex items-center justify-between border-t border-border px-3 py-2">
          <span className="text-xs text-muted">
            {page * PAGE_SIZE + 1}–{Math.min((page + 1) * PAGE_SIZE, items.length)} of {items.length}
          </span>
          <div className="flex items-center gap-1">
            <button
              onClick={() => setPage((p) => Math.max(0, p - 1))}
              disabled={page === 0}
              className="rounded p-1 text-muted hover:text-zinc-100 disabled:opacity-30"
            >
              <ChevronLeft size={16} />
            </button>
            <span className="text-xs text-muted px-2">
              {page + 1} / {totalPages}
            </span>
            <button
              onClick={() => setPage((p) => Math.min(totalPages - 1, p + 1))}
              disabled={page >= totalPages - 1}
              className="rounded p-1 text-muted hover:text-zinc-100 disabled:opacity-30"
            >
              <ChevronRight size={16} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
