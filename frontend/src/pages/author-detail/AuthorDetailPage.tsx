import { useState, useEffect, useRef } from "react";
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
  updateSeries,
  reResolveAuthor,
} from "@/api";
import type { SeriesResponse } from "@/types/api";
import { SUPPORTED_LANGUAGES } from "@/types/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { MediaStatusRow } from "@/components/MediaStatusRow";
import { formatRelativeDate } from "@/utils/format";
import { BookCover } from "@/components/BookCover";
import { cn } from "@/utils/cn";
import { HelpTip } from "@/components/HelpTip";
import { useSeriesPromote } from "@/hooks/useSeriesPromote";
import { SeriesPickerModal, AuthorResolveModal as SeriesAuthorResolveModal } from "@/components/SeriesPromoteModals";
import { AuthorLinkPanel } from "./AuthorLinkPanel";
import { authorGateMessage } from "@/utils/authorLink";
import type { AuthorDetailResponse } from "@/types/api";

// #112: a plain, human-readable label for a language code — "Auto: ES"
// meant nothing to a user seeing it for the first time; a flag + real
// language name needs no explanation.
function languageLabel(code: string): string {
  const known = SUPPORTED_LANGUAGES.find((l) => l.code === code);
  return known ? `${known.flag} ${known.englishName}` : code.toUpperCase();
}

// Stable identity for a bibliography row: title alone collides when an author
// has two distinct entries with the same title (different OL keys/years).
function bibliographyEntryKey(e: {
  olKey: string | null;
  title: string;
  year: number | null;
}): string {
  return e.olKey ?? `${e.title}|${e.year ?? ""}`;
}

export default function AuthorDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const authorId = Number(id);

  const [deleteOpen, setDeleteOpen] = useState(false);
  // #112: one shared toggle for both Series and Bibliography (was two
  // independent per-section toggles that could disagree on what's hidden).
  const [showAllLanguages, setShowAllLanguages] = useState(false);

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
    mutationFn: (req: {
      monitored?: boolean;
      monitorNewItems?: boolean;
      monitorLanguage?: string;
    }) => updateAuthor(authorId, req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["author", id] });
      queryClient.invalidateQueries({ queryKey: ["authors"] });
    },
    // The monitor gate is the server's; show the reason it gave.
    onError: (err: unknown) =>
      toast.error(authorGateMessage(err, "Failed to update author")),
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

  // Pre-fill for the monitor language: the dominant language among this
  // author's library works; a tie or an all-language-less set suggests
  // nothing (the selector falls back to English).
  const suggestedLanguage = (() => {
    const counts = new Map<string, number>();
    for (const w of works) {
      if (w.language) counts.set(w.language, (counts.get(w.language) ?? 0) + 1);
    }
    let best: string | null = null;
    let bestN = 0;
    let tied = false;
    for (const [lang, n] of counts) {
      if (n > bestN) {
        best = lang;
        bestN = n;
        tied = false;
      } else if (n === bestN) {
        tied = true;
      }
    }
    return tied ? null : best;
  })();
  // Once monitoring is on, show only the persisted truth; the suggestion is a
  // pre-fill for the not-yet-monitored state and is persisted the moment
  // monitoring is enabled.
  const displayedLanguage =
    author.monitorLanguage ?? (author.monitored ? "en" : (suggestedLanguage ?? "en"));
  // #112: the single language a series/bibliography entry is compared
  // against to decide "does this match the author" — the persisted monitor
  // setting, same fallback used everywhere else on this page.
  const authorLanguage = author.monitorLanguage ?? "en";

  // #129: after "Add All" lands the bibliography, keep the author current —
  // same enable path as the Monitored button.
  const enableMonitoring = () => {
    if (author.monitored && author.monitorNewItems) return;
    updateMutation.mutate({
      monitored: true,
      monitorNewItems: true,
      monitorLanguage: displayedLanguage,
    });
    toast.success(`Monitoring enabled for ${author.name}`);
  };

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
            onClick={() =>
              updateMutation.mutate(
                author.monitored
                  ? { monitored: false }
                  : { monitored: true, monitorLanguage: displayedLanguage },
              )
            }
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
          <span className="inline-flex items-center gap-1.5">
            <span className="text-xs text-zinc-500">Monitor language:</span>
            <select
              value={displayedLanguage}
              onChange={(e) => updateMutation.mutate({ monitorLanguage: e.target.value })}
              disabled={updateMutation.isPending}
              className="h-7 rounded border border-border bg-zinc-800 px-2 text-xs text-zinc-100"
            >
              {SUPPORTED_LANGUAGES.map((lang) => (
                <option key={lang.code} value={lang.code}>
                  {lang.flag} {lang.englishName}
                </option>
              ))}
            </select>
            <HelpTip text="Language stamped on new works this author's monitor auto-adds (and the fallback for series with no detected language of their own). Pre-filled from this author's library." />
          </span>
        </div>

        {/* #112: one shared language-visibility filter for both the Series
            and Bibliography sections below — a single control, not one per
            section, so they can't disagree on what's hidden. */}
        <div className="mb-4 flex items-center gap-2 text-sm">
          <span className="text-muted">Discovery view:</span>
          <button
            onClick={() => setShowAllLanguages((v) => !v)}
            className="rounded border border-border px-2.5 py-1 text-xs text-zinc-300 hover:bg-surface-hover"
          >
            {showAllLanguages ? "All languages" : `${authorLanguage.toUpperCase()} + unknown only`}
          </button>
          <HelpTip text="Filters foreign-language editions and series out of the lists below by default. Unknown-language entries always show — they aren't confirmed foreign." />
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
        {/* Who this author is: provider links, names, merge */}
        <AuthorLinkPanel author={author} />
        {/* Series */}
        <SeriesSection
          authorId={authorId}
          author={author}
          authorLanguage={authorLanguage}
          showAllLanguages={showAllLanguages}
        />
        {/* Bibliography */}
        <BibliographySection
          authorId={authorId}
          author={author}
          authorLanguage={authorLanguage}
          showAllLanguages={showAllLanguages}
          libraryOlKeys={new Set(works.map((w) => w.olKey).filter(Boolean) as string[])}
          enableMonitoring={enableMonitoring}
        />
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
  authorLanguage,
  showAllLanguages,
  libraryOlKeys,
  enableMonitoring,
}: {
  authorId: number;
  author: AuthorDetailResponse["author"];
  authorLanguage: string;
  showAllLanguages: boolean;
  libraryOlKeys: Set<string>;
  enableMonitoring: () => void;
}) {
  const queryClient = useQueryClient();
  const [addedKeys, setAddedKeys] = useState<Set<string>>(new Set());
  const [addingKey, setAddingKey] = useState<string | null>(null);
  const [showRaw, setShowRaw] = useState(false);
  const [addAllOpen, setAddAllOpen] = useState(false);
  const [addAllProgress, setAddAllProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);

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
    mutationFn: (entry: {
      olKey: string | null;
      title: string;
      year: number | null;
      language?: string | null;
    }) => {
      setAddingKey(bibliographyEntryKey(entry));
      return addWork({
        olKey: entry.olKey || null,
        title: entry.title,
        authorName: author.name,
        authorOlKey: author.olKey ?? null,
        year: entry.year,
        coverUrl: null,
        // #112: an unknown-language entry should still get the author's own
        // language, not silently fall through to the install-wide default
        // (which ignored the author page you're looking at entirely).
        language: entry.language ?? authorLanguage,
      });
    },
    onSuccess: (data, entry) => {
      setAddedKeys((prev) => new Set(prev).add(bibliographyEntryKey(entry)));
      setAddingKey(null);
      queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
      queryClient.invalidateQueries({ queryKey: ["bibliography", authorId] });
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
  const visibleEntries = (bib?.entries ?? []).filter(
    (e) => showAllLanguages || e.language == null || e.language === authorLanguage,
  );
  const hiddenCount = (bib?.entries.length ?? 0) - visibleEntries.length;

  // #129: everything on screen that isn't in the library yet — exactly the
  // rows whose per-entry "Add" button shows. "Add All" presses them all.
  const missingEntries = visibleEntries.filter(
    (e) =>
      !(
        e.alreadyInLibrary ||
        (e.olKey != null && libraryOlKeys.has(e.olKey)) ||
        addedKeys.has(bibliographyEntryKey(e))
      ),
  );

  // Leaving the page stops the add loop (and blocks a second overlapping run
  // on return); the works added so far stay, the rest need another click.
  const unmountedRef = useRef(false);
  useEffect(() => {
    unmountedRef.current = false;
    return () => {
      unmountedRef.current = true;
    };
  }, []);

  const runAddAll = async () => {
    const seen = new Set<string>();
    const targets = missingEntries.filter((e) => {
      const k = bibliographyEntryKey(e);
      if (seen.has(k)) return false;
      seen.add(k);
      return true;
    });
    let added = 0;
    let failed = 0;
    setAddAllProgress({ done: 0, total: targets.length });
    for (let i = 0; i < targets.length; i++) {
      if (unmountedRef.current) break;
      const entry = targets[i];
      if (!entry) continue;
      try {
        await addWork({
          olKey: entry.olKey || null,
          title: entry.title,
          authorName: author.name,
          authorOlKey: author.olKey ?? null,
          year: entry.year,
          coverUrl: null,
          language: entry.language ?? authorLanguage,
        });
        added++;
        setAddedKeys((prev) => new Set(prev).add(bibliographyEntryKey(entry)));
      } catch {
        failed++;
      }
      setAddAllProgress({ done: i + 1, total: targets.length });
    }
    // Refresh caches even after an aborted run, so a return visit shows the
    // true partial state instead of stale "not in library" rows.
    queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
    queryClient.invalidateQueries({ queryKey: ["bibliography", authorId] });
    queryClient.invalidateQueries({ queryKey: ["works"] });
    if (unmountedRef.current) return;
    setAddAllProgress(null);
    if (failed === 0) {
      toast.success(`Added ${added} work${added !== 1 ? "s" : ""} to your library`);
    } else {
      toast.warning(`Added ${added}, failed ${failed} of ${targets.length} works`);
    }
    if (added > 0) enableMonitoring();
  };

  return (
    <section className="mt-8">
      <div className="flex items-center gap-3 mb-3">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted">
          Bibliography
        </h2>
        {hasBib && !showAllLanguages && hiddenCount > 0 && (
          <span className="text-xs text-zinc-600">({hiddenCount} hidden by language filter)</span>
        )}
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
            disabled={addAllProgress != null}
            className="text-xs text-zinc-500 hover:text-zinc-300 disabled:opacity-50"
          >
            Refresh
          </button>
        )}
        {addAllProgress ? (
          <span className="flex items-center gap-1.5 text-xs text-brand">
            <Loader2 size={10} className="animate-spin" /> Adding{" "}
            {addAllProgress.done}/{addAllProgress.total}...
          </span>
        ) : (
          hasBib &&
          missingEntries.length > 0 && (
            <button
              onClick={() => setAddAllOpen(true)}
              disabled={addMutation.isPending}
              className="text-xs text-brand hover:underline disabled:opacity-50"
            >
              Add All ({missingEntries.length})
            </button>
          )
        )}
      </div>
      {!hasBib && !isFetching && (
        <p className="text-sm text-zinc-500">No bibliography available.</p>
      )}
      {hasBib && visibleEntries.length === 0 && (
        <p className="text-sm text-zinc-500">
          All entries are in other languages — use the "Discovery view" toggle above to see them.
        </p>
      )}
      {hasBib && visibleEntries.length > 0 && <div className="overflow-x-auto rounded border border-border">
        <table className="w-full text-sm">
          <tbody>
            {visibleEntries.map((entry, entryIdx) => {
              const inLibrary = entry.alreadyInLibrary || (entry.olKey != null && libraryOlKeys.has(entry.olKey)) || addedKeys.has(bibliographyEntryKey(entry));
              const isForeign = entry.language != null && entry.language !== authorLanguage;
              const isUnknownLanguage = entry.language == null;
              return (
                <tr
                  key={`${bibliographyEntryKey(entry)}-${entryIdx}`}
                  className={cn(
                    "border-b border-border/50",
                    inLibrary ? "text-zinc-500" : "text-zinc-200",
                  )}
                >
                  <td className="px-2 py-1.5">
                    <span className="font-medium">{entry.title}</span>
                    {inLibrary && <span className="ml-2 text-xs text-green-600">In Library</span>}
                    {isForeign && (
                      <span className="ml-2 rounded bg-zinc-800 px-1.5 py-0.5 text-xs text-zinc-400">
                        {languageLabel(entry.language!)}
                      </span>
                    )}
                    {isUnknownLanguage && (
                      <span
                        className="ml-2 text-xs italic text-zinc-600"
                        title="Couldn't confirm what language this is — shown by default rather than guessed."
                      >
                        language unknown
                      </span>
                    )}
                    {entry.seriesName && (
                      <span className="ml-2 text-xs text-zinc-500">
                        {entry.seriesName}
                        {entry.seriesPosition != null && ` #${entry.seriesPosition}`}
                      </span>
                    )}
                  </td>
                  <td className="px-2 py-1.5 w-10 text-right">
                    {!inLibrary && (
                      addingKey === bibliographyEntryKey(entry) ? (
                        <Loader2 size={12} className="inline animate-spin text-brand" />
                      ) : (
                        <button
                          onClick={() => addMutation.mutate(entry)}
                          disabled={addMutation.isPending || addAllProgress != null}
                          className="text-xs text-brand hover:underline disabled:opacity-50"
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
      <ConfirmModal
        open={addAllOpen}
        onOpenChange={setAddAllOpen}
        title={`Add ${missingEntries.length} works?`}
        description={`Add all ${missingEntries.length} listed works by ${author.name} to your library, then turn on monitoring so future releases are picked up automatically. Works are added one at a time — a large bibliography can take a few minutes.${
          !showAllLanguages && hiddenCount > 0
            ? ` The ${hiddenCount} entr${hiddenCount === 1 ? "y" : "ies"} hidden by the language filter will not be added.`
            : ""
        }`}
        confirmLabel="Add All"
        variant="default"
        onConfirm={() => {
          void runAddAll();
        }}
      />
    </section>
  );
}

function SeriesSection({
  authorId,
  author,
  authorLanguage,
  showAllLanguages,
}: {
  authorId: number;
  author: AuthorDetailResponse["author"];
  authorLanguage: string;
  showAllLanguages: boolean;
}) {
  const queryClient = useQueryClient();
  const [showRaw, setShowRaw] = useState(false);

  // Series listings come from the author's Goodreads route.
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

  const handleMonitored = () => {
    queryClient.invalidateQueries({ queryKey: ["series", authorId] });
    queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
  };

  if (!hasGrKey) {
    return (
      <section className="mt-8">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted mb-3">
          Series
        </h2>
        <div className="flex flex-wrap items-center gap-3 text-sm text-zinc-500">
          <Library size={16} />
          <span>
            Series monitoring needs a Goodreads link for this author.
          </span>
          <GoodreadsLinkHint authorId={authorId} />
        </div>
      </section>
    );
  }

  const hasSeries = data && data.series.length > 0;
  const isFetching = isLoading || refreshMutation.isPending;
  const visibleSeries = (data?.series ?? []).filter(
    (s) => showAllLanguages || s.language == null || s.language === authorLanguage,
  );
  const hiddenSeriesCount = (data?.series.length ?? 0) - visibleSeries.length;

  return (
    <section className="mt-8">
      <div className="flex items-center gap-3 mb-3">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted">
          Series
        </h2>
        {hasSeries && !showAllLanguages && hiddenSeriesCount > 0 && (
          <span className="text-xs text-zinc-600">({hiddenSeriesCount} hidden by language filter)</span>
        )}
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
      {hasSeries && visibleSeries.length === 0 && (
        <p className="text-sm text-zinc-500">
          All series are in other languages — use the "Discovery view" toggle above to see them.
        </p>
      )}
      {hasSeries && visibleSeries.length > 0 && (
        <div className="overflow-x-auto rounded border border-border">
          <table className="w-full text-sm">
            <tbody>
              {visibleSeries.map((s) => (
                <SeriesRow
                  key={s.id ?? s.grKey}
                  authorId={authorId}
                  authorName={author.name}
                  series={s}
                  authorLanguage={authorLanguage}
                  onMonitored={handleMonitored}
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
  authorId,
  authorName,
  series,
  authorLanguage,
  onMonitored,
  onUnmonitor,
}: {
  authorId: number;
  authorName: string;
  series: SeriesResponse;
  authorLanguage: string;
  onMonitored: () => void;
  onUnmonitor: () => void;
}) {
  const isMonitored = series.monitorEbook || series.monitorAudiobook;
  const isForeign = series.language != null && series.language !== authorLanguage;
  const isUnknownLanguage = series.language == null;
  // A stub has no Goodreads key yet; its count is the FK-linked library count.
  const isStub = !series.grKey;
  const displayCount = isStub ? series.worksInLibrary : series.bookCount;
  // This listing already carries the real Goodreads key for any row that has
  // one (masked to "" only for a genuine unresolved stub, same signal as
  // isStub above) — always forward it so a series with no DB row yet doesn't
  // fall back to sending an empty grKey to the monitor door.
  const knownGrKey = series.grKey || undefined;

  const { promote, isPending, flow, cancelFlow } = useSeriesPromote({
    authorId,
    seriesId: series.id,
    language: authorLanguage,
    onMonitoring: onMonitored,
  });

  return (
    <>
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
            {isForeign && (
              <span
                className="rounded bg-zinc-800 px-1.5 py-0.5 text-xs text-zinc-400"
                title="Detected automatically. If you monitor this series, books it creates will use this language, not the Monitor language setting above."
              >
                {languageLabel(series.language!)}
              </span>
            )}
            {isUnknownLanguage && (
              <span
                className="text-xs italic text-zinc-600"
                title="Couldn't confirm what language this is — shown by default rather than guessed."
              >
                language unknown
              </span>
            )}
          </div>
          {isMonitored && (
            <div className="ml-4 mt-0.5 flex gap-2 text-xs text-zinc-500">
              {series.monitorEbook && <span className="text-green-600">Ebook</span>}
              {series.monitorAudiobook && <span className="text-green-600">Audiobook</span>}
            </div>
          )}
        </td>
        <td className="hidden sm:table-cell px-2 py-2 text-xs text-zinc-500 text-right whitespace-nowrap">
          {displayCount} {displayCount === 1 ? "book" : "books"}
        </td>
        <td className="hidden sm:table-cell px-2 py-2 text-xs text-zinc-500 text-right whitespace-nowrap">
          {!isStub && series.worksInLibrary > 0 && (
            <span className="text-green-600">{series.worksInLibrary} in library</span>
          )}
        </td>
        <td className="px-2 py-2 text-right whitespace-nowrap">
          {isPending ? (
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
                onClick={() => promote({ grKey: knownGrKey, flags: { monitorEbook: true, monitorAudiobook: false } })}
                className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
              >
                Ebook
              </button>
              <button
                type="button"
                onClick={() => promote({ grKey: knownGrKey, flags: { monitorEbook: false, monitorAudiobook: true } })}
                className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
              >
                Audio
              </button>
              <button
                type="button"
                onClick={() => promote({ grKey: knownGrKey, flags: { monitorEbook: true, monitorAudiobook: true } })}
                className="rounded border border-border px-2 py-0.5 text-xs text-zinc-300 hover:bg-surface-hover hover:text-brand"
              >
                Both
              </button>
            </div>
          )}
        </td>
      </tr>
      {flow?.step === "picker" && (
        <SeriesPickerModal
          seriesName={series.name}
          candidates={flow.candidates}
          pending={isPending}
          onPick={(grKey) => promote({ grKey, flags: flow.flags })}
          onCancel={cancelFlow}
        />
      )}
      {flow?.step === "author" && (
        <SeriesAuthorResolveModal
          authorId={flow.authorId}
          authorName={authorName}
          onCancel={cancelFlow}
        />
      )}
    </>
  );
}

/**
 * How an author gets a Goodreads link now.
 *
 * Hand-picking a Goodreads author from a name search is gone: a name match
 * alone was never proof, and the link it wrote had no evidence behind it.
 * Links are earned from matched books, or picked from the review page where
 * the evidence is shown alongside them. All this door does is ask for another
 * look.
 */
function GoodreadsLinkHint({ authorId }: { authorId: number }) {
  const queryClient = useQueryClient();

  const lookAgain = useMutation({
    mutationFn: () => reResolveAuthor(authorId),
    onSuccess: () => {
      toast.success("Queued — we'll look this author up in the background");
      queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
      queryClient.invalidateQueries({ queryKey: ["author-link-sweep"] });
    },
    onError: () => toast.error("Could not queue this author"),
  });

  return (
    <>
      <button
        type="button"
        onClick={() => lookAgain.mutate()}
        disabled={lookAgain.isPending}
        className="text-xs text-brand hover:underline disabled:opacity-50"
      >
        {lookAgain.isPending ? "Queueing…" : "Look again"}
      </button>
      <Link to="/review" className="text-xs text-zinc-400 hover:underline">
        Review suggestions
      </Link>
      <HelpTip text="Goodreads links come from books of theirs we have already matched, or from a suggestion you approve on the review page. There is no name-only linking any more." />
    </>
  );
}
