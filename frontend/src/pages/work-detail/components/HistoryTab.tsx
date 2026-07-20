import { useQuery } from "@tanstack/react-query";
import { Download, Trash2, Book, ExternalLink, Pencil, Clock, BookPlus, GitMerge, BadgeCheck } from "lucide-react";
import { getHistory } from "@/api";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { formatRelativeDate } from "@/utils/format";
import type { HistoryResponse } from "@/types/api";

const EVENT_ICONS: Record<string, typeof Download> = {
  grabbed: Download,
  downloadCompleted: Download,
  downloadFailed: Trash2,
  imported: Book,
  importFailed: Trash2,
  enriched: ExternalLink,
  enrichmentFailed: Trash2,
  tagWritten: Pencil,
  tagWriteFailed: Trash2,
  fileDeleted: Trash2,
  added: BookPlus,
  workDeleted: Trash2,
  worksMerged: GitMerge,
  identityResolved: BadgeCheck,
};

export function HistoryTab({ workId }: { workId: number }) {
  const {
    data: history,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["history", workId],
    queryFn: () => getHistory({ workId }),
    select: (res) => res.items,
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  if (!history || history.length === 0) {
    return <EmptyState icon={<Clock size={24} />} title="No history" />;
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead className="border-b border-border">
          <tr>
            <th className="w-10 px-3 py-2" />
            <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
              Event
            </th>
            <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
              Details
            </th>
            <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
              Date
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-border">
          {history.map((entry) => {
            const Icon = EVENT_ICONS[entry.eventType] ?? Clock;
            return (
              <tr key={entry.id} className="hover:bg-zinc-800/50">
                <td className="px-3 py-2 text-muted">
                  <Icon size={14} />
                </td>
                <td className="px-3 py-2 text-zinc-300 capitalize">
                  {entry.eventType.replace(/([A-Z])/g, " $1").trim()}
                </td>
                <td className="max-w-md truncate px-3 py-2 text-xs text-muted">
                  {summarizeHistoryData(entry)}
                </td>
                <td className="px-3 py-2 text-muted">
                  {formatRelativeDate(entry.date)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function summarizeHistoryData(entry: HistoryResponse): string {
  const d = entry.data;
  if (d.title && typeof d.title === "string") return d.title;
  if (d.message && typeof d.message === "string") return d.message;
  if (d.path && typeof d.path === "string") return d.path;
  if (d.work_title && typeof d.work_title === "string") return d.work_title;
  return "";
}
