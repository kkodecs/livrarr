import { useState, useEffect } from "react";
import { useSearchParams, useNavigate, Link } from "react-router";
import { useQuery, useMutation } from "@tanstack/react-query";
import { Search, Plus, Loader2 } from "lucide-react";
import { toast } from "sonner";
import { lookupWorks, addWork, listWorks } from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { EmptyState } from "@/components/Page/EmptyState";
import { cn } from "@/utils/cn";
import { getCoverUrl } from "@/utils/format";
import type {
  WorkSearchResult,
  AddWorkResponse,
  WorkDetailResponse,
} from "@/types/api";
import { ApiError } from "@/api/client";

export default function SearchPage() {
  const navigate = useNavigate();
  const [searchParams, setSearchParams] = useSearchParams();
  const initialQuery = searchParams.get("q") ?? "";
  const [term, setTerm] = useState(initialQuery);
  const [olResults, setOlResults] = useState<WorkSearchResult[] | null>(null);
  const [selectedOlKey, setSelectedOlKey] = useState<string | null>(null);
  const [monitored, setMonitored] = useState(false);
  const [lastSearched, setLastSearched] = useState("");

  // Local library data
  const { data: allWorks } = useQuery({
    queryKey: ["works"],
    queryFn: listWorks,
  });

  // Filter local works by search term
  const query = searchParams.get("q")?.trim().toLowerCase() ?? "";
  const libraryMatches = query
    ? (allWorks ?? []).filter(
        (w) =>
          w.title.toLowerCase().includes(query) ||
          w.authorName.toLowerCase().includes(query),
      )
    : [];

  // OL search
  const searchMutation = useMutation({
    mutationFn: (q: string) => lookupWorks(q),
    onSuccess: (data) => {
      setOlResults(data);
      setSelectedOlKey(null);
    },
    onError: () => toast.error("Search failed"),
  });

  const addMutation = useMutation({
    mutationFn: (work: WorkSearchResult) =>
      addWork({
        olKey: work.olKey,
        title: work.title,
        authorName: work.authorName,
        authorOlKey: work.authorOlKey,
        year: work.year,
        coverUrl: work.coverUrl,
      }),
    onSuccess: (data: AddWorkResponse) => {
      data.messages.forEach((msg) => toast.success(msg));
      navigate(`/work/${data.work.id}`);
    },
    onError: (err: Error) => {
      if (err instanceof ApiError && err.status === 409) {
        toast.error("Already in your library");
      } else {
        toast.error(err.message || "Failed to add work");
      }
    },
  });

  // Auto-search when query param changes (from header search bar or direct nav)
  useEffect(() => {
    const q = searchParams.get("q")?.trim() ?? "";
    if (q && q !== lastSearched && !searchMutation.isPending) {
      setTerm(q);
      setLastSearched(q);
      searchMutation.mutate(q);
    }
  }, [searchParams]); // intentional: only re-run when URL params change

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    const q = term.trim();
    if (!q) return;
    setLastSearched(q);
    setSearchParams({ q });
    searchMutation.mutate(q);
  };

  // Filter add-results to exclude works already in the library.
  // Match on olKey OR title+author (case-insensitive) since Hardcover results
  // use different keys than OL-sourced library items.
  const libraryOlKeys = new Set((allWorks ?? []).map((w) => w.olKey).filter(Boolean));
  const libraryTitleAuthor = new Set(
    (allWorks ?? []).map((w) => `${w.title.toLowerCase()}|${w.authorName.toLowerCase()}`),
  );
  const filteredOlResults =
    olResults?.filter(
      (r) =>
        !libraryOlKeys.has(r.olKey) &&
        !libraryTitleAuthor.has(`${r.title.toLowerCase()}|${r.authorName.toLowerCase()}`),
    ) ?? null;

  const hasQuery = !!query;
  const hasLibraryResults = libraryMatches.length > 0;
  const hasOlResults = filteredOlResults !== null && filteredOlResults.length > 0;
  const showNoResults =
    hasQuery &&
    !searchMutation.isPending &&
    !hasLibraryResults &&
    filteredOlResults !== null &&
    filteredOlResults.length === 0;

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Search</h1>
      </PageToolbar>

      <PageContent>
        <form onSubmit={handleSearch} className="flex gap-2">
          <div className="relative flex-1">
            <Search
              size={16}
              className="absolute left-3 top-1/2 -translate-y-1/2 text-muted"
            />
            <input
              type="text"
              value={term}
              onChange={(e) => setTerm(e.target.value)}
              placeholder="Search by title, author, or ISBN..."
              className="w-full rounded border border-border bg-zinc-800 py-2 pl-9 pr-3 text-sm text-zinc-100 placeholder:text-muted focus:border-brand focus:outline-none"
              autoFocus
            />
          </div>
          <button
            type="submit"
            disabled={searchMutation.isPending || !term.trim()}
            className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {searchMutation.isPending ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Search size={14} />
            )}
            Search
          </button>
        </form>

        <div className="mt-6 space-y-8">
          {/* ── In Your Library ── */}
          {hasLibraryResults && (
            <section>
              <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted">
                In Your Library
              </h2>
              <div className="rounded border border-border">
                {libraryMatches.map((work) => (
                  <LibraryResult key={work.id} work={work} />
                ))}
              </div>
            </section>
          )}

          {/* ── Loading ── */}
          {searchMutation.isPending && (
            <div className="flex items-center justify-center py-12">
              <Loader2 size={24} className="animate-spin text-muted" />
            </div>
          )}

          {/* ── Add to Your Library ── */}
          {!searchMutation.isPending && hasOlResults && (
            <section>
              <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted">
                Add to Your Library
              </h2>
              <div className="rounded border border-border">
                {filteredOlResults!.map((work) => (
                  <OlResult
                    key={work.olKey}
                    work={work}
                    isSelected={selectedOlKey === work.olKey}
                    onSelect={() =>
                      setSelectedOlKey(
                        selectedOlKey === work.olKey ? null : work.olKey,
                      )
                    }
                    onAdd={() => addMutation.mutate(work)}
                    isAdding={addMutation.isPending}
                    monitored={monitored}
                    onMonitoredChange={setMonitored}
                  />
                ))}
              </div>
            </section>
          )}

          {/* ── No Results ── */}
          {showNoResults && (
            <EmptyState
              icon={<Search size={32} />}
              title="No results"
              description="Try a different search term."
            />
          )}
        </div>
      </PageContent>
    </>
  );
}

function LibraryResult({ work }: { work: WorkDetailResponse }) {
  return (
    <Link
      to={`/work/${work.id}`}
      className="flex items-center gap-2 border-b border-border/50 px-2 py-1.5 hover:bg-zinc-800/50"
    >
      <img
        src={getCoverUrl(work.id)}
        alt=""
        className="h-8 w-6 shrink-0 rounded bg-zinc-700 object-cover"
      />
      <span className="min-w-0 truncate font-medium text-sm text-zinc-100">{work.title}</span>
      {work.seriesName && (
        <span className="shrink-0 text-xs text-zinc-500">
          {work.seriesName}
          {work.seriesPosition != null && ` #${work.seriesPosition}`}
        </span>
      )}
      <span className="flex-1" />
      <span className="shrink-0 text-xs text-muted">{work.authorName}</span>
      <span className="shrink-0 w-10 text-right text-xs text-zinc-500">
        {work.year ?? ""}
      </span>
    </Link>
  );
}

function OlResult({
  work,
  isSelected,
  onSelect,
  onAdd,
  isAdding,
  monitored,
  onMonitoredChange,
}: {
  work: WorkSearchResult;
  isSelected: boolean;
  onSelect: () => void;
  onAdd: () => void;
  isAdding: boolean;
  monitored: boolean;
  onMonitoredChange: (v: boolean) => void;
}) {
  return (
    <div
      className={cn(
        "border-b border-border/50 transition-colors",
        isSelected && "bg-brand/5",
      )}
    >
      <button
        type="button"
        onClick={onSelect}
        className="flex w-full items-center gap-2 px-2 py-1.5 text-left hover:bg-zinc-800/50"
      >
        {work.coverUrl ? (
          <img
            src={work.coverUrl}
            alt=""
            className="h-8 w-6 shrink-0 rounded bg-zinc-700 object-cover"
          />
        ) : (
          <div className="flex h-8 w-6 shrink-0 items-center justify-center rounded bg-zinc-700 text-[8px] text-zinc-500">
            ?
          </div>
        )}
        <span className="min-w-0 truncate font-medium text-sm text-zinc-100">{work.title}</span>
        {work.seriesName && (
          <span className="shrink-0 text-xs text-zinc-500">
            {work.seriesName}
            {work.seriesPosition != null && ` #${work.seriesPosition}`}
          </span>
        )}
        <span className="flex-1" />
        <span className="shrink-0 text-xs text-muted">{work.authorName}</span>
        <span className="shrink-0 w-10 text-right text-xs text-zinc-500">
          {work.year ?? ""}
        </span>
      </button>

      {isSelected && (
        <div className="flex items-center gap-4 px-2 pb-1.5 pt-0.5">
          <label className="flex items-center gap-2 text-xs text-zinc-300">
            <input
              type="checkbox"
              checked={monitored}
              onChange={(e) => onMonitoredChange(e.target.checked)}
              className="h-3.5 w-3.5 rounded border-zinc-600 bg-zinc-900 text-brand"
            />
            Monitored
          </label>
          <button
            onClick={onAdd}
            disabled={isAdding}
            className="inline-flex items-center gap-1 rounded bg-brand px-3 py-1 text-xs font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {isAdding ? (
              <Loader2 size={12} className="animate-spin" />
            ) : (
              <Plus size={12} />
            )}
            Add
          </button>
        </div>
      )}
    </div>
  );
}
