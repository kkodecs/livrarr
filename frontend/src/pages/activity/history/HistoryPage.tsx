import { useState, useMemo } from "react";
import { Link } from "react-router";
import { useQuery } from "@tanstack/react-query";
import { subDays, formatISO } from "date-fns";
import {
  Download,
  Check,
  AlertCircle,
  Trash2,
  RefreshCw,
  Tag,
  XCircle,
  FileDown,
  FileX,
} from "lucide-react";
import { getHistory, listWorks } from "@/api";
import { workName } from "@/utils/works";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { EmptyState } from "@/components/Page/EmptyState";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { formatRelativeDate, formatAbsoluteDate } from "@/utils/format";
import { useSort } from "@/hooks/useSort";
import { SortHeader } from "@/components/Page/SortHeader";
import type { EventType } from "@/types/api";

type HistorySortField = "eventType" | "workId" | "date";

const EVENT_ICONS: Record<EventType, React.ElementType> = {
  grabbed: Download,
  downloadCompleted: FileDown,
  downloadFailed: FileX,
  imported: Check,
  importFailed: AlertCircle,
  enriched: RefreshCw,
  enrichmentFailed: XCircle,
  tagWritten: Tag,
  tagWriteFailed: Tag,
  fileDeleted: Trash2,
};

const EVENT_LABELS: Record<EventType, string> = {
  grabbed: "Grabbed",
  downloadCompleted: "Download Completed",
  downloadFailed: "Download Failed",
  imported: "Imported",
  importFailed: "Import Failed",
  enriched: "Enriched",
  enrichmentFailed: "Enrichment Failed",
  tagWritten: "Tag Written",
  tagWriteFailed: "Tag Write Failed",
  fileDeleted: "File Deleted",
};

const ALL_EVENT_TYPES: EventType[] = [
  "grabbed",
  "downloadCompleted",
  "downloadFailed",
  "imported",
  "importFailed",
  "enriched",
  "enrichmentFailed",
  "tagWritten",
  "tagWriteFailed",
  "fileDeleted",
];

const ROW_LIMIT = 1000;

export default function HistoryPage() {
  const [filterType, setFilterType] = useState<EventType | "">("");
  const [showAll, setShowAll] = useState(false);
  const [loadOlder, setLoadOlder] = useState(false);

  const defaultStart = useMemo(() => formatISO(subDays(new Date(), 30)), []);

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["history", filterType, loadOlder],
    queryFn: () =>
      getHistory({
        eventType: filterType || undefined,
        startDate: loadOlder ? undefined : defaultStart,
      }),
    select: (res) => res.items,
  });

  const { data: works } = useQuery({
    queryKey: ["works"],
    queryFn: () => listWorks(),
    select: (res) => res.items,
  });

  const sorting = useSort<HistorySortField>("date", "desc");

  if (isLoading || !works) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  const allRows = sorting.sort(data ?? [], (item, field) => {
    switch (field) {
      case "eventType": return item.eventType;
      case "workId": return item.workId ?? 0;
      case "date": return item.date;
    }
  });
  const truncated = !showAll && allRows.length > ROW_LIMIT;
  const displayRows = truncated ? allRows.slice(0, ROW_LIMIT) : allRows;

  function eventDetails(eventData: Record<string, unknown>): string {
    const parts: string[] = [];
    if (eventData.title) parts.push(String(eventData.title));
    if (eventData.indexer) parts.push(`via ${eventData.indexer}`);
    if (eventData.path) parts.push(String(eventData.path));
    if (eventData.message) parts.push(String(eventData.message));
    return parts.join(" \u2014 ") || "\u2014";
  }

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">History</h1>
        <div className="flex items-center gap-3">
          <select
            value={filterType}
            onChange={(e) => setFilterType(e.target.value as EventType | "")}
            className="rounded border border-border bg-zinc-800 px-3 py-1.5 text-sm text-zinc-200"
          >
            <option value="">All Events</option>
            {ALL_EVENT_TYPES.map((t) => (
              <option key={t} value={t}>
                {EVENT_LABELS[t]}
              </option>
            ))}
          </select>
        </div>
      </PageToolbar>

      <PageContent>
        {allRows.length === 0 ? (
          <EmptyState title="No activity yet" />
        ) : (
          <>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead className="sticky top-0 z-10 bg-zinc-900">
                  <tr className="border-b border-border text-left text-xs text-muted">
                    <SortHeader field="eventType" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Event</SortHeader>
                    <th className="hidden lg:table-cell px-3 py-2 text-zinc-600 w-12">Event ID</th>
                    <SortHeader field="workId" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Work</SortHeader>
                    <th className="hidden sm:table-cell px-3 py-2">Details</th>
                    <SortHeader field="date" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle} className="hidden sm:table-cell">Date</SortHeader>
                  </tr>
                </thead>
                <tbody>
                  {displayRows.map((row) => {
                    const Icon = EVENT_ICONS[row.eventType] ?? AlertCircle;
                    return (
                      <tr
                        key={row.id}
                        className="border-b border-border/50 text-zinc-300 hover:bg-zinc-800/50"
                      >
                        <td className="whitespace-nowrap px-3 py-2">
                          <span className="inline-flex items-center gap-1.5 text-xs">
                            <Icon size={14} className="text-muted" />
                            {EVENT_LABELS[row.eventType] ?? row.eventType}
                          </span>
                        </td>
                        <td className="hidden lg:table-cell px-3 py-2 text-[11px] text-zinc-600">{row.id}</td>
                        <td className="px-3 py-2">
                          {row.workId ? (
                            <Link
                              to={`/work/${row.workId}`}
                              className="text-brand hover:underline"
                            >
                              {workName(works, row.workId)}
                            </Link>
                          ) : (
                            <span className="text-muted">—</span>
                          )}
                        </td>
                        <td
                          className="hidden sm:table-cell max-w-[300px] truncate px-3 py-2"
                          title={eventDetails(row.data)}
                        >
                          {eventDetails(row.data)}
                        </td>
                        <td
                          className="hidden sm:table-cell whitespace-nowrap px-3 py-2 text-muted"
                          title={formatAbsoluteDate(row.date)}
                        >
                          {formatRelativeDate(row.date)}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>

            {truncated && (
              <div className="mt-4 text-center">
                <button
                  onClick={() => setShowAll(true)}
                  className="btn-secondary text-sm"
                >
                  Show All ({allRows.length} total)
                </button>
              </div>
            )}

            {!loadOlder && (
              <div className="mt-4 text-center">
                <button
                  onClick={() => setLoadOlder(true)}
                  className="btn-secondary text-sm"
                >
                  Load older
                </button>
              </div>
            )}
          </>
        )}
      </PageContent>
    </>
  );
}
