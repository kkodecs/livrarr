import { useState, useMemo, useRef, useEffect } from "react";
import { HelpTip } from "@/components/HelpTip";
import { Link, useParams, useNavigate, useSearchParams } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import * as Tabs from "@radix-ui/react-tabs";
import {
  RefreshCw,
  Search,
  Pencil,
  Trash2,
  Download,
  Check,
  Loader2,
  Book,
  BookOpen,
  Play,
  Headphones,
  ExternalLink,
  Clock,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Mail,
  ImagePlus,
  Upload,
} from "lucide-react";
import {
  getWork,
  refreshWork,
  updateWork,
  deleteWork,
  uploadWorkCover,
  getCoverAlternatives,
  selectCover,
  type CoverCandidate,
  searchReleases,
  grabRelease,
  getHistory,
  deleteLibraryFile,
  getMediaManagementConfig,
  sendFileEmail,
  getQueue,
} from "@/api";
import { computeTotalPages } from "@/utils/pagination";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { FormModal } from "@/components/Page/FormModal";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { cn } from "@/utils/cn";
import { useSort } from "@/hooks/useSort";
import { SortHeader } from "@/components/Page/SortHeader";
import {
  formatBytes,
  formatRelativeDate,
  formatDuration,
} from "@/utils/format";
import { BookCover } from "@/components/BookCover";
import ProgressBadge from "@/components/ProgressBadge";

type ReleaseSortField = "title" | "indexer" | "size" | "seeders" | "leechers" | "publishDate";
import type {
  WorkDetailResponse,
  UpdateWorkRequest,
  ReleaseResponse,
  HistoryResponse,
  EnrichmentStatus,
  IdentityStatus,
} from "@/types/api";


interface EditForm {
  title: string;
  authorName: string;
  seriesName: string;
  seriesPosition: string;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
}

export default function WorkDetailPage() {
  const { id } = useParams();
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const queryClient = useQueryClient();
  const initialTab = searchParams.get("tab") ?? "files";

  const [coverPollBaseline, setCoverPollBaseline] = useState<
    { ebook: number | null; audiobook: number | null } | undefined
  >(undefined);

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
      const status = query.state.data?.enrichmentStatus;
      if (status === "unenriched") return 3_000;
      if (coverPollBaseline === undefined) return false;
      const ebook = query.state.data?.coverMtime ?? null;
      const audiobook = query.state.data?.audiobookCoverMtime ?? null;
      const unchanged =
        ebook === coverPollBaseline.ebook &&
        audiobook === coverPollBaseline.audiobook;
      return unchanged ? 1_000 : false;
    },
  });

  const [editOpen, setEditOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);

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
        />

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
            <BookInformationTab work={work} onRefresh={() => refreshMutation.mutate()} refreshing={refreshMutation.isPending} />
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

    </>
  );
}

// --- Header ---

function WorkHeader({
  work,
  activeGrabs,
  onToggleMonitor,
  onEditCover,
}: {
  work: WorkDetailResponse;
  activeGrabs: Set<string>;
  onToggleMonitor: (field: "monitorEbook" | "monitorAudiobook") => void;
  onEditCover: () => void;
}) {
  const ebookItems = work.libraryItems?.filter((li) => li.mediaType === "ebook") ?? [];
  const audioItems = work.libraryItems?.filter((li) => li.mediaType === "audiobook") ?? [];
  const ebookSize = ebookItems.reduce((acc, li) => acc + li.fileSize, 0);
  const audioSize = audioItems.reduce((acc, li) => acc + li.fileSize, 0);
  const ebookDownloading = activeGrabs.has(`${work.id}-ebook`);
  const audioDownloading = activeGrabs.has(`${work.id}-audiobook`);

  function monitorStatus(
    monitored: boolean,
    hasFile: boolean,
    fileSize: number,
    downloading: boolean,
  ): { color: string; label: string } {
    if (!monitored) return { color: "text-zinc-600", label: "Not Monitored" };
    if (hasFile) return { color: "text-green-400", label: formatBytes(fileSize) };
    if (downloading) return { color: "text-purple-400", label: "Downloading" };
    return { color: "text-amber-500", label: "Missing" };
  }

  const ebook = monitorStatus(work.monitorEbook, ebookItems.length > 0, ebookSize, ebookDownloading);
  const audio = monitorStatus(work.monitorAudiobook, audioItems.length > 0, audioSize, audioDownloading);

  const hasAudioFiles = audioItems.length > 0;
  const hasDedicatedAudioCover = !!work.audiobookCoverUrl;
  const showSeparateAudioCover = hasAudioFiles && hasDedicatedAudioCover;

  return (
    <div className="flex flex-col items-center gap-4 sm:flex-row sm:items-start sm:gap-6">
      <div className="flex gap-3 flex-shrink-0 order-first sm:order-last">
        <div className="flex flex-col items-center gap-1">
          <div className="relative group h-[200px] w-[133px] sm:h-[300px] sm:w-[200px]">
            <BookCover
              workId={work.id}
              title={work.title}
              authorName={work.authorName}
              coverVersion={work.coverMtime ?? undefined}
              className="h-full w-full rounded-lg shadow-lg"
              iconSize={32}
            />
            {(ebookItems.length > 0 || (audioItems.length > 0 && !showSeparateAudioCover)) && (
              <div className="absolute inset-0 flex items-center justify-center gap-4 opacity-0 group-hover:opacity-100 transition-opacity bg-black/40 rounded-lg">
                {ebookItems[0] && (
                  <Link
                    to={`/read/${ebookItems[0].id}`}
                    className="rounded-full bg-black/60 p-3 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
                  >
                    <BookOpen size={24} />
                  </Link>
                )}
                {audioItems[0] && !showSeparateAudioCover && (
                  <Link
                    to={`/listen/${audioItems[0].id}?workId=${work.id}`}
                    className="rounded-full bg-black/60 p-3 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
                  >
                    <Headphones size={24} />
                  </Link>
                )}
              </div>
            )}
          </div>
          <span className="text-[10px] text-zinc-500">{hasAudioFiles && !hasDedicatedAudioCover ? "Ebook/Audiobook Cover" : "Ebook Cover"}</span>
          <button
            type="button"
            onClick={onEditCover}
            className="text-[10px] text-brand hover:text-brand/80"
          >
            Update
          </button>
        </div>
        {showSeparateAudioCover && (
          <div className="hidden sm:flex flex-col items-center gap-1">
            <div className="relative group h-[200px] w-[133px] sm:h-[300px] sm:w-[200px]">
              <BookCover
                workId={work.id}
                title={work.title}
                coverVersion={work.audiobookCoverMtime ?? work.coverMtime ?? undefined}
                mediaType="audiobook"
                className="h-full w-full rounded-lg shadow-lg"
                iconSize={32}
              />
              {audioItems[0] && (
                <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity bg-black/40 rounded-lg">
                  <Link
                    to={`/listen/${audioItems[0].id}?workId=${work.id}`}
                    className="rounded-full bg-black/60 p-3 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
                  >
                    <Headphones size={24} />
                  </Link>
                </div>
              )}
            </div>
            <span className="text-[10px] text-zinc-500">Audiobook Cover</span>
            <button
              type="button"
              onClick={onEditCover}
              className="text-[10px] text-brand hover:text-brand/80"
            >
              Update
            </button>
          </div>
        )}
      </div>
      <div className="min-w-0 flex-1 text-center sm:text-left">
        <div className="flex items-baseline gap-2">
          <h1 className="text-2xl font-bold text-zinc-100">{work.title}</h1>
          <span className="text-xs text-zinc-600">#{work.id}</span>
        </div>

        <div className="mt-2 flex flex-wrap items-center gap-2 text-sm">
          {work.authorId ? (
            <Link
              to={`/author/${work.authorId}`}
              className="text-brand hover:underline"
            >
              {work.authorName}
            </Link>
          ) : (
            <span className="text-muted">{work.authorName}</span>
          )}
          {work.year && <span className="text-muted">({work.year})</span>}
          {work.seriesName && (
            <span className="text-muted">
              {work.seriesName}
              {work.seriesPosition != null && ` #${work.seriesPosition}`}
            </span>
          )}
        </div>

        <div className="mt-3 flex items-center gap-4">
          <button
            onClick={() => onToggleMonitor("monitorEbook")}
            className={cn("inline-flex items-center gap-1.5 text-sm transition-colors hover:opacity-80", ebook.color)}
            title={`Ebook: ${ebook.label}. Click to ${work.monitorEbook ? "stop" : "start"} monitoring.`}
          >
            <Book size={16} />
            <span>{ebook.label}</span>
          </button>
          <button
            onClick={() => onToggleMonitor("monitorAudiobook")}
            className={cn("inline-flex items-center gap-1.5 text-sm transition-colors hover:opacity-80", audio.color)}
            title={`Audiobook: ${audio.label}. Click to ${work.monitorAudiobook ? "stop" : "start"} monitoring.`}
          >
            <Headphones size={16} />
            <span>{audio.label}</span>
          </button>
        </div>

        {work.detailUrl && (
          <a
            href={work.detailUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="mt-2 inline-flex items-center gap-1 text-sm text-brand hover:underline"
          >
            <ExternalLink size={14} />
            View on Goodreads
          </a>
        )}

        {work.genres && work.genres.length > 0 && (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {work.genres.map((genre) => (
              <span
                key={genre}
                className="rounded bg-zinc-700 px-2 py-0.5 text-xs text-zinc-300"
              >
                {genre}
              </span>
            ))}
          </div>
        )}

        {work.description && (
          <p className="mt-4 line-clamp-4 text-sm text-zinc-400">
            {work.description}
          </p>
        )}

      </div>
    </div>
  );
}

// --- Tab trigger helper ---

function TabTrigger({
  value,
  children,
}: {
  value: string;
  children: React.ReactNode;
}) {
  return (
    <Tabs.Trigger
      value={value}
      className="border-b-2 border-transparent px-4 py-2 text-sm font-medium text-muted hover:text-zinc-100 data-[state=active]:border-brand data-[state=active]:text-zinc-100"
    >
      {children}
    </Tabs.Trigger>
  );
}

// --- Library Files Tab ---

// Formats Amazon accepts via Send to Kindle email
const KINDLE_ACCEPTED_FORMATS = new Set([
  "epub", "pdf", "docx", "doc", "rtf", "htm", "html", "txt",
]);

const MAX_EMAIL_SIZE = 50 * 1024 * 1024; // 50 MB

function getFileExtension(path: string): string {
  const dot = path.lastIndexOf(".");
  return dot >= 0 ? path.slice(dot + 1).toLowerCase() : "";
}

const READABLE_FORMATS = new Set(["epub", "pdf"]);
const LISTENABLE_FORMATS = new Set(["m4b", "m4a", "mp3", "flac", "ogg"]);

function LibraryFilesTab({ work }: { work: WorkDetailResponse }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [sendingId, setSendingId] = useState<number | null>(null);
  const [sentIds, setSentIds] = useState<Set<number>>(new Set());

  const deleteFileMutation = useMutation({
    mutationFn: (fileId: number) => deleteLibraryFile(fileId),
    onSuccess: () => {
      toast.success("File deleted");
      queryClient.invalidateQueries({ queryKey: ["work"] });
      setConfirmDelete(null);
    },
    onError: () => toast.error("Failed to delete file"),
  });

  const sendEmailMutation = useMutation({
    mutationFn: sendFileEmail,
    onSuccess: (_data, itemId) => {
      setSendingId(null);
      setSentIds((prev) => new Set(prev).add(itemId));
      toast.success("Sent to Kindle");
    },
    onError: (e: Error) => {
      setSendingId(null);
      toast.error(e.message || "Failed to send email");
    },
  });

  const handleSendEmail = (itemId: number) => {
    setSendingId(itemId);
    sendEmailMutation.mutate(itemId);
  };

  if (work.libraryItems.length === 0) {
    return (
      <EmptyState
        icon={<Book size={24} />}
        title="No library files"
        description="Files will appear here after import."
      />
    );
  }

  return (
    <>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="border-b border-border">
            <tr>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Path
              </th>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Type
              </th>
              <th className="px-3 py-2 text-right text-xs font-medium uppercase text-muted">
                Size
              </th>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Imported
              </th>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Progress
              </th>
              <th className="w-20 px-3 py-2" />
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {work.libraryItems.map((item) => {
              const ext = getFileExtension(item.path);
              const canSend = KINDLE_ACCEPTED_FORMATS.has(ext);
              const tooLarge = item.fileSize > MAX_EMAIL_SIZE;
              const isSending = sendingId === item.id;
              const wasSent = sentIds.has(item.id);

              return (
                <tr key={item.id} className="hover:bg-zinc-800/50">
                  <td
                    className="max-w-md truncate px-3 py-2 font-mono text-xs text-zinc-300"
                    title={item.path}
                  >
                    {item.path}
                  </td>
                  <td className="px-3 py-2">
                    <div className="inline-flex items-center gap-1 text-zinc-400">
                      {item.mediaType === "ebook" ? (
                        <Book size={14} />
                      ) : (
                        <Headphones size={14} />
                      )}
                      <span className="text-xs capitalize">{item.mediaType}</span>
                    </div>
                  </td>
                  <td className="px-3 py-2 text-right text-muted">
                    {formatBytes(item.fileSize)}
                  </td>
                  <td className="px-3 py-2 text-muted">
                    {formatRelativeDate(item.importedAt)}
                  </td>
                  <td className="px-3 py-2">
                    {item.progressPct != null && item.progressPct > 0 ? (
                      <div className="flex items-center gap-2">
                        <div className="w-16 h-1.5 bg-zinc-700 rounded-full overflow-hidden">
                          <div
                            className="h-full bg-brand rounded-full"
                            style={{ width: `${Math.min(item.progressPct * 100, 100)}%` }}
                          />
                        </div>
                        <ProgressBadge
                          progressPct={item.progressPct}
                          mediaType={item.mediaType}
                          durationSeconds={item.durationSeconds}
                          finishedAt={item.finishedAt}
                        />
                      </div>
                    ) : null}
                  </td>
                  <td className="px-3 py-2 flex items-center justify-end gap-1">
                    {READABLE_FORMATS.has(ext) && (
                      <button
                        onClick={() => navigate(`/read/${item.id}`)}
                        className="rounded p-1 text-muted hover:text-brand"
                        title="Read"
                      >
                        <BookOpen size={14} />
                      </button>
                    )}
                    {LISTENABLE_FORMATS.has(ext) && (
                      <button
                        onClick={() => navigate(`/listen/${item.id}?workId=${work.id}`)}
                        className="rounded p-1 text-muted hover:text-brand"
                        title="Listen"
                      >
                        <Play size={14} />
                      </button>
                    )}
                    {canSend && (
                      <button
                        onClick={() => handleSendEmail(item.id)}
                        disabled={isSending || tooLarge}
                        className={`rounded p-1 hover:text-brand disabled:opacity-40 ${tooLarge ? "disabled:cursor-not-allowed text-muted" : isSending ? "cursor-wait text-brand" : wasSent ? "text-green-400" : "text-muted"}`}
                        title={
                          tooLarge
                            ? `File too large (${formatBytes(item.fileSize)}). Amazon limit is 50 MB.`
                            : wasSent
                              ? "Sent to Kindle"
                              : "Send to Kindle"
                        }
                      >
                        {isSending ? (
                          <Loader2 size={14} className="animate-spin text-brand" />
                        ) : (
                          <Mail size={14} className={wasSent ? "text-green-400" : ""} />
                        )}
                      </button>
                    )}
                    <button
                      onClick={() => setConfirmDelete(item.id)}
                      className="rounded p-1 text-muted hover:text-red-400"
                      title="Delete file"
                    >
                      <Trash2 size={14} />
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <ConfirmModal
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
        title="Delete File"
        description="Are you sure you want to delete this library file?"
        confirmLabel="Delete"
        variant="danger"
        onConfirm={() => {
          if (confirmDelete !== null)
            return deleteFileMutation.mutateAsync(confirmDelete);
        }}
      />
    </>
  );
}

// --- Releases Tab ---

function ReleasesTab({ workId }: { workId: number }) {
  const queryClient = useQueryClient();
  const [ebookFormatFilter, setEbookFormatFilter] = useState<Set<string> | null>(null);
  const [audiobookFormatFilter, setAudiobookFormatFilter] = useState<Set<string> | null>(null);

  // Mode ref controls what the queryFn does:
  // 'cacheCheck' = ask backend for cached results only (no indexer hits) — used on mount
  // 'search'     = full search hitting all indexers
  // 'refresh'    = full search bypassing backend cache
  const modeRef = useRef<"cacheCheck" | "search" | "refresh">("search");
  const [hasSearched, setHasSearched] = useState(false);
  const {
    data: searchResponse,
    fetchStatus,
    dataUpdatedAt,
    refetch,
    isError,
    error,
  } = useQuery({
    queryKey: ["releases", workId],
    queryFn: () => {
      const mode = modeRef.current;
      modeRef.current = "cacheCheck";
      if (mode === "refresh") return searchReleases(workId, { refresh: true });
      if (mode === "search") return searchReleases(workId);
      return searchReleases(workId, { cacheOnly: true });
    },
    staleTime: Infinity,
    gcTime: 30 * 60 * 1000,
    retry: false,
  });
  const isLoading = fetchStatus === "fetching";
  const hasResults = (searchResponse?.results?.length ?? 0) > 0;

  // Mark searched when cache returns results (so we skip the "Search" prompt).
  useEffect(() => {
    if (hasResults) setHasSearched(true);
  }, [hasResults]);

  // Live-updating cache age from React Query's own timestamp.
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (!dataUpdatedAt) return;
    const id = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(id);
  }, [dataUpdatedAt]);

  const { data: mmConfig } = useQuery({
    queryKey: ["mediaManagementConfig"],
    queryFn: getMediaManagementConfig,
  });

  // Initialize filters from preferences once loaded.
  // If no preferred formats have results, default to all formats active.
  const ebookPrefs = mmConfig?.preferredEbookFormats ?? ["epub"];
  const audiobookPrefs = mmConfig?.preferredAudiobookFormats ?? ["m4b"];
  const ebookPrefsSet = new Set(ebookPrefs);
  const audiobookPrefsSet = new Set(audiobookPrefs);

  const allEbookFormats = ["epub", "mobi", "azw3", "pdf", "cbz", "cbr"];
  const allAudiobookFormats = ["m4b", "m4a", "mp3", "flac", "ogg", "wma"];

  // Placeholder — orderedFormats computed after release splitting below.
  let orderedEbookFormats: string[] = [];
  let orderedAudiobookFormats: string[] = [];

  const [ebooksOpen, setEbooksOpen] = useState(true);
  const [audiobooksOpen, setAudiobooksOpen] = useState(true);
  const [grabbedGuids, setGrabbedGuids] = useState<Set<string>>(new Set());
  const [grabbingGuid, setGrabbingGuid] = useState<string | null>(null);

  const grabMutation = useMutation({
    mutationFn: (release: ReleaseResponse) => {
      setGrabbingGuid(release.guid);
      return grabRelease({
        workId,
        downloadUrl: release.downloadUrl,
        title: release.title,
        indexer: release.indexer,
        guid: release.guid,
        size: release.size,
        protocol: release.protocol,
        categories: release.categories,
      });
    },
    onSuccess: (_data, release) => {
      setGrabbedGuids((prev) => new Set(prev).add(release.guid));
      setGrabbingGuid(null);
      queryClient.invalidateQueries({ queryKey: ["queue"] });
      toast.success("Release grabbed");
    },
    onError: (e: Error) => {
      setGrabbingGuid(null);
      toast.error(e.message || "Failed to grab release");
    },
  });

  const releases = searchResponse?.results ?? [];
  const warnings = searchResponse?.warnings ?? [];

  // Split by category: 7000s = ebook, 3000s = audiobook.
  const ebookReleases = releases.filter(
    (r) => r.categories.some((c) => c >= 7000 && c < 8000),
  );
  const audiobookReleases = releases.filter(
    (r) => r.categories.some((c) => c >= 3000 && c < 4000),
  );
  const uncategorized = releases.filter(
    (r) =>
      !r.categories.some((c) => c >= 7000 && c < 8000) &&
      !r.categories.some((c) => c >= 3000 && c < 4000),
  );

  // Only show format chips for formats that have at least one release.
  const detectFormatsInReleases = (items: ReleaseResponse[], formats: string[]) =>
    formats.filter((fmt) => items.some((r) => r.format === fmt));
  orderedEbookFormats = (() => {
    const present = detectFormatsInReleases([...ebookReleases, ...uncategorized], allEbookFormats);
    return [...ebookPrefs.filter((f) => present.includes(f)), ...present.filter((f) => !ebookPrefs.includes(f))];
  })();
  orderedAudiobookFormats = (() => {
    const present = detectFormatsInReleases(audiobookReleases, allAudiobookFormats);
    return [...audiobookPrefs.filter((f) => present.includes(f)), ...present.filter((f) => !audiobookPrefs.includes(f))];
  })();

  // Default active formats: preferred if any have results, otherwise all present.
  const ebookDefault = orderedEbookFormats.some((f) => ebookPrefsSet.has(f))
    ? new Set(ebookPrefs)
    : new Set(orderedEbookFormats);
  const audiobookDefault = orderedAudiobookFormats.some((f) => audiobookPrefsSet.has(f))
    ? new Set(audiobookPrefs)
    : new Set(orderedAudiobookFormats);
  const activeEbookFormats = ebookFormatFilter ?? ebookDefault;
  const activeAudiobookFormats = audiobookFormatFilter ?? audiobookDefault;

  // Filter by selected formats. Detect format from title, then check if it's selected.
  // Releases with no detectable format are always shown.

  const filterByFormat = (
    items: ReleaseResponse[],
    formats: Set<string>,
  ) => {
    if (formats.size === 0) return items;
    return items.filter((r) => {
      if (!r.format) return true;
      return formats.has(r.format);
    });
  };

  const sorting = useSort<ReleaseSortField>("seeders", "desc");
  const sortFn = (item: ReleaseResponse, field: ReleaseSortField) => {
    switch (field) {
      case "title": return item.title;
      case "indexer": return item.indexer;
      case "size": return item.size;
      case "seeders": return item.seeders ?? -1;
      case "leechers": return item.leechers ?? -1;
      case "publishDate": return item.publishDate ?? "";
    }
  };

  const filteredEbooks = filterByFormat([...ebookReleases, ...uncategorized], activeEbookFormats);
  const filteredAudiobooks = filterByFormat(audiobookReleases, activeAudiobookFormats);
  const sortedEbooks = sorting.sort(filteredEbooks, sortFn);
  const sortedAudiobooks = sorting.sort(filteredAudiobooks, sortFn);

  const toggleFormat = (
    current: Set<string>,
    setter: (s: Set<string>) => void,
    fmt: string,
  ) => {
    const next = new Set(current);
    if (next.has(fmt)) {
      next.delete(fmt);
    } else {
      next.add(fmt);
    }
    setter(next);
  };

  const runQuery = (mode: "search" | "refresh") => {
    modeRef.current = mode;
    setHasSearched(true);
    refetch();
  };
  const doSearch = () => runQuery("search");

  // Error state — show if query failed and we have no prior results.
  if (isError && !hasResults) {
    return (
      <EmptyState
        icon={<Search size={24} />}
        title="Failed to load releases"
        description={(error as Error)?.message || "An error occurred"}
        action={
          <button
            onClick={doSearch}
            disabled={isLoading}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <RefreshCw size={14} />
            Retry
          </button>
        }
      />
    );
  }

  // No results yet and haven't searched — show search prompt.
  if (!hasResults && !isLoading && !hasSearched) {
    return (
      <div className="flex flex-col items-center py-12">
        <button
          onClick={doSearch}
          disabled={isLoading}
          className="btn-primary inline-flex items-center gap-1.5"
        >
          <Search size={14} />
          Search Releases
        </button>
      </div>
    );
  }

  if (!hasResults && isLoading) return <PageLoading />;

  // Searched but got 0 results — show empty state with retry.
  if (releases.length === 0 && warnings.length === 0) {
    return (
      <EmptyState
        icon={<Search size={24} />}
        title="No releases found"
        action={
          <button
            onClick={doSearch}
            disabled={isLoading}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <RefreshCw size={14} />
            Search Again
          </button>
        }
      />
    );
  }

  const renderTable = (items: ReleaseResponse[]) => (
    <PaginatedReleaseTable
      items={items}
      sorting={sorting}
      grabbedGuids={grabbedGuids}
      grabbingGuid={grabbingGuid}
      grabMutation={grabMutation}
    />
  );

  const handleRefresh = () => runQuery("refresh");
  const cacheAgeSecs = dataUpdatedAt
    ? (searchResponse?.cacheAgeSeconds ?? 0) + Math.floor((now - dataUpdatedAt) / 1000)
    : null;

  const formatCacheAge = (secs: number) => {
    if (secs < 60) return "just now";
    if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
    if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
    return `${Math.floor(secs / 86400)}d ago`;
  };

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-end gap-3">
        {searchResponse?.searchQuery && (
          <span className="text-xs text-muted">
            Results for &lsquo;{searchResponse.searchQuery}&rsquo;
            {cacheAgeSecs != null && <> &middot; cached {formatCacheAge(cacheAgeSecs)}</>}
          </span>
        )}
        <button
          onClick={handleRefresh}
          disabled={isLoading}
          className="btn-secondary inline-flex items-center gap-1.5 text-xs"
        >
          <RefreshCw size={12} className={isLoading ? "animate-spin" : ""} />
          Refresh
        </button>
      </div>
      {isError && hasResults && (
        <div className="rounded border border-red-500/30 bg-red-500/10 p-3">
          <p className="text-sm text-red-400">
            Failed to update results: {(error as Error)?.message || "An error occurred"}. Showing previously cached results.
          </p>
        </div>
      )}
      {warnings.length > 0 && (
        <div className="rounded border border-amber-500/30 bg-amber-500/10 p-3">
          {warnings.map((w, i) => (
            <p key={i} className="text-sm text-amber-400">
              <span className="font-medium">{w.indexer}:</span> {w.error}
            </p>
          ))}
        </div>
      )}

      {releases.length > 0 && (
        <section>
          <div className="mb-2 flex items-center gap-3">
            <button
              onClick={() => setEbooksOpen((o) => !o)}
              className="flex items-center gap-1.5 text-sm font-semibold text-zinc-100 hover:text-zinc-300"
            >
              {ebooksOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              <Book size={14} />
              Ebooks ({sortedEbooks.length}{filteredEbooks.length !== ebookReleases.length + uncategorized.length ? ` of ${ebookReleases.length + uncategorized.length}` : ""})
            </button>
            {ebooksOpen && (
              <div className="flex items-center gap-2">
                {orderedEbookFormats.map((fmt) => (
                  <label
                    key={fmt}
                    className={cn(
                      "flex items-center gap-1 rounded px-1.5 py-0.5 text-xs cursor-pointer",
                      activeEbookFormats.has(fmt)
                        ? ebookPrefsSet.has(fmt)
                          ? "bg-brand/20 text-brand"
                          : "bg-amber-500/20 text-amber-400"
                        : "bg-zinc-800 text-zinc-500 hover:text-zinc-400",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={activeEbookFormats.has(fmt)}
                      onChange={() => toggleFormat(activeEbookFormats, setEbookFormatFilter, fmt)}
                      className="sr-only"
                    />
                    .{fmt}
                  </label>
                ))}
              </div>
            )}
          </div>
          {ebooksOpen && (
            sortedEbooks.length > 0 ? renderTable(sortedEbooks) : (
              <p className="text-sm text-muted py-2">{ebookReleases.length + uncategorized.length === 0 ? "No ebook releases found." : "No results match selected formats."}</p>
            )
          )}
        </section>
      )}

      {releases.length > 0 && (
        <section>
          <div className="mb-2 flex items-center gap-3">
            <button
              onClick={() => setAudiobooksOpen((o) => !o)}
              className="flex items-center gap-1.5 text-sm font-semibold text-zinc-100 hover:text-zinc-300"
            >
              {audiobooksOpen ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
              <Headphones size={14} />
              Audiobooks ({sortedAudiobooks.length}{filteredAudiobooks.length !== audiobookReleases.length ? ` of ${audiobookReleases.length}` : ""})
            </button>
            {audiobooksOpen && (
              <div className="flex items-center gap-2">
                {orderedAudiobookFormats.map((fmt) => (
                  <label
                    key={fmt}
                    className={cn(
                      "flex items-center gap-1 rounded px-1.5 py-0.5 text-xs cursor-pointer",
                      activeAudiobookFormats.has(fmt)
                        ? audiobookPrefsSet.has(fmt)
                          ? "bg-brand/20 text-brand"
                          : "bg-amber-500/20 text-amber-400"
                        : "bg-zinc-800 text-zinc-500 hover:text-zinc-400",
                    )}
                  >
                    <input
                      type="checkbox"
                      checked={activeAudiobookFormats.has(fmt)}
                      onChange={() => toggleFormat(activeAudiobookFormats, setAudiobookFormatFilter, fmt)}
                      className="sr-only"
                    />
                    .{fmt}
                  </label>
                ))}
              </div>
            )}
          </div>
          {audiobooksOpen && (
            sortedAudiobooks.length > 0 ? renderTable(sortedAudiobooks) : (
              <p className="text-sm text-muted py-2">{audiobookReleases.length === 0 ? "No audiobook releases found." : "No results match selected formats."}</p>
            )
          )}
        </section>
      )}

      {ebookReleases.length === 0 && uncategorized.length === 0 && audiobookReleases.length === 0 && (
        <EmptyState
          icon={<Search size={24} />}
          title="No releases found"
          action={
            <button
              onClick={() => refetch()}
              className="btn-secondary inline-flex items-center gap-1.5"
            >
              <RefreshCw size={14} />
              Search Again
            </button>
          }
        />
      )}
    </div>
  );
}

// --- History Tab ---

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
};

// --- Book Information Tab ---

function MetadataRow({ label, value }: { label: string; value: React.ReactNode }) {
  if (!value) return null;
  return (
    <div className="flex gap-4 py-2 border-b border-border/30">
      <dt className="w-36 shrink-0 text-xs text-muted uppercase tracking-wide">{label}</dt>
      <dd className="text-sm text-zinc-200">{value}</dd>
    </div>
  );
}

type BadgeTone = "green" | "blue" | "amber" | "red" | "orange" | "zinc";

const BADGE_TONE: Record<BadgeTone, { wrap: string; dot: string }> = {
  green: { wrap: "text-green-400 bg-green-500/10 border-green-500/30", dot: "bg-green-500" },
  blue: { wrap: "text-blue-400 bg-blue-500/10 border-blue-500/30", dot: "bg-blue-500" },
  amber: { wrap: "text-amber-400 bg-amber-500/10 border-amber-500/30", dot: "bg-amber-500" },
  red: { wrap: "text-red-400 bg-red-500/10 border-red-500/30", dot: "bg-red-500" },
  orange: { wrap: "text-orange-400 bg-orange-500/10 border-orange-500/30", dot: "bg-orange-500" },
  zinc: { wrap: "text-zinc-400 bg-zinc-500/10 border-zinc-500/25", dot: "bg-zinc-500" },
};

function StatusBadge({ tone, label, tip }: { tone: BadgeTone; label: string; tip: string }) {
  const t = BADGE_TONE[tone];
  return (
    <span className={cn("inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium", t.wrap)}>
      <span className={cn("h-1.5 w-1.5 rounded-full", t.dot)} />
      {label}
      <HelpTip text={tip} />
    </span>
  );
}

// Identity state machine (REQ-014) — "which book is this?". The section header
// supplies the context a bare floating badge lacks.
const IDENTITY_BADGE: Record<IdentityStatus, { tone: BadgeTone; label: string; tip: string }> = {
  pending: { tone: "amber", label: "Pending", tip: "Still matching — only a fuzzy title/author guess so far." },
  confirmed: { tone: "green", label: "Confirmed", tip: "Locked to a master catalog record." },
  provisional: { tone: "blue", label: "Provisional", tip: "Identified by ISBN (barcode); no master record yet — may later upgrade to Confirmed." },
  conflict: { tone: "red", label: "Conflict", tip: "Sources disagree on the match; needs your review." },
  needs_review: { tone: "orange", label: "Needs Review", tip: "Couldn't match this book automatically; needs your review." },
  not_found: { tone: "amber", label: "Unverified", tip: "No source could confirm this match — every provider was rejected. Edit the identity or refresh to retry." },
};

// Enrichment (details) state machine — "what do we know about it?". The canonical
// outcomes are Pending/Enriched/Sparse (+ Failed). Identity outcomes (incl. the
// "unverified" not-found case) live on the Identity badge above, not here.
const DETAILS_BADGE: Record<EnrichmentStatus, { tone: BadgeTone; label: string; tip: string }> = {
  unenriched: { tone: "amber", label: "Pending", tip: "Details haven't been fetched yet." },
  enriched: { tone: "green", label: "Enriched", tip: "Real information is present. A cover is a separate lazy asset and isn't required." },
  thin: { tone: "zinc", label: "Sparse", tip: "Known book, but providers returned almost nothing — a settled result, not still loading." },
  failed: { tone: "red", label: "Failed", tip: "A lookup error occurred while fetching details. Try refreshing." },
};

function BookInformationTab({
  work,
  onRefresh,
  refreshing,
}: {
  work: WorkDetailResponse;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const identity = IDENTITY_BADGE[work.identityStatus];
  const details = DETAILS_BADGE[work.enrichmentStatus];
  const missing = <span className="text-zinc-600">—</span>;

  return (
    <div className="max-w-2xl">
      {/* Identity — which book this is */}
      <section>
        <div className="flex items-center justify-between gap-4">
          <h3 className="text-sm font-medium text-zinc-100">Identity</h3>
          <button
            onClick={onRefresh}
            disabled={refreshing}
            className="btn-secondary inline-flex items-center gap-1.5 text-xs"
          >
            <RefreshCw size={12} className={cn(refreshing && "animate-spin")} />
            Refresh
          </button>
        </div>
        <p className="mt-0.5 mb-3 text-xs text-muted">Which book this is — catalog match &amp; identifiers.</p>
        <StatusBadge tone={identity.tone} label={identity.label} tip={identity.tip} />
        <dl className="mt-4">
          <MetadataRow label="Open Library" value={work.olKey || missing} />
          <MetadataRow label="Hardcover" value={work.hcKey || missing} />
          <MetadataRow label="Goodreads" value={work.grKey || missing} />
          <MetadataRow label="ISBN-13" value={work.isbn13 || missing} />
          <MetadataRow label="ASIN" value={work.asin || missing} />
        </dl>
      </section>

      {/* Details — what we know about it */}
      <section className="mt-7 pt-6 border-t border-border">
        <h3 className="text-sm font-medium text-zinc-100">Details</h3>
        <p className="mt-0.5 mb-3 text-xs text-muted">What we know about it — series, genres, publisher, cover.</p>
        <StatusBadge tone={details.tone} label={details.label} tip={details.tip} />
        <dl className="mt-4">
          {work.originalTitle && <MetadataRow label="Original title" value={work.originalTitle} />}
          <MetadataRow label="Year" value={work.year} />
          {work.seriesName && (
            <MetadataRow label="Series" value={
              `${work.seriesName}${work.seriesPosition != null ? ` #${work.seriesPosition}` : ""}`
            } />
          )}
          {work.genres && work.genres.length > 0 && (
            <MetadataRow label="Genres" value={work.genres.join(", ")} />
          )}
          <MetadataRow label="Publisher" value={work.publisher} />
          <MetadataRow label="Publish date" value={work.publishDate} />
          <MetadataRow label="Language" value={work.language?.toUpperCase()} />
          <MetadataRow label="Pages" value={work.pageCount} />
          {work.durationSeconds && (
            <MetadataRow label="Duration" value={formatDuration(work.durationSeconds)} />
          )}
          {work.narrator && work.narrator.length > 0 && (
            <MetadataRow label="Narrator" value={work.narrator.join(", ")} />
          )}
          {work.narrationType && <MetadataRow label="Narration" value={work.narrationType} />}
          {work.abridged && <MetadataRow label="Abridged" value="Yes" />}
          {work.rating != null && (
            <MetadataRow label="Rating" value={
              `${work.rating.toFixed(1)}/5${work.ratingCount != null ? ` (${work.ratingCount} ratings)` : ""}`
            } />
          )}
          <MetadataRow label="Source" value={work.enrichmentSource} />
          {work.enrichedAt && (
            <MetadataRow label="Last enriched" value={formatRelativeDate(work.enrichedAt)} />
          )}
        </dl>
        <p className="mt-3 text-xs text-muted">
          The cover is fetched separately and may appear after this — it never affects the Enriched status.
        </p>
      </section>
    </div>
  );
}

function HistoryTab({ workId }: { workId: number }) {
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
  return "";
}

// --- Edit Modal ---

function EditModal({
  work,
  open,
  onOpenChange,
  onCoverUploaded,
}: {
  work: WorkDetailResponse;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCoverUploaded?: () => void;
}) {
  const queryClient = useQueryClient();

  const {
    register,
    handleSubmit,
    formState: { isSubmitting },
  } = useForm<EditForm>({
    defaultValues: {
      title: work.title,
      authorName: work.authorName,
      seriesName: work.seriesName ?? "",
      seriesPosition:
        work.seriesPosition != null ? String(work.seriesPosition) : "",
      monitorEbook: work.monitorEbook,
      monitorAudiobook: work.monitorAudiobook,
    },
  });

  const updateMutation = useMutation({
    mutationFn: (req: UpdateWorkRequest) => updateWork(work.id, req),
    onSuccess: () => {
      toast.success("Work updated");
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
      onOpenChange(false);
    },
    onError: () => toast.error("Failed to update work"),
  });

  const onSubmit = (data: EditForm) => {
    const req: UpdateWorkRequest = {
      title: data.title || null,
      authorName: data.authorName || null,
      seriesName: data.seriesName || null,
      seriesPosition: data.seriesPosition ? Number(data.seriesPosition) : null,
      monitorEbook: data.monitorEbook,
      monitorAudiobook: data.monitorAudiobook,
    };
    updateMutation.mutate(req);
  };

  return (
    <FormModal open={open} onOpenChange={onOpenChange} title="Edit Work">
      <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
        <label className="block">
          <span className="mb-1 block text-sm font-medium text-zinc-300">
            Title
          </span>
          <input {...register("title")} className="input-field" />
        </label>
        <label className="block">
          <span className="mb-1 block text-sm font-medium text-zinc-300">
            Author
          </span>
          <input {...register("authorName")} className="input-field" />
        </label>
        <div className="grid grid-cols-2 gap-3">
          <label className="block">
            <span className="mb-1 block text-sm font-medium text-zinc-300">
              Series
            </span>
            <input {...register("seriesName")} className="input-field" />
          </label>
          <label className="block">
            <span className="mb-1 block text-sm font-medium text-zinc-300">
              Position
            </span>
            <input
              {...register("seriesPosition")}
              type="number"
              step="any"
              className="input-field"
            />
          </label>
        </div>

        <div className="flex gap-6">
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input
              type="checkbox"
              {...register("monitorEbook")}
              className="rounded border-border"
            />
            Monitor Ebook
          </label>
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input
              type="checkbox"
              {...register("monitorAudiobook")}
              className="rounded border-border"
            />
            Monitor Audiobook
          </label>
        </div>

        <CoverSection
          work={work}
          onCoverUploaded={onCoverUploaded}
          onClose={() => onOpenChange(false)}
        />

        <div className="flex justify-end gap-3 pt-2">
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={isSubmitting || updateMutation.isPending}
            className="btn-primary"
          >
            {updateMutation.isPending ? "Saving..." : "Save"}
          </button>
        </div>
      </form>
    </FormModal>
  );
}

function CoverSection({
  work,
  onCoverUploaded,
  onClose,
}: {
  work: WorkDetailResponse;
  onCoverUploaded?: () => void;
  onClose: () => void;
}) {
  const [showAlternatives, setShowAlternatives] = useState(false);
  const queryClient = useQueryClient();

  const altQuery = useQuery({
    queryKey: ["coverAlternatives", work.id],
    queryFn: () => getCoverAlternatives(work.id),
    enabled: showAlternatives,
    staleTime: 60_000,
  });

  const selectMutation = useMutation({
    mutationFn: (candidate: CoverCandidate) =>
      selectCover(work.id, candidate.candidateId, candidate.mediaType),
    onSuccess: () => {
      toast.success("Cover updated");
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["coverAlternatives", work.id] });
      onCoverUploaded?.();
      onClose();
    },
    onError: () => toast.error("Failed to select cover"),
  });

  const uploadMutation = useMutation({
    mutationFn: ({ file, mediaType }: { file: Blob; mediaType: string }) =>
      uploadWorkCover(work.id, file, mediaType),
    onSuccess: () => {
      toast.success("Cover uploaded");
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["coverAlternatives", work.id] });
      onCoverUploaded?.();
      onClose();
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : "Failed to upload cover";
      toast.error(msg);
    },
  });

  const ebookCandidates = altQuery.data?.filter((c) => c.mediaType === "ebook") ?? [];
  const audioCandidates = altQuery.data?.filter((c) => c.mediaType === "audiobook") ?? [];

  return (
    <div>
      <span className="mb-1 block text-sm font-medium text-zinc-300">Cover</span>

      {/* Current covers */}
      <div className="flex gap-4 mb-2">
        <div className="flex items-start gap-2">
          <div className="h-20 w-14 flex-shrink-0">
            <BookCover
              workId={work.id}
              coverVersion={work.coverMtime ?? undefined}
              className="h-full w-full rounded"
            />
          </div>
          <div className="text-xs text-zinc-400 space-y-0.5">
            <p className="text-[10px] text-zinc-500 uppercase tracking-wider">Ebook</p>
            <p>Trust: <span className="text-zinc-200">{work.coverTrust}</span></p>
            {work.coverSource && (
              <p>Source: <span className="text-zinc-200">{work.coverSource}</span></p>
            )}
            {(work.coverWidth > 0 || work.coverHeight > 0) && (
              <p>Size: <span className="text-zinc-200">{work.coverWidth}&times;{work.coverHeight}</span></p>
            )}
            {work.coverTrust === "user" && (
              <p className="text-amber-500 text-[10px]">Locked</p>
            )}
          </div>
        </div>
        <div className="flex items-start gap-2">
          <div className="h-20 w-20 flex-shrink-0">
            <BookCover
              workId={work.id}
              coverVersion={work.audiobookCoverMtime ?? work.coverMtime ?? undefined}
              mediaType="audiobook"
              className="h-full w-full rounded"
            />
          </div>
          <div className="text-xs text-zinc-400 space-y-0.5">
            <p className="text-[10px] text-zinc-500 uppercase tracking-wider">Audiobook</p>
            <p>Trust: <span className="text-zinc-200">{work.audiobookCoverTrust}</span></p>
            {work.audiobookCoverSource && (
              <p>Source: <span className="text-zinc-200">{work.audiobookCoverSource}</span></p>
            )}
            {(work.audiobookCoverWidth > 0 || work.audiobookCoverHeight > 0) && (
              <p>Size: <span className="text-zinc-200">{work.audiobookCoverWidth}&times;{work.audiobookCoverHeight}</span></p>
            )}
            {work.audiobookCoverTrust === "user" && (
              <p className="text-amber-500 text-[10px]">Locked</p>
            )}
            {!work.audiobookCoverUrl && (
              <p className="text-zinc-600 text-[10px]">Falls back to ebook</p>
            )}
          </div>
        </div>
      </div>

      {/* Browse alternatives toggle */}
      <button
        type="button"
        onClick={() => setShowAlternatives(!showAlternatives)}
        className="flex items-center gap-1.5 text-xs text-brand hover:text-brand/80 mb-2"
      >
        <ImagePlus size={14} />
        {showAlternatives ? "Hide alternatives" : "Browse alternatives"}
        <ChevronDown
          size={12}
          className={cn("transition-transform", showAlternatives && "rotate-180")}
        />
      </button>

      {/* Alternatives grid */}
      {showAlternatives && (
        <div className="space-y-3 mb-3">
          {altQuery.isLoading && (
            <div className="flex items-center gap-2 text-xs text-zinc-400 py-4">
              <Loader2 size={14} className="animate-spin" />
              Loading alternatives...
            </div>
          )}

          {altQuery.isError && (
            <p className="text-xs text-red-400">Failed to load alternatives</p>
          )}

          {altQuery.data && (
            <>
              <CoverGrid
                label="Ebook"
                candidates={ebookCandidates}
                selecting={selectMutation.isPending}
                onSelect={(c) => selectMutation.mutate(c)}
                onUpload={(file) => uploadMutation.mutate({ file, mediaType: "ebook" })}
              />
              <CoverGrid
                label="Audiobook"
                candidates={audioCandidates}
                selecting={selectMutation.isPending}
                onSelect={(c) => selectMutation.mutate(c)}
                onUpload={(file) => uploadMutation.mutate({ file, mediaType: "audiobook" })}
              />
              {ebookCandidates.length === 0 && audioCandidates.length === 0 && (
                <p className="text-xs text-zinc-500 py-2">No alternative covers found</p>
              )}
              <button
                type="button"
                onClick={() => altQuery.refetch()}
                disabled={altQuery.isFetching}
                className="flex items-center gap-1 text-[11px] text-zinc-400 hover:text-zinc-200"
              >
                <RefreshCw size={10} className={altQuery.isFetching ? "animate-spin" : ""} />
                Refresh
              </button>
              <p className="text-[10px] text-zinc-600">
                Selecting a cover locks it from automatic updates.
              </p>
            </>
          )}
        </div>
      )}

      {/* Upload fallback when alternatives not shown */}
      {!showAlternatives && (
        <div className="flex items-center gap-2">
          <label className="flex items-center gap-1.5 cursor-pointer rounded bg-zinc-700 px-3 py-1.5 text-xs text-zinc-100 hover:bg-zinc-600">
            <Upload size={12} />
            Upload ebook cover
            <input
              type="file"
              accept="image/jpeg,image/png,image/webp"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) uploadMutation.mutate({ file, mediaType: "ebook" });
              }}
              className="hidden"
            />
          </label>
          <span className="text-[10px] text-zinc-500">JPEG, PNG, or WebP (max 5MB)</span>
        </div>
      )}
    </div>
  );
}

function CoverGrid({
  label,
  candidates,
  selecting,
  onSelect,
  onUpload,
}: {
  label: string;
  candidates: CoverCandidate[];
  selecting: boolean;
  onSelect: (c: CoverCandidate) => void;
  onUpload: (file: Blob) => void;
}) {
  return (
    <div>
      <span className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider">{label}</span>
      <div className="grid grid-cols-3 gap-2 mt-1">
        {candidates.map((c) => (
          <CoverCard
            key={c.candidateId}
            candidate={c}
            selecting={selecting}
            onSelect={() => onSelect(c)}
          />
        ))}
        <label className="flex flex-col items-center justify-center gap-1 rounded border border-dashed border-zinc-600 hover:border-brand bg-zinc-800/50 cursor-pointer aspect-[2/3] transition-colors">
          <Upload size={16} className="text-zinc-500" />
          <span className="text-[9px] text-zinc-500">Upload</span>
          <input
            type="file"
            accept="image/jpeg,image/png,image/webp"
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) onUpload(file);
            }}
            className="hidden"
          />
        </label>
      </div>
    </div>
  );
}

function CoverCard({
  candidate,
  selecting,
  onSelect,
}: {
  candidate: CoverCandidate;
  selecting: boolean;
  onSelect: () => void;
}) {
  const [failed, setFailed] = useState(false);
  const [dims, setDims] = useState<{ w: number; h: number } | null>(null);
  const good = dims ? dims.w >= 400 && dims.h >= 600 : null;

  if (failed) return null;

  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={selecting}
      className="group relative rounded border border-zinc-700 hover:border-brand overflow-hidden bg-zinc-800 transition-colors"
    >
      <div className="aspect-[2/3] relative">
        <img
          src={candidate.proxyUrl}
          alt={candidate.source}
          className="absolute inset-0 h-full w-full object-contain"
          onError={() => setFailed(true)}
          onLoad={(e) => {
            const img = e.target as HTMLImageElement;
            setDims({ w: img.naturalWidth, h: img.naturalHeight });
          }}
        />
      </div>
      <div className="px-1.5 py-1 text-center">
        <span className="text-[11px] text-zinc-400 block truncate">{candidate.source}</span>
        {dims && (
          <span className={cn("text-[10px]", good ? "text-green-500" : "text-amber-500")}>
            {dims.w}&times;{dims.h}
          </span>
        )}
      </div>
    </button>
  );
}

const PAGE_SIZE = 10;

function PaginatedReleaseTable({
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
                  {release.seeders ?? "\u2014"}
                </td>
                <td className="px-3 py-2 text-right text-muted">
                  {release.leechers ?? "\u2014"}
                </td>
                <td className="px-3 py-2 text-muted">
                  {release.publishDate
                    ? formatRelativeDate(release.publishDate)
                    : "\u2014"}
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
