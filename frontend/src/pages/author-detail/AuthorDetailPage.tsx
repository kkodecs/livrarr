import { useState, useEffect } from "react";
import { Link, useParams, useNavigate } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  ExternalLink,
  RefreshCw,
  Trash2,
  BookOpen,
  Loader2,
  Library,
} from "lucide-react";
import { toast } from "sonner";
import {
  getAuthor,
  updateAuthor,
  deleteAuthor,
  searchAuthors,
  getAuthorBibliography,
  refreshAuthorBibliography,
  addWork,
  getAuthorSeries,
  refreshAuthorSeries,
  monitorSeries,
  updateSeries,
  resolveGr,
} from "@/api";
import type { SeriesResponse } from "@/types/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import { MediaStatusRow } from "@/components/MediaStatusRow";
import { formatRelativeDate } from "@/utils/format";
import { BookCover } from "@/components/BookCover";
import { cn } from "@/utils/cn";
import { HelpTip } from "@/components/HelpTip";
import type { AuthorDetailResponse } from "@/types/api";

export default function AuthorDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const authorId = Number(id);

  const [deleteOpen, setDeleteOpen] = useState(false);

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["author", id],
    queryFn: () => getAuthor(authorId),
    enabled: !isNaN(authorId),
  });

  const refreshMutation = useMutation({
    mutationFn: () => searchAuthors(),
    onSuccess: () => {
      toast.success("Author refresh started");
    },
    onError: () => {
      toast.error("Failed to refresh author");
    },
  });

  const updateMutation = useMutation({
    mutationFn: (req: { monitored?: boolean; monitorNewItems?: boolean }) =>
      updateAuthor(authorId, req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["author", id] });
      queryClient.invalidateQueries({ queryKey: ["authors"] });
    },
    onError: () => toast.error("Failed to update author"),
  });

  const deleteMutation = useMutation({
    mutationFn: () => deleteAuthor(authorId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["authors"] });
      toast.success("Author deleted");
      navigate("/author");
    },
    onError: () => {
      toast.error("Failed to delete author");
    },
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error as Error} onRetry={refetch} />;
  if (!data) return <ErrorState error={new Error("Author not found")} />;

  const { author, works } = data;

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-3">
          <div className="flex items-baseline gap-2">
            <h1 className="text-lg font-semibold text-zinc-100">{author.name}</h1>
            <span className="text-xs text-zinc-600">#{author.id}</span>
          </div>
          {author.olKey && (
            <a
              href={`https://openlibrary.org/authors/${author.olKey}`}
              target="_blank"
              rel="noopener noreferrer"
              className="text-muted hover:text-zinc-200"
              title="View on Open Library"
            >
              <ExternalLink size={14} />
            </a>
          )}
        </div>

        <div className="flex items-center gap-2">
          <button
            onClick={() => refreshMutation.mutate()}
            disabled={refreshMutation.isPending}
            className="inline-flex items-center gap-1.5 rounded border border-border px-3 py-1.5 text-sm text-zinc-200 hover:bg-surface-hover"
          >
            <RefreshCw
              size={14}
              className={refreshMutation.isPending ? "animate-spin" : ""}
            />
            <span className="hidden sm:inline">Refresh</span>
          </button>
          <button
            onClick={() => setDeleteOpen(true)}
            className="inline-flex items-center gap-1.5 rounded border border-red-800 px-3 py-1.5 text-sm text-red-400 hover:bg-red-900/30"
          >
            <Trash2 size={14} />
            <span className="hidden sm:inline">Delete</span>
          </button>
        </div>
      </PageToolbar>

      <PageContent>
        {/* Author header info */}
        <div className="mb-6 flex flex-wrap items-center gap-4 text-sm text-muted">
          <span>
            {works.length} {works.length === 1 ? "work" : "works"}
          </span>
          <button
            onClick={() => updateMutation.mutate({ monitored: !author.monitored })}
            disabled={updateMutation.isPending}
            className={cn(
              "inline-flex items-center gap-1.5 rounded border px-2.5 py-1 text-xs transition-colors",
              author.monitored
                ? "border-green-700 bg-green-900/20 text-green-400 hover:bg-green-900/40"
                : "border-border text-zinc-500 hover:bg-surface-hover hover:text-zinc-300",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                author.monitored ? "bg-green-500" : "bg-zinc-600",
              )}
            />
            Monitored
            <HelpTip text="Monitor indexers for new uploads of all content by author." />
          </button>
          <button
            onClick={() => updateMutation.mutate({ monitorNewItems: !author.monitorNewItems })}
            disabled={updateMutation.isPending}
            className={cn(
              "inline-flex items-center gap-1.5 rounded border px-2.5 py-1 text-xs transition-colors",
              author.monitorNewItems
                ? "border-green-700 bg-green-900/20 text-green-400 hover:bg-green-900/40"
                : "border-border text-zinc-500 hover:bg-surface-hover hover:text-zinc-300",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                author.monitorNewItems ? "bg-green-500" : "bg-zinc-600",
              )}
            />
            Monitor New
            <HelpTip text="Auto-add new works by this author when detected." />
          </button>
        </div>

        {/* Works list */}
        {works.length === 0 ? (
          <EmptyState
            icon={<BookOpen size={32} />}
            title="No works"
            description="No works found for this author."
          />
        ) : (
          <div className="space-y-2">
            {works.map((work) => (
              <Link
                key={work.id}
                to={`/work/${work.id}`}
                className="flex items-center gap-3 sm:gap-4 rounded-lg border border-border bg-surface p-2 sm:p-3 hover:border-brand"
              >
                <BookCover
                  workId={work.id}
                  title={work.title}
                  authorName={work.authorName}
                  className="h-12 w-8 sm:h-16 sm:w-11"
                  iconSize={14}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium text-sm sm:text-base text-zinc-100">
                    {work.title}
                  </p>
                  <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                    {work.year && <span>{work.year}</span>}
                    <MediaStatusRow work={work} />
                  </div>
                </div>
              </Link>
            ))}
          </div>
        )}
        {/* Series */}
        <SeriesSection authorId={authorId} author={author} />
        {/* Bibliography */}
        <BibliographySection authorId={authorId} author={author} libraryOlKeys={new Set(works.map((w) => w.olKey).filter(Boolean) as string[])} />
      </PageContent>

      {/* Delete Confirm */}
      <ConfirmModal
        open={deleteOpen}
        onOpenChange={setDeleteOpen}
        title="Delete Author"
        description="This will remove the author from your library. Works will be preserved."
        confirmLabel="Delete"
        variant="danger"
        onConfirm={() => deleteMutation.mutateAsync()}
      />
    </>
  );
}

function BibliographySection({
  authorId,
  author,
  libraryOlKeys,
}: {
  authorId: number;
  author: AuthorDetailResponse["author"];
  libraryOlKeys: Set<string>;
}) {
  const queryClient = useQueryClient();
  const [addedKeys, setAddedKeys] = useState<Set<string>>(new Set());
  const [addingKey, setAddingKey] = useState<string | null>(null);
  const [showRaw, setShowRaw] = useState(false);

  const { data: bib, isLoading } = useQuery({
    queryKey: ["bibliography", authorId, showRaw],
    queryFn: () => getAuthorBibliography(authorId, showRaw),
    retry: 2,
    retryDelay: 3000,
  });

  const refreshMutation = useMutation({
    mutationFn: () => refreshAuthorBibliography(authorId),
    onSuccess: () => {
      queryClient.invalidateQueries({
        queryKey: ["bibliography", authorId],
      });
      toast.success("Bibliography refreshed");
    },
    onError: () => toast.error("Failed to refresh bibliography"),
  });

  const addMutation = useMutation({
    mutationFn: (entry: { olKey: string; title: string; year: number | null }) => {
      setAddingKey(entry.olKey);
      return addWork({
        olKey: entry.olKey,
        title: entry.title,
        authorName: author.name,
        authorOlKey: author.olKey ?? null,
        year: entry.year,
        coverUrl: null,
      });
    },
    onSuccess: (data, entry) => {
      setAddedKeys((prev) => new Set(prev).add(entry.olKey));
      setAddingKey(null);
      queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
      toast.success(`Added "${data.work.title}"`);
    },
    onError: (err: Error) => {
      setAddingKey(null);
      toast.error(err.message || "Failed to add work");
    },
  });

  const hasBib = bib && bib.entries.length > 0;
  const isFetching = isLoading || refreshMutation.isPending;

  return (
    <section className="mt-8">
      <div className="flex items-center gap-3 mb-3">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted">
          Bibliography
        </h2>
        {bib?.rawAvailable && (
          <div className="flex items-center rounded border border-border text-xs">
            <button
              onClick={() => setShowRaw(false)}
              className={cn(
                "px-2 py-0.5 rounded-l",
                !showRaw ? "bg-brand text-white" : "text-muted hover:text-zinc-100",
              )}
            >
              LLM Filtered {bib.filteredCount}
            </button>
            <button
              onClick={() => setShowRaw(true)}
              className={cn(
                "px-2 py-0.5 rounded-r",
                showRaw ? "bg-brand text-white" : "text-muted hover:text-zinc-100",
              )}
            >
              Raw {bib.rawCount}
            </button>
          </div>
        )}
        {hasBib && (
          <span className="text-xs text-zinc-500">
            fetched {formatRelativeDate(bib.fetchedAt)}
          </span>
        )}
        {isFetching ? (
          <span className="flex items-center gap-1.5 text-xs text-zinc-500">
            <RefreshCw size={10} className="animate-spin" /> Refreshing...
          </span>
        ) : (
          <button
            onClick={() => refreshMutation.mutate()}
            className="text-xs text-zinc-500 hover:text-zinc-300"
          >
            Refresh
          </button>
        )}
      </div>
      {!hasBib && !isFetching && (
        <p className="text-sm text-zinc-500">No bibliography available.</p>
      )}
      {hasBib && <div className="overflow-x-auto rounded border border-border">
        <table className="w-full text-sm">
          <tbody>
            {bib!.entries.map((entry) => {
              const inLibrary = libraryOlKeys.has(entry.olKey) || addedKeys.has(entry.olKey);
              return (
                <tr
                  key={entry.olKey}
                  className={cn(
                    "border-b border-border/50",
                    inLibrary ? "text-zinc-500" : "text-zinc-200",
                  )}
                >
                  <td className="px-2 py-1.5">
                    <span className="font-medium">{entry.title}</span>
                    {inLibrary && <span className="ml-2 text-xs text-green-600">In Library</span>}
                    {entry.seriesName && (
                      <span className="ml-2 text-xs text-zinc-500">
                        {entry.seriesName}
                        {entry.seriesPosition != null && ` #${entry.seriesPosition}`}
                      </span>
                    )}
                  </td>
                  <td className="hidden sm:table-cell px-2 py-1.5 w-12 text-right text-xs text-zinc-500">
                    {entry.year ?? ""}
                  </td>
                  <td className="px-2 py-1.5 w-10 text-right">
                    {!inLibrary && (
                      addingKey === entry.olKey ? (
                        <Loader2 size={12} className="inline animate-spin text-brand" />
                      ) : (
                        <button
                          onClick={() => addMutation.mutate(entry)}
                          disabled={addMutation.isPending}
                          className="text-xs text-brand hover:underline"
                        >
                          Add
                        </button>
                      )
                    )}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>}
    </section>
  );
}

function SeriesSection({
  authorId,
  author,
}: {
  authorId: number;
  author: AuthorDetailResponse["author"];
}) {
  const queryClient = useQueryClient();
  const [monitoringKey, setMonitoringKey] = useState<string | null>(null);
  const [resolveOpen, setResolveOpen] = useState(false);
  const [showRaw, setShowRaw] = useState(false);

  // Only show if author has grKey.
  const hasGrKey = !!author.grKey;

  const { data, isLoading } = useQuery({
    queryKey: ["series", authorId, showRaw],
    queryFn: () => getAuthorSeries(authorId, showRaw),
    enabled: hasGrKey,
    retry: 2,
    retryDelay: 3000,
  });

  const refreshMutation = useMutation({
    mutationFn: () => refreshAuthorSeries(authorId),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["series", authorId] });
      toast.success("Series list refreshed");
    },
    onError: () => toast.error("Failed to refresh series"),
  });

  const monitorMutation = useMutation({
    mutationFn: (params: { grKey: string; monitorEbook: boolean; monitorAudiobook: boolean }) => {
      setMonitoringKey(params.grKey);
      return monitorSeries(authorId, params);
    },
    onSuccess: () => {
      setMonitoringKey(null);
      queryClient.invalidateQueries({ queryKey: ["series", authorId] });
      queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
      toast.success("Series monitoring started");
    },
    onError: () => {
      setMonitoringKey(null);
      toast.error("Failed to monitor series");
    },
  });

  const unmonitorMutation = useMutation({
    mutationFn: (seriesId: number) =>
      updateSeries(seriesId, { monitorEbook: false, monitorAudiobook: false }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["series", authorId] });
      queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
      toast.success("Series unmonitored");
    },
    onError: () => toast.error("Failed to unmonitor series"),
  });

  if (!hasGrKey) {
    return (
      <section className="mt-8">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted mb-3">
          Series
        </h2>
        <div className="flex items-center gap-3 text-sm text-zinc-500">
          <Library size={16} />
          <span>Link Goodreads author to enable series monitoring.</span>
          <button
            onClick={() => setResolveOpen(true)}
            className="text-xs text-brand hover:underline"
          >
            Link
          </button>
        </div>
        {resolveOpen && (
          <ResolveGrModal
            authorId={authorId}
            authorName={author.name}
            open={resolveOpen}
            onOpenChange={setResolveOpen}
            onLinked={() => {
              queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
              queryClient.invalidateQueries({ queryKey: ["series", authorId] });
              setResolveOpen(false);
            }}
          />
        )}
      </section>
    );
  }

  const hasSeries = data && data.series.length > 0;
  const isFetching = isLoading || refreshMutation.isPending;

  return (
    <section className="mt-8">
      <div className="flex items-center gap-3 mb-3">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted">
          Series
        </h2>
        {data?.rawAvailable && (
          <div className="flex items-center rounded border border-border text-xs">
            <button
              onClick={() => setShowRaw(false)}
              className={cn(
                "px-2 py-0.5 rounded-l",
                !showRaw ? "bg-brand text-white" : "text-muted hover:text-zinc-100",
              )}
            >
              LLM Filtered {data.filteredCount}
            </button>
            <button
              onClick={() => setShowRaw(true)}
              className={cn(
                "px-2 py-0.5 rounded-r",
                showRaw ? "bg-brand text-white" : "text-muted hover:text-zinc-100",
              )}
            >
              Raw {data.rawCount}
            </button>
          </div>
        )}
        {data?.fetchedAt && (
          <span className="text-xs text-zinc-500">
            fetched {new Date(data.fetchedAt).toLocaleDateString()}
          </span>
        )}
        {isFetching ? (
          <span className="flex items-center gap-1.5 text-xs text-zinc-500">
            <RefreshCw size={10} className="animate-spin" /> Loading...
          </span>
        ) : (
          <button
            onClick={() => refreshMutation.mutate()}
            className="text-xs text-zinc-500 hover:text-zinc-300"
          >
            Refresh
          </button>
        )}
      </div>
      {!hasSeries && !isFetching && (
        <p className="text-sm text-zinc-500">No series found on Goodreads.</p>
      )}
      {hasSeries && (
        <div className="overflow-x-auto rounded border border-border">
          <table className="w-full text-sm">
            <tbody>
              {data!.series.map((s) => (
                <SeriesRow
                  key={s.grKey}
                  series={s}
                  isMonitoring={monitoringKey === s.grKey}
                  onMonitor={(monitorEbook, monitorAudiobook) =>
                    monitorMutation.mutate({
                      grKey: s.grKey,
                      monitorEbook,
                      monitorAudiobook,
                    })
                  }
                  onUnmonitor={() => s.id && unmonitorMutation.mutate(s.id)}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function SeriesRow({
  series,
  isMonitoring,
  onMonitor,
  onUnmonitor,
}: {
  series: SeriesResponse;
  isMonitoring: boolean;
  onMonitor: (ebook: boolean, audiobook: boolean) => void;
  onUnmonitor: () => void;
}) {
  const isMonitored = series.monitorEbook || series.monitorAudiobook;

  return (
    <tr className="border-b border-border/50">
      <td className="px-2 py-2">
        <div className="flex items-center gap-2">
          <span
            className={cn(
              "h-2 w-2 shrink-0 rounded-full",
              isMonitored ? "bg-green-500" : "bg-zinc-600",
            )}
          />
          <span className="font-medium text-zinc-200">{series.name}</span>
        </div>
        {isMonitored && (
          <div className="ml-4 mt-0.5 flex gap-2 text-xs text-zinc-500">
            {series.monitorEbook && <span className="text-green-600">Ebook</span>}
            {series.monitorAudiobook && <span className="text-green-600">Audiobook</span>}
          </div>
        )}
      </td>
      <td className="hidden sm:table-cell px-2 py-2 text-xs text-zinc-500 text-right whitespace-nowrap">
        {series.bookCount} {series.bookCount === 1 ? "book" : "books"}
      </td>
      <td className="hidden sm:table-cell px-2 py-2 text-xs text-zinc-500 text-right whitespace-nowrap">
        {series.worksInLibrary > 0 && (
          <span className="text-green-600">{series.worksInLibrary} in library</span>
        )}
      </td>
      <td className="px-2 py-2 text-right whitespace-nowrap">
        {isMonitoring ? (
          <Loader2 size={14} className="inline animate-spin text-brand" />
        ) : isMonitored ? (
          <button
            type="button"
            onClick={onUnmonitor}
            className="text-xs text-red-400 hover:underline"
          >
            Unmonitor
          </button>
        ) : (
          <div className="flex items-center gap-1.5">
            <span className="text-xs text-zinc-500">Monitor:</span>
            <button
              type="button"
              onClick={() => onMonitor(true, false)}
              className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
            >
              Ebook
            </button>
            <button
              type="button"
              onClick={() => onMonitor(false, true)}
              className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
            >
              Audio
            </button>
            <button
              type="button"
              onClick={() => onMonitor(true, true)}
              className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
            >
              Both
            </button>
          </div>
        )}
      </td>
    </tr>
  );
}

function ResolveGrModal({
  authorId,
  authorName,
  open,
  onOpenChange,
  onLinked,
}: {
  authorId: number;
  authorName: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onLinked: () => void;
}) {
  const [candidates, setCandidates] = useState<
    { grKey: string; name: string; profileUrl: string }[]
  >([]);
  const [loading, setLoading] = useState(false);
  const [linking, setLinking] = useState<string | null>(null);

  const handleOpen = async () => {
    setLoading(true);
    try {
      const resp = await resolveGr(authorId);
      if (resp.autoLinked) {
        toast.success("Goodreads author auto-linked");
        onLinked();
        return;
      }
      setCandidates(resp.candidates);
    } catch {
      toast.error("Failed to search Goodreads");
    } finally {
      setLoading(false);
    }
  };

  const handleLink = async (grKey: string) => {
    setLinking(grKey);
    try {
      await updateAuthor(authorId, { grKey });
      toast.success("Goodreads author linked");
      onLinked();
    } catch {
      toast.error("Failed to link author");
    } finally {
      setLinking(null);
    }
  };

  // Fetch on first open.
  useEffect(() => {
    if (open) handleOpen();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <FormModal open={open} onOpenChange={onOpenChange} title="Link Goodreads Author">
      <div className="space-y-3">
        <p className="text-sm text-muted">
          Select the correct Goodreads author for "{authorName}":
        </p>
        {loading && (
          <div className="flex items-center gap-2 text-sm text-zinc-500">
            <Loader2 size={14} className="animate-spin" /> Searching Goodreads...
          </div>
        )}
        {!loading && candidates.length === 0 && (
          <p className="text-sm text-zinc-500">No matches found.</p>
        )}
        {candidates.map((c) => (
          <div
            key={c.grKey}
            className="flex items-center justify-between rounded border border-border p-2"
          >
            <div>
              <span className="text-sm text-zinc-200">{c.name}</span>
              <a
                href={c.profileUrl}
                target="_blank"
                rel="noopener noreferrer"
                className="ml-2 text-xs text-zinc-500 hover:text-zinc-300"
              >
                <ExternalLink size={10} className="inline" />
              </a>
            </div>
            {linking === c.grKey ? (
              <Loader2 size={14} className="animate-spin text-brand" />
            ) : (
              <button
                onClick={() => handleLink(c.grKey)}
                disabled={!!linking}
                className="text-xs text-brand hover:underline"
              >
                Link
              </button>
            )}
          </div>
        ))}
        <div className="flex justify-end pt-2">
          <button
            onClick={() => onOpenChange(false)}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
        </div>
      </div>
    </FormModal>
  );
}
