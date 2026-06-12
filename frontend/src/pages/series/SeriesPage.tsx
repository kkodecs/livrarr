import { useEffect, useState } from "react";
import { Link } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Library, PlusCircle, ChevronRight, ChevronDown, Loader2 } from "lucide-react";
import { toast } from "sonner";
import {
  listAllSeries,
  listAuthors,
  getAuthorSeries,
  getSeriesBooks,
  monitorSeries,
  promoteSeries,
  resolveGr,
  updateAuthor,
} from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { FormModal } from "@/components/Page/FormModal";
import { HelpTip } from "@/components/HelpTip";
import { BookCover } from "@/components/BookCover";
import { MediaStatusRow } from "@/components/MediaStatusRow";
import { cn } from "@/utils/cn";
import type {
  AuthorResponse,
  PromoteSeriesResponse,
  SeriesResponse,
  SeriesWithAuthorResponse,
} from "@/types/api";

export default function SeriesPage() {
  const queryClient = useQueryClient();
  const [addOpen, setAddOpen] = useState(false);

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["series-all"],
    queryFn: listAllSeries,
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error as Error} onRetry={refetch} />;

  const series = data ?? [];
  const monitored = series.filter((s) => s.monitorEbook || s.monitorAudiobook);
  const unmonitored = series.filter(
    (s) => !s.monitorEbook && !s.monitorAudiobook,
  );

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-3">
          <h1 className="text-lg font-semibold text-zinc-100">Series</h1>
          <span className="text-xs text-zinc-500">
            {monitored.length} monitored
          </span>
        </div>
        <button
          onClick={() => setAddOpen(true)}
          className="inline-flex items-center gap-1.5 rounded bg-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-brand-hover"
        >
          <PlusCircle size={14} />
          <span className="hidden sm:inline">Add Series</span>
        </button>
      </PageToolbar>

      <PageContent>
        {series.length === 0 ? (
          <EmptyState
            icon={<Library size={32} />}
            title="No series"
            description="Add a series to start monitoring."
            action={
              <button
                onClick={() => setAddOpen(true)}
                className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover"
              >
                <PlusCircle size={14} />
                Add Series
              </button>
            }
          />
        ) : (
          <div className="space-y-2">
            {[...monitored, ...unmonitored].map((s) => (
              <SeriesRow
                key={s.id}
                series={s}
                onChanged={() =>
                  queryClient.invalidateQueries({ queryKey: ["series-all"] })
                }
              />
            ))}
          </div>
        )}
      </PageContent>

      <AddSeriesModal
        open={addOpen}
        onOpenChange={setAddOpen}
        existingGrKeys={new Set(series.map((s) => s.grKey))}
        onAdded={() => {
          queryClient.invalidateQueries({ queryKey: ["series-all"] });
        }}
      />
    </>
  );
}

type PromoteFlags = { monitorEbook: boolean; monitorAudiobook: boolean };

type PromoteFlow =
  | { step: "picker"; candidates: SeriesResponse[]; flags: PromoteFlags }
  | { step: "author"; authorId: number; flags: PromoteFlags }
  | null;

function SeriesRow({
  series: s,
  onChanged,
}: {
  series: SeriesWithAuthorResponse;
  onChanged: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const [flow, setFlow] = useState<PromoteFlow>(null);
  const isMonitored = s.monitorEbook || s.monitorAudiobook;
  // A stub has no Goodreads key; its count is the FK-linked library count.
  const isStub = !s.grKey;
  const displayCount = isStub ? s.worksInLibrary : s.bookCount;

  const promoteMutation = useMutation({
    mutationFn: (params: { grKey?: string; flags: PromoteFlags }) =>
      promoteSeries(s.id, {
        grKey: params.grKey ?? null,
        monitorEbook: params.flags.monitorEbook,
        monitorAudiobook: params.flags.monitorAudiobook,
      }),
    onSuccess: (resp: PromoteSeriesResponse, params) => {
      if (resp.status === "monitoring") {
        setFlow(null);
        toast.success("Series monitoring started");
        onChanged();
      } else if (resp.status === "needsPicker") {
        setFlow({
          step: "picker",
          candidates: resp.candidates ?? [],
          flags: params.flags,
        });
      } else if (resp.status === "needsAuthorResolution") {
        setFlow({ step: "author", authorId: resp.authorId, flags: params.flags });
      }
    },
    onError: () => {
      toast.error("Failed to start monitoring");
    },
  });

  return (
    <div className="rounded-lg border border-border bg-surface">
      <div className="flex items-center gap-2 sm:gap-3 p-2 sm:p-3">
        <button
          type="button"
          onClick={() => setExpanded(!expanded)}
          className="shrink-0 rounded p-1 text-zinc-500 hover:bg-surface-hover hover:text-zinc-200"
          title={expanded ? "Collapse" : "Show books"}
        >
          {expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
        </button>
        <Link
          to={`/series/${s.id}`}
          className="flex min-w-0 flex-1 items-center gap-3 sm:gap-4 hover:opacity-90"
        >
          {s.firstWorkId ? (
            <BookCover
              workId={s.firstWorkId}
              title={s.name}
              authorName={s.authorName}
              className="h-12 w-8 sm:h-16 sm:w-11"
              iconSize={14}
            />
          ) : (
            <div className="h-12 w-8 sm:h-16 sm:w-11 shrink-0 rounded bg-zinc-800 border border-zinc-700 flex items-center justify-center">
              <Library size={14} className="text-zinc-600" />
            </div>
          )}
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span
                className={cn(
                  "h-2 w-2 shrink-0 rounded-full",
                  isMonitored ? "bg-green-500" : "bg-zinc-600",
                )}
                title={isMonitored ? "Monitored" : "Not monitored"}
              />
              <p className="truncate font-medium text-sm sm:text-base text-zinc-100">
                {s.name}
              </p>
            </div>
            <div className="mt-0.5 ml-4 flex flex-wrap items-center gap-2 text-xs text-muted">
              <span>{s.authorName}</span>
              <span>
                {displayCount} {displayCount === 1 ? "book" : "books"}
              </span>
              {!isStub && s.worksInLibrary > 0 && (
                <span className="text-green-600">
                  {s.worksInLibrary} in library
                </span>
              )}
            </div>
          </div>
        </Link>
        <div className="shrink-0 flex flex-col items-end gap-1 text-xs">
          {s.monitorEbook && <span className="text-green-600">Ebook</span>}
          {s.monitorAudiobook && (
            <span className="text-green-600">Audiobook</span>
          )}
          {!isMonitored &&
            (promoteMutation.isPending ? (
              <Loader2 size={14} className="animate-spin text-brand" />
            ) : (
              <div className="flex items-center gap-1.5">
                <span className="hidden sm:inline text-zinc-500">Monitor:</span>
                {(
                  [
                    ["Ebook", { monitorEbook: true, monitorAudiobook: false }],
                    ["Audio", { monitorEbook: false, monitorAudiobook: true }],
                    ["Both", { monitorEbook: true, monitorAudiobook: true }],
                  ] as const
                ).map(([label, flags]) => (
                  <button
                    key={label}
                    type="button"
                    onClick={() => promoteMutation.mutate({ flags })}
                    className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
                  >
                    {label}
                  </button>
                ))}
              </div>
            ))}
        </div>
      </div>

      {expanded && <SeriesWorksExpansion seriesId={s.id} wasStub={isStub} />}

      {flow?.step === "picker" && (
        <SeriesPickerModal
          seriesName={s.name}
          candidates={flow.candidates}
          pending={promoteMutation.isPending}
          onPick={(grKey) => promoteMutation.mutate({ grKey, flags: flow.flags })}
          onCancel={() => setFlow(null)}
        />
      )}
      {flow?.step === "author" && (
        <AuthorResolveModal
          authorId={flow.authorId}
          authorName={s.authorName}
          onResolved={() => promoteMutation.mutate({ flags: flow.flags })}
          onCancel={() => setFlow(null)}
        />
      )}
    </div>
  );
}

/// AC-020 (REQ-010): expansion lists the series' FULL roster in position
/// order — in-library entries as links with the standard presence indication,
/// missing entries muted. Stubs resolve silently on first expand; only an
/// unresolvable stub falls back to linked works with the can't-match hint.
function SeriesWorksExpansion({
  seriesId,
  wasStub,
}: {
  seriesId: number;
  wasStub: boolean;
}) {
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ["series-books", seriesId],
    queryFn: () => getSeriesBooks(seriesId),
  });

  // A stub that came back with a roster was silently resolved — its list row
  // now has a real GR identity (count, link); refresh the list to show it.
  const resolvedNow = wasStub && data?.rosterAvailable === true;
  useEffect(() => {
    if (resolvedNow) {
      queryClient.invalidateQueries({ queryKey: ["series-all"] });
    }
  }, [resolvedNow, queryClient]);

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 border-t border-border px-4 py-2 text-xs text-zinc-500">
        <Loader2 size={12} className="animate-spin" /> Loading books...
      </div>
    );
  }
  if (error || !data) {
    return (
      <p className="border-t border-border px-4 py-2 text-xs text-red-400">
        Failed to load books.
      </p>
    );
  }

  if (data.rows.length === 0) {
    return (
      <p className="border-t border-border px-4 py-2 text-xs text-zinc-500">
        {data.rosterAvailable
          ? "No books found for this series."
          : "No books linked yet, and this series couldn't be matched on Goodreads automatically — monitor it to pick the right one."}
      </p>
    );
  }

  return (
    <div className="border-t border-border">
      {data.rows.map((row, i) => (
        <div
          key={row.work?.id ?? `missing-${i}`}
          className="flex items-center justify-between gap-2 border-t border-border/50 px-4 py-1.5 first:border-t-0"
        >
          <div className="flex min-w-0 items-center gap-2 text-sm">
            <span className="w-7 shrink-0 text-right text-xs text-zinc-500">
              {row.position ?? "–"}
            </span>
            {row.inLibrary && row.work ? (
              <Link
                to={`/work/${row.work.id}`}
                className="truncate text-zinc-200 hover:text-brand"
              >
                {row.title}
              </Link>
            ) : (
              <span className="truncate text-zinc-500">
                {row.title}
                {row.year != null && (
                  <span className="ml-1.5 text-xs text-zinc-600">
                    ({row.year})
                  </span>
                )}
              </span>
            )}
          </div>
          <div className="shrink-0">
            {row.inLibrary && row.work ? (
              <MediaStatusRow work={row.work} />
            ) : (
              <span className="text-xs text-zinc-600">Not in library</span>
            )}
          </div>
        </div>
      ))}
      {!data.rosterAvailable && (
        <p className="border-t border-border/50 px-4 py-1.5 text-xs text-zinc-600">
          Showing library books only — this series couldn't be matched on
          Goodreads automatically. Monitor it to pick the right series.
        </p>
      )}
    </div>
  );
}

/// REQ-009 picker: no/ambiguous exact match — the user chooses which of the
/// author's GR series this stub is, or cancels (stub left unchanged).
function SeriesPickerModal({
  seriesName,
  candidates,
  pending,
  onPick,
  onCancel,
}: {
  seriesName: string;
  candidates: SeriesResponse[];
  pending: boolean;
  onPick: (grKey: string) => void;
  onCancel: () => void;
}) {
  return (
    <FormModal open onOpenChange={(o) => !o && onCancel()} title="Match Series">
      <p className="mb-3 text-xs text-muted">
        Which Goodreads series is <span className="text-zinc-200">{seriesName}</span>?
      </p>
      {candidates.length === 0 && (
        <p className="py-2 text-sm text-zinc-500">
          No series found on the author's Goodreads page.
        </p>
      )}
      <div className="space-y-1">
        {candidates.map((c) => (
          <button
            key={c.grKey}
            type="button"
            disabled={pending}
            onClick={() => onPick(c.grKey)}
            className="flex w-full items-center justify-between rounded border border-border px-3 py-2 text-sm text-zinc-200 hover:border-brand hover:bg-surface-hover"
          >
            <span className="truncate">{c.name}</span>
            <span className="ml-2 shrink-0 text-xs text-zinc-500">
              {c.bookCount} {c.bookCount === 1 ? "book" : "books"}
            </span>
          </button>
        ))}
      </div>
    </FormModal>
  );
}

/// REQ-009 author leg: the stub's author has no Goodreads key — resolve it
/// via the existing author-candidate road, then retry the promotion.
function AuthorResolveModal({
  authorId,
  authorName,
  onResolved,
  onCancel,
}: {
  authorId: number;
  authorName: string;
  onResolved: () => void;
  onCancel: () => void;
}) {
  const [linking, setLinking] = useState(false);

  const { data, isLoading, error } = useQuery({
    queryKey: ["resolve-gr", authorId],
    queryFn: () => resolveGr(authorId),
    staleTime: 0,
  });

  // resolve-gr may auto-link when there's a single unambiguous match.
  const autoLinked = data?.autoLinked === true;
  useEffect(() => {
    if (autoLinked) onResolved();
  }, [autoLinked]);
  if (autoLinked) return null;

  const pick = async (grKey: string) => {
    setLinking(true);
    try {
      await updateAuthor(authorId, { grKey });
      onResolved();
    } catch {
      toast.error("Failed to link author");
      setLinking(false);
    }
  };

  return (
    <FormModal open onOpenChange={(o) => !o && onCancel()} title="Link Author to Goodreads">
      <p className="mb-3 text-xs text-muted">
        <span className="text-zinc-200">{authorName}</span> has no Goodreads
        link yet — pick the right author to continue.
      </p>
      {isLoading && (
        <div className="flex items-center gap-2 py-2 text-sm text-zinc-500">
          <Loader2 size={14} className="animate-spin" /> Searching Goodreads...
        </div>
      )}
      {error != null && (
        <p className="py-2 text-sm text-red-400">
          Author lookup failed — try again later.
        </p>
      )}
      {data && !data.autoLinked && data.candidates.length === 0 && (
        <p className="py-2 text-sm text-zinc-500">
          Author not found on Goodreads.
        </p>
      )}
      <div className="space-y-1">
        {(data?.candidates ?? []).map((c) => (
          <button
            key={c.grKey}
            type="button"
            disabled={linking}
            onClick={() => void pick(c.grKey)}
            className="flex w-full items-center justify-between rounded border border-border px-3 py-2 text-sm text-zinc-200 hover:border-brand hover:bg-surface-hover"
          >
            <span className="truncate">{c.name}</span>
            <span className="ml-2 shrink-0 text-xs text-zinc-500">
              {c.grKey}
            </span>
          </button>
        ))}
      </div>
    </FormModal>
  );
}

function AddSeriesModal({
  open,
  onOpenChange,
  existingGrKeys,
  onAdded,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  existingGrKeys: Set<string>;
  onAdded: () => void;
}) {
  const [expandedAuthor, setExpandedAuthor] = useState<number | null>(null);

  const { data: authors, isLoading: authorsLoading } = useQuery({
    queryKey: ["authors"],
    queryFn: listAuthors,
    enabled: open,
    staleTime: 0,
  });

  const eligibleAuthors = (authors ?? []).filter((a) => a.grKey);

  return (
    <FormModal open={open} onOpenChange={onOpenChange} title="Add Series">
      <div className="space-y-1">
        <div className="flex items-center justify-between mb-3">
          <p className="text-xs text-muted">
            Select an author to browse their series.
            <HelpTip text="Only authors already added to your library are shown." />
          </p>
          <Link
            to="/author/add"
            onClick={() => onOpenChange(false)}
            className="inline-flex items-center gap-1 text-xs text-brand hover:underline shrink-0"
          >
            <PlusCircle size={12} />
            Add Author
          </Link>
        </div>
        {authorsLoading && (
          <div className="flex items-center gap-2 text-sm text-zinc-500 py-4">
            <Loader2 size={14} className="animate-spin" /> Loading authors...
          </div>
        )}
        {!authorsLoading && eligibleAuthors.length === 0 && (
          <div className="py-4 text-center">
            <p className="text-sm text-zinc-500 mb-2">
              No authors with Goodreads linked.
            </p>
            <Link
              to="/author/add"
              onClick={() => onOpenChange(false)}
              className="inline-flex items-center gap-1.5 rounded bg-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-brand-hover"
            >
              <PlusCircle size={14} />
              Add Author
            </Link>
          </div>
        )}
        {eligibleAuthors.map((author) => (
          <AuthorSeriesExpander
            key={author.id}
            author={author}
            expanded={expandedAuthor === author.id}
            onToggle={() =>
              setExpandedAuthor(
                expandedAuthor === author.id ? null : author.id,
              )
            }
            existingGrKeys={existingGrKeys}
            onAdded={onAdded}
          />
        ))}
      </div>
    </FormModal>
  );
}

function AuthorSeriesExpander({
  author,
  expanded,
  onToggle,
  existingGrKeys,
  onAdded,
}: {
  author: AuthorResponse;
  expanded: boolean;
  onToggle: () => void;
  existingGrKeys: Set<string>;
  onAdded: () => void;
}) {
  const queryClient = useQueryClient();
  const [monitoringKey, setMonitoringKey] = useState<string | null>(null);

  const { data, isLoading } = useQuery({
    queryKey: ["series", author.id],
    queryFn: () => getAuthorSeries(author.id),
    enabled: expanded,
    staleTime: 0,
  });

  const monitorMutation = useMutation({
    mutationFn: (params: {
      grKey: string;
      monitorEbook: boolean;
      monitorAudiobook: boolean;
    }) => {
      setMonitoringKey(params.grKey);
      return monitorSeries(author.id, params);
    },
    onSuccess: () => {
      setMonitoringKey(null);
      queryClient.invalidateQueries({ queryKey: ["series", author.id] });
      onAdded();
      toast.success("Series monitoring started");
    },
    onError: () => {
      setMonitoringKey(null);
      toast.error("Failed to monitor series");
    },
  });

  const unmonitoredSeries = (data?.series ?? []).filter(
    (s) => !s.monitorEbook && !s.monitorAudiobook && !existingGrKeys.has(s.grKey),
  );

  return (
    <div className="rounded border border-border">
      <button
        onClick={onToggle}
        className="flex w-full items-center gap-2 px-3 py-2 text-sm text-zinc-200 hover:bg-surface-hover"
      >
        {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <span className="font-medium">{author.name}</span>
      </button>
      {expanded && (
        <div className="border-t border-border">
          {isLoading && (
            <div className="flex items-center gap-2 px-3 py-2 text-xs text-zinc-500">
              <Loader2 size={12} className="animate-spin" /> Loading series...
            </div>
          )}
          {!isLoading && unmonitoredSeries.length === 0 && (
            <p className="px-3 py-2 text-xs text-zinc-500">
              All series already monitored.
            </p>
          )}
          {unmonitoredSeries.map((s) => (
            <div
              key={s.grKey}
              className="flex items-center justify-between px-3 py-1.5 text-sm border-t border-border/50"
            >
              <div>
                <span className="text-zinc-200">{s.name}</span>
                <span className="ml-2 text-xs text-zinc-500">
                  {s.bookCount} {s.bookCount === 1 ? "book" : "books"}
                </span>
              </div>
              {monitoringKey === s.grKey ? (
                <Loader2 size={12} className="animate-spin text-brand" />
              ) : (
                <div className="flex items-center gap-1.5">
                  <span className="text-xs text-zinc-500">Monitor:</span>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      monitorMutation.mutate({
                        grKey: s.grKey,
                        monitorEbook: true,
                        monitorAudiobook: false,
                      });
                    }}
                    disabled={monitorMutation.isPending}
                    className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
                  >
                    Ebook
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      monitorMutation.mutate({
                        grKey: s.grKey,
                        monitorEbook: false,
                        monitorAudiobook: true,
                      });
                    }}
                    disabled={monitorMutation.isPending}
                    className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
                  >
                    Audio
                  </button>
                  <button
                    type="button"
                    onClick={(e) => {
                      e.preventDefault();
                      e.stopPropagation();
                      monitorMutation.mutate({
                        grKey: s.grKey,
                        monitorEbook: true,
                        monitorAudiobook: true,
                      });
                    }}
                    disabled={monitorMutation.isPending}
                    className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
                  >
                    Both
                  </button>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
