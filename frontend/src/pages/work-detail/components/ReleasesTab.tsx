import { useState, useRef, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { RefreshCw, Search, ChevronDown, ChevronRight, Book, Headphones } from "lucide-react";
import { searchReleases, grabRelease, getMediaManagementConfig } from "@/api";
import { EmptyState } from "@/components/Page/EmptyState";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { cn } from "@/utils/cn";
import { useSort } from "@/hooks/useSort";
import type { ReleaseResponse } from "@/types/api";
import { PaginatedReleaseTable, type ReleaseSortField } from "./PaginatedReleaseTable";

export function ReleasesTab({ workId }: { workId: number }) {
  const queryClient = useQueryClient();
  const [ebookFormatFilter, setEbookFormatFilter] = useState<Set<string> | null>(null);
  const [audiobookFormatFilter, setAudiobookFormatFilter] = useState<Set<string> | null>(null);

  // Mode ref controls what the queryFn does:
  // 'cacheCheck' = ask backend for cached results only (no indexer hits) — used on mount
  // 'search'     = full search hitting all indexers
  // 'refresh'    = full search bypassing backend cache
  const modeRef = useRef<"cacheCheck" | "search" | "refresh">("cacheCheck");
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
  const orderedEbookFormats = (() => {
    const present = detectFormatsInReleases([...ebookReleases, ...uncategorized], allEbookFormats);
    return [...ebookPrefs.filter((f) => present.includes(f)), ...present.filter((f) => !ebookPrefs.includes(f))];
  })();
  const orderedAudiobookFormats = (() => {
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
  const doSearch = () => runQuery("refresh");

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
