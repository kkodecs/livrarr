import { useState, useMemo, useRef, useEffect } from "react";
import { useParams, useNavigate, useSearchParams } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import * as Tabs from "@radix-ui/react-tabs";
import { RefreshCw, Pencil, Trash2, GitMerge } from "lucide-react";
import {
  getWork,
  refreshWork,
  updateWork,
  deleteWork,
  getQueue,
} from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { PendingAnchorBanner } from "@/components/PendingAnchorBanner";
import { nextEnrichmentPollIntervalMs } from "@/utils/enrichmentPoll";
import type { UpdateWorkRequest } from "@/types/api";

import { WorkHeader } from "./components/WorkHeader";
import { TabTrigger } from "./components/TabTrigger";
import { LibraryFilesTab } from "./components/LibraryFilesTab";
import { ReleasesTab } from "./components/ReleasesTab";
import { HistoryTab } from "./components/HistoryTab";
import { BookInformationTab } from "./components/BookInformationTab";
import { EditModal } from "./components/EditModal";
import { MergeDialog } from "./components/MergeDialog";

export default function WorkDetailPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const queryClient = useQueryClient();
  const initialTab = searchParams.get("tab") ?? "files";

  const [coverPollBaseline, setCoverPollBaseline] = useState<
    { ebook: number | null; audiobook: number | null } | undefined
  >(undefined);
  // D-006: wall-clock start of the current enrichment run. Set/read inside
  // refetchInterval so it tracks real elapsed time across polls, not React's
  // render cadence; reset to null whenever the work isn't enriching.
  const enrichingSinceRef = useRef<number | null>(null);

  const {
    data: work,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["work", id],
    queryFn: () => getWork(Number(id)),
    enabled: !!id,
    refetchInterval: (query) => {
      if (query.state.data?.enriching) {
        if (enrichingSinceRef.current === null) {
          enrichingSinceRef.current = Date.now();
        }
        const nextInterval = nextEnrichmentPollIntervalMs(Date.now() - enrichingSinceRef.current);
        if (nextInterval !== false) return nextInterval;
        // Hard cap reached — fall through and stop; the pill degrades to
        // "attention" (see pillEnriching below) instead of trusting a
        // frozen enriching=true forever.
      } else {
        enrichingSinceRef.current = null;
      }
      if (coverPollBaseline === undefined) return false;
      const ebook = query.state.data?.coverMtime ?? null;
      const audiobook = query.state.data?.audiobookCoverMtime ?? null;
      const unchanged =
        ebook === coverPollBaseline.ebook &&
        audiobook === coverPollBaseline.audiobook;
      return unchanged ? 1_000 : false;
    },
  });

  // Effective "still usefully in flight" signal for the header pill: true
  // only while enriching AND we haven't given up polling (D-006 60s cap /
  // REQ-008). Skeletons elsewhere key off the raw work.enriching instead.
  const enrichingElapsedMs =
    enrichingSinceRef.current === null ? 0 : Date.now() - enrichingSinceRef.current;
  const pillEnriching =
    !!work?.enriching && nextEnrichmentPollIntervalMs(enrichingElapsedMs) !== false;

  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [mergeOpen, setMergeOpen] = useState(false);

  useEffect(() => {
    if (coverPollBaseline === undefined) return;
    const ebook = work?.coverMtime ?? null;
    const audiobook = work?.audiobookCoverMtime ?? null;
    if (
      ebook !== coverPollBaseline.ebook ||
      audiobook !== coverPollBaseline.audiobook
    ) {
      setCoverPollBaseline(undefined);
    }
  }, [work?.coverMtime, work?.audiobookCoverMtime, coverPollBaseline]);

  const refreshMutation = useMutation({
    mutationFn: () => refreshWork(Number(id)),
    onSuccess: () => {
      toast.success("Work refreshed");
      setCoverPollBaseline({
        ebook: work?.coverMtime ?? null,
        audiobook: work?.audiobookCoverMtime ?? null,
      });
      queryClient.invalidateQueries({ queryKey: ["work", id] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Failed to refresh work"),
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteWork(Number(id)),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["works"] });
      toast.success("Work deleted");
      navigate("/");
    },
    onError: () => toast.error("Failed to delete work"),
  });

  const { data: queueItems } = useQuery({
    queryKey: ["queue"],
    queryFn: () => getQueue(),
    select: (res) => res.items,
    refetchInterval: 30_000,
  });

  const activeGrabs = useMemo(() => {
    const set = new Set<string>();
    queueItems?.forEach((item) => {
      if (["sent", "confirmed", "importing"].includes(item.status) && item.mediaType) {
        set.add(`${item.workId}-${item.mediaType}`);
      }
    });
    return set;
  }, [queueItems]);

  const toggleMonitorMutation = useMutation({
    mutationFn: (req: UpdateWorkRequest) => updateWork(Number(id), req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["work", id] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Failed to update monitoring"),
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;
  if (!work) return <ErrorState error={new Error("Work not found")} />;

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-2">
          <button
            onClick={() => refreshMutation.mutate()}
            disabled={refreshMutation.isPending}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <RefreshCw size={14} className={refreshMutation.isPending ? "animate-spin" : ""} />
            Refresh
          </button>
          <button
            onClick={() => setEditOpen(true)}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <Pencil size={14} />
            Edit
          </button>
          <button
            onClick={() => setMergeOpen(true)}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <GitMerge size={14} />
            Merge Duplicate
          </button>
          <button
            onClick={() => setDeleteOpen(true)}
            className="btn-secondary inline-flex items-center gap-1.5 text-red-400 hover:text-red-300"
          >
            <Trash2 size={14} />
            Delete
          </button>
        </div>
      </PageToolbar>

      <PageContent>
        <WorkHeader
          work={work}
          activeGrabs={activeGrabs}
          onToggleMonitor={(field) =>
            toggleMonitorMutation.mutate({
              [field]: !work[field],
            } as UpdateWorkRequest)
          }
          onEditCover={() => setEditOpen(true)}
          pillEnriching={pillEnriching}
          onRefresh={() => refreshMutation.mutate()}
          refreshing={refreshMutation.isPending}
        />

        <PendingAnchorBanner workId={work.id} />

        <Tabs.Root defaultValue={initialTab} className="mt-6">
          <Tabs.List className="flex overflow-x-auto border-b border-border">
            <TabTrigger value="files">Library Files</TabTrigger>
            <TabTrigger value="releases">Search</TabTrigger>
            <TabTrigger value="history">History</TabTrigger>
            <TabTrigger value="metadata">Book Information</TabTrigger>
          </Tabs.List>

          <Tabs.Content value="files" className="mt-4">
            <LibraryFilesTab work={work} />
          </Tabs.Content>
          <Tabs.Content value="releases" className="mt-4">
            <ReleasesTab workId={work.id} />
          </Tabs.Content>
          <Tabs.Content value="history" className="mt-4">
            <HistoryTab workId={work.id} />
          </Tabs.Content>
          <Tabs.Content value="metadata" className="mt-4">
            <BookInformationTab
              work={work}
              onRefresh={() => refreshMutation.mutate()}
              refreshing={refreshMutation.isPending}
              onMergeWorks={() => setMergeOpen(true)}
            />
          </Tabs.Content>
        </Tabs.Root>
      </PageContent>

      <EditModal work={work} open={editOpen} onOpenChange={setEditOpen} onCoverUploaded={() => refetch()} />

      <ConfirmModal
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title="Delete Work"
        description={`Permanently delete "${work.title}" and all associated files on disk? This cannot be undone.`}
        confirmLabel="Delete"
        variant="danger"
        onConfirm={async () => {
          await deleteMutation.mutateAsync();
        }}
      />

      <MergeDialog work={work} open={mergeOpen} onOpenChange={setMergeOpen} />
    </>
  );
}
