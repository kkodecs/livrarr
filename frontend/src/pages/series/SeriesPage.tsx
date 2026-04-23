import { useState } from "react";
import { Link } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Library, PlusCircle, ChevronRight, ChevronDown, Loader2 } from "lucide-react";
import { toast } from "sonner";
import {
  listAllSeries,
  listAuthors,
  getAuthorSeries,
  monitorSeries,
} from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { FormModal } from "@/components/Page/FormModal";
import { HelpTip } from "@/components/HelpTip";
import { BookCover } from "@/components/BookCover";
import { cn } from "@/utils/cn";
import type { AuthorResponse } from "@/types/api";

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
            {[...monitored, ...unmonitored].map((s) => {
              const isMonitored = s.monitorEbook || s.monitorAudiobook;
              return (
                <Link
                  key={s.id}
                  to={`/series/${s.id}`}
                  className="flex items-center gap-3 sm:gap-4 rounded-lg border border-border bg-surface p-2 sm:p-3 hover:border-brand"
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
                        {s.bookCount} {s.bookCount === 1 ? "book" : "books"}
                      </span>
                      {s.worksInLibrary > 0 && (
                        <span className="text-green-600">
                          {s.worksInLibrary} in library
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="hidden sm:flex shrink-0 flex-col items-end gap-1 text-xs">
                    {s.monitorEbook && (
                      <span className="text-green-600">Ebook</span>
                    )}
                    {s.monitorAudiobook && (
                      <span className="text-green-600">Audiobook</span>
                    )}
                    {!isMonitored && (
                      <span className="text-zinc-600">Not monitored</span>
                    )}
                  </div>
                </Link>
              );
            })}
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
