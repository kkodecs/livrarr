import { useState } from "react";
import { Link } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { RefreshCw, RotateCcw, Trash2 } from "lucide-react";
import { getQueue, removeQueueItem, retryImport, listWorks } from "@/api";
import { computeTotalPages } from "@/utils/pagination";
import { workName } from "@/utils/works";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { Pagination } from "@/components/Page/Pagination";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { EmptyState } from "@/components/Page/EmptyState";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { formatMB, formatEta, formatRelativeDate } from "@/utils/format";
import type { QueueItemResponse, GrabStatus } from "@/types/api";

const STATUS_LABELS: Record<GrabStatus, string> = {
  sent: "Downloading",
  confirmed: "Downloading",
  importing: "Importing",
  imported: "Imported",
  importFailed: "Import Failed",
  removed: "Removed",
  failed: "Failed",
};

const STATUS_COLORS: Record<GrabStatus, string> = {
  sent: "bg-blue-500/20 text-blue-400",
  confirmed: "bg-blue-500/20 text-blue-400",
  importing: "bg-yellow-500/20 text-yellow-400",
  imported: "bg-green-500/20 text-green-400",
  importFailed: "bg-red-500/20 text-red-400",
  removed: "bg-zinc-600/20 text-zinc-400",
  failed: "bg-red-500/20 text-red-400",
};

export default function QueuePage() {
  const queryClient = useQueryClient();
  const [page, setPage] = useState(1);
  const [removeTarget, setRemoveTarget] = useState<QueueItemResponse | null>(null);

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["queue", page],
    queryFn: () => getQueue(page),
  });

  const { data: works } = useQuery({
    queryKey: ["works"],
    queryFn: () => listWorks(),
    select: (res) => res.items,
  });

  const removeMutation = useMutation({
    mutationFn: (id: number) => removeQueueItem(id),
    onSuccess: () => {
      toast.success("Item removed");
      queryClient.invalidateQueries({ queryKey: ["queue"] });
    },
    onError: (e: Error) => toast.error(e.message ?? "Failed to remove"),
  });

  const retryMutation = useMutation({
    mutationFn: (grabId: number) => retryImport(grabId),
    onSuccess: () => {
      toast.success("Import retry queued");
      queryClient.invalidateQueries({ queryKey: ["queue"] });
    },
    onError: (e: Error) => toast.error(e.message ?? "Failed to retry"),
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  const items = data?.items ?? [];
  const total = data?.total ?? 0;
  const totalPages = computeTotalPages(total, data?.perPage ?? 25);

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Queue</h1>
        <button
          onClick={() => refetch()}
          className="btn-secondary inline-flex items-center gap-1.5"
        >
          <RefreshCw size={14} />
          Refresh
        </button>
      </PageToolbar>

      <PageContent>
        {items.length === 0 && page === 1 ? (
          <EmptyState title="No grabs yet" />
        ) : (
          <>
            <div className="mb-3">
              <Pagination
                page={page}
                totalPages={totalPages}
                total={total}
                pageSize={data?.perPage ?? 25}
                onPageChange={setPage}
              />
            </div>
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-border text-left text-xs text-muted">
                    <th className="px-3 py-2">Status</th>
                    <th className="hidden lg:table-cell px-3 py-2 text-zinc-600 w-16">Grab ID</th>
                    <th className="px-3 py-2">Work</th>
                    <th className="hidden sm:table-cell px-3 py-2">Format</th>
                    <th className="hidden md:table-cell px-3 py-2">Release</th>
                    <th className="hidden lg:table-cell px-3 py-2">Indexer</th>
                    <th className="hidden lg:table-cell px-3 py-2">Protocol</th>
                    <th className="hidden sm:table-cell px-3 py-2">Size</th>
                    <th className="px-3 py-2">Progress</th>
                    <th className="hidden lg:table-cell px-3 py-2">Client</th>
                    <th className="hidden md:table-cell px-3 py-2">Grabbed</th>
                    <th className="px-3 py-2" />
                  </tr>
                </thead>
                <tbody>
                  {items.map((item) => (
                    <tr
                      key={item.id}
                      className="border-b border-border/50 text-zinc-300 hover:bg-zinc-800/50"
                    >
                      <td className="px-3 py-2">
                        {item.status === "importFailed" ? (
                          <button
                            onClick={() => retryMutation.mutate(item.id)}
                            disabled={retryMutation.isPending}
                            className="inline-flex items-center gap-1 rounded bg-orange-600/20 px-2 py-1 text-xs text-orange-400 hover:bg-orange-600/30 hover:text-orange-300"
                            title={item.error ?? "Retry Import"}
                          >
                            <RotateCcw size={12} />
                            Retry
                          </button>
                        ) : (
                          <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${STATUS_COLORS[item.status]}`}>
                            {STATUS_LABELS[item.status]}
                          </span>
                        )}
                      </td>
                      <td className="hidden lg:table-cell px-3 py-2 text-[11px] text-zinc-600">{item.id}</td>
                      <td className="px-3 py-2">
                        <Link to={`/work/${item.workId}`} className="text-brand hover:underline">
                          {workName(works, item.workId)}
                        </Link>
                      </td>
                      <td className="hidden sm:table-cell px-3 py-2 text-xs text-muted">
                        {item.mediaType === "ebook" ? "Ebook" : item.mediaType === "audiobook" ? "Audiobook" : "\u2014"}
                      </td>
                      <td className="hidden md:table-cell max-w-[200px] truncate px-3 py-2" title={item.title}>
                        {item.title}
                      </td>
                      <td className="hidden lg:table-cell px-3 py-2 text-xs text-muted">
                        {item.indexer || "\u2014"}
                      </td>
                      <td className="hidden lg:table-cell px-3 py-2">
                        <span className={`text-xs ${item.protocol === "usenet" ? "text-purple-400" : "text-cyan-400"}`}>
                          {item.protocol === "usenet" ? "NZB" : "Torrent"}
                        </span>
                      </td>
                      <td className="hidden sm:table-cell whitespace-nowrap px-3 py-2 text-muted">
                        {item.size ? formatMB(item.size) : "\u2014"}
                      </td>
                      <td className="px-3 py-2">
                        {item.progress ? (
                          <div className="flex items-center gap-2">
                            <div className="h-1.5 w-16 rounded-full bg-zinc-700">
                              <div
                                className="h-1.5 rounded-full bg-brand transition-all"
                                style={{ width: `${item.progress.percent}%` }}
                              />
                            </div>
                            <span className="text-xs text-muted">
                              {Math.round(item.progress.percent)}%
                            </span>
                            {item.progress.eta && (
                              <span className="text-xs text-muted">
                                {formatEta(item.progress.eta)}
                              </span>
                            )}
                          </div>
                        ) : (
                          <span className="text-xs text-muted">{"\u2014"}</span>
                        )}
                      </td>
                      <td className="hidden lg:table-cell px-3 py-2 text-muted text-xs">
                        {item.downloadClient || "\u2014"}
                      </td>
                      <td className="hidden md:table-cell px-3 py-2 text-muted text-xs" title={item.grabbedAt}>
                        {formatRelativeDate(item.grabbedAt)}
                      </td>
                      <td className="px-3 py-2">
                        <div className="flex items-center gap-1">
                          {!["imported", "removed", "importFailed"].includes(item.status) && (
                            <button
                              onClick={() => setRemoveTarget(item)}
                              className="rounded p-1 text-muted hover:text-red-400"
                              title="Remove"
                            >
                              <Trash2 size={14} />
                            </button>
                          )}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>

            <div className="mt-4">
              <Pagination
                page={page}
                totalPages={totalPages}
                total={total}
                pageSize={data?.perPage ?? 25}
                onPageChange={setPage}
              />
            </div>
          </>
        )}
      </PageContent>

      <ConfirmModal
        open={removeTarget !== null}
        onOpenChange={(open) => !open && setRemoveTarget(null)}
        title="Remove from Queue"
        description={`Remove "${removeTarget?.title}" from the queue?`}
        confirmLabel="Remove"
        variant="danger"
        onConfirm={() => {
          if (removeTarget) return removeMutation.mutateAsync(removeTarget.id);
        }}
      />
    </>
  );
}
