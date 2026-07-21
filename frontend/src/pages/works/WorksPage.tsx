import { useCallback, useMemo, useState } from "react";
import { Link, useNavigate, useSearchParams } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Book,
  BookOpen,
  ChevronDown,
  ChevronRight,
  Headphones,
  Layers,
  Plus,
  RefreshCw,
  RotateCcw,
  Rss,
  TableProperties,
  LayoutGrid,
  LayoutList,
  Search,
  Pencil,
  Trash2,
  CheckSquare,
  ZoomIn,
  ZoomOut,
  Star,
} from "lucide-react";
import { listWorks, refreshAllWorks, retryAllIncomplete, deleteWork, refreshWork, triggerRssSync, getQueue, updateWork } from "@/api";
import type { UpdateWorkRequest } from "@/types/api";
import { computeTotalPages } from "@/utils/pagination";
import type { WorkSortField } from "@/utils/works";
import { useUIStore } from "@/stores/ui";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { Pagination } from "@/components/Page/Pagination";
import { cn } from "@/utils/cn";
import { SortHeader } from "@/components/Page/SortHeader";
import { formatRelativeDate, formatDuration } from "@/utils/format";
import { MediaStatusRow } from "@/components/MediaStatusRow";
import { BookCover } from "@/components/BookCover";
import ProgressBar from "@/components/ProgressBar";
import ProgressBadge from "@/components/ProgressBadge";
import type {
  WorkDetailResponse,
  LibraryItemResponse,
  MediaType,
} from "@/types/api";
import { SUPPORTED_LANGUAGES } from "@/types/api";

function bestProgress(items: LibraryItemResponse[]): LibraryItemResponse | null {
  let best: LibraryItemResponse | null = null;
  for (const li of items) {
    if (li.progressPct != null && li.progressPct > 0) {
      if (!best || (li.progressPct > (best.progressPct ?? 0))) {
        best = li;
      }
    }
  }
  return best;
}

const PAGE_SIZE = 50;

// --- Series grouping (collapse-series view) ---
//
// One entity per collapsed series, positioned where its best-sorted member
// falls in the server sort order; standalone works (and series with a single
// work in the library) stay as plain work entities.

type SeriesGroup = {
  kind: "series";
  key: string;
  seriesId: number | null;
  seriesName: string;
  works: WorkDetailResponse[];
};

type WorkEntity = { kind: "work"; work: WorkDetailResponse } | SeriesGroup;

function seriesKeyOf(w: WorkDetailResponse): string | null {
  if (w.seriesId != null) return `id:${w.seriesId}`;
  // Name-only fallback is scoped per author: two authors' identically-named
  // series (e.g. "Collected Works") must not merge into one group.
  if (w.seriesName)
    return `name:${w.authorId ?? w.authorName.toLowerCase()}:${w.seriesName.toLowerCase()}`;
  return null;
}

function groupBySeries(works: WorkDetailResponse[]): WorkEntity[] {
  const order: WorkEntity[] = [];
  const groups = new Map<string, SeriesGroup>();
  for (const w of works) {
    const key = seriesKeyOf(w);
    if (!key) {
      order.push({ kind: "work", work: w });
      continue;
    }
    let g = groups.get(key);
    if (!g) {
      g = {
        kind: "series",
        key,
        seriesId: w.seriesId ?? null,
        seriesName: w.seriesName ?? "",
        works: [],
      };
      groups.set(key, g);
      order.push(g);
    }
    g.works.push(w);
  }
  return order.map((e) => {
    if (e.kind === "series" && e.works.length === 1) {
      const only = e.works[0];
      if (only) return { kind: "work" as const, work: only };
    }
    if (e.kind === "series") {
      // Series order inside a group; position-less members keep server order.
      e.works = [...e.works].sort(
        (a, b) =>
          (a.seriesPosition ?? Number.POSITIVE_INFINITY) -
          (b.seriesPosition ?? Number.POSITIVE_INFINITY),
      );
    }
    return e;
  });
}

function groupLibraryCount(g: SeriesGroup): number {
  return g.works.filter((w) => w.libraryItems.length > 0).length;
}

const SORT_FIELD_MAP: Record<WorkSortField, string> = {
  title: "title",
  authorName: "author",
  year: "year",
  addedAt: "date_added",
  recentlyDownloaded: "recently_downloaded",
};

export function WorksPage() {
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();

  const page = Math.max(1, Number(searchParams.get("page")) || 1);
  const setPage = (p: number) => {
    setSearchParams((prev) => {
      const next = new URLSearchParams(prev);
      if (p <= 1) next.delete("page");
      else next.set("page", String(p));
      return next;
    }, { replace: false });
  };

  const worksView = useUIStore((s) => s.worksView);
  const setWorksView = useUIStore((s) => s.setWorksView);
  const worksSort = useUIStore((s) => s.worksSort) as WorkSortField;
  const worksSortDir = useUIStore((s) => s.worksSortDir);
  const setWorksSort = useUIStore((s) => s.setWorksSort);
  const posterZoom = useUIStore((s) => s.posterZoom);
  const setPosterZoom = useUIStore((s) => s.setPosterZoom);
  const mediaTypeFilter = useUIStore((s) => s.worksMediaFilter) as MediaType | "";
  const setMediaTypeFilter = useUIStore((s) => s.setWorksMediaFilter);
  const languageFilter = useUIStore((s) => s.worksLanguageFilter);
  const setLanguageFilter = useUIStore((s) => s.setWorksLanguageFilter);
  const collapseSeries = useUIStore((s) => s.worksCollapseSeries);
  const setCollapseSeries = useUIStore((s) => s.setWorksCollapseSeries);

  const {
    data: worksData,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["works", page, worksSort, worksSortDir, mediaTypeFilter, languageFilter],
    queryFn: () =>
      listWorks({
        page,
        pageSize: PAGE_SIZE,
        sortBy: SORT_FIELD_MAP[worksSort] ?? "date_added",
        sortDir: worksSortDir,
        mediaType: mediaTypeFilter || undefined,
        language: languageFilter || undefined,
      }),
    enabled: !collapseSeries,
    refetchInterval: 60_000,
    placeholderData: (prev) => prev,
  });

  // Collapsed mode needs the whole (filtered, sorted) library to group series
  // across page boundaries — walk the server pages once, client-paginate the
  // grouped result. Same pattern the Servarr apps use (full library client-side).
  const {
    data: allWorks,
    isLoading: allLoading,
    error: allError,
    refetch: refetchAll,
  } = useQuery({
    // "works"-prefixed so every existing invalidateQueries({queryKey: ["works"]})
    // site refreshes this view too.
    queryKey: ["works", "all", worksSort, worksSortDir, mediaTypeFilter, languageFilter],
    queryFn: async ({ signal }) => {
      const pageSize = 1000;
      const params = {
        pageSize,
        sortBy: SORT_FIELD_MAP[worksSort] ?? "date_added",
        sortDir: worksSortDir,
        mediaType: mediaTypeFilter || undefined,
        language: languageFilter || undefined,
      };
      const first = await listWorks({ ...params, page: 1 });
      const items = [...first.items];
      const pages = computeTotalPages(first.total, pageSize);
      for (let p = 2; p <= pages; p++) {
        // A superseded walk (filter/sort changed) must not keep hammering the
        // server behind the fresh one.
        if (signal.aborted) throw new DOMException("aborted", "AbortError");
        const next = await listWorks({ ...params, page: p });
        items.push(...next.items);
      }
      return items;
    },
    enabled: collapseSeries,
    // No interval refetch: this walks the whole library — mutations invalidate
    // the "works" prefix, which covers this key.
    staleTime: 60_000,
    placeholderData: (prev) => prev,
  });

  const works = collapseSeries ? allWorks : worksData?.items;

  const refreshMutation = useMutation({
    mutationFn: () =>
      refreshAllWorks({
        language: languageFilter || undefined,
        mediaType: mediaTypeFilter || undefined,
      }),
    onSuccess: () => toast.success("Refreshing all works"),
    onError: (e: Error) => toast.error(e.message),
  });

  const retryMutation = useMutation({
    mutationFn: retryAllIncomplete,
    onSuccess: () => toast.success("Retrying incomplete works"),
    onError: () => toast.error("Failed to start retry"),
  });

  const rssSyncMutation = useMutation({
    mutationFn: triggerRssSync,
    onSuccess: () => toast.success("RSS sync started"),
    onError: () => toast.error("RSS sync already running"),
  });

  const toggleMonitorMutation = useMutation({
    mutationFn: ({ workId, field }: { workId: number; field: "monitorEbook" | "monitorAudiobook" }) => {
      const work = works?.find((w) => w.id === workId);
      if (!work) return Promise.reject(new Error("Work not found"));
      return updateWork(workId, { [field]: !work[field] } as UpdateWorkRequest);
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Failed to update monitoring"),
  });

  const handleToggleMonitor = useCallback(
    (workId: number, field: "monitorEbook" | "monitorAudiobook") => {
      toggleMonitorMutation.mutate({ workId, field });
    },
    [toggleMonitorMutation],
  );

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

  const [searchQuery, setSearchQuery] = useState("");

  const [editorMode, setEditorMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [showDeleteModal, setShowDeleteModal] = useState(false);

  const toggleEditorMode = useCallback(() => {
    setEditorMode((prev) => {
      if (prev) {
        setSelectedIds(new Set());
      }
      return !prev;
    });
  }, []);

  const toggleSelection = useCallback((id: number) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) {
        next.delete(id);
      } else {
        next.add(id);
      }
      return next;
    });
  }, []);

  const filtered = useMemo(() => {
    if (!works) return [];
    if (!searchQuery) return works;
    const q = searchQuery.toLowerCase();
    return works.filter(
      (w) =>
        w.title.toLowerCase().includes(q) ||
        w.authorName.toLowerCase().includes(q) ||
        (collapseSeries && (w.seriesName?.toLowerCase().includes(q) ?? false)),
    );
  }, [works, searchQuery, collapseSeries]);

  const entities = useMemo<WorkEntity[]>(() => {
    if (!collapseSeries) return filtered.map((work) => ({ kind: "work" as const, work }));
    return groupBySeries(filtered);
  }, [filtered, collapseSeries]);

  // Collapsed mode paginates entities client-side (a collapsed series counts
  // as one item); flat mode keeps server-side pagination.
  const total = collapseSeries ? entities.length : (worksData?.total ?? 0);
  const totalPages = computeTotalPages(total, PAGE_SIZE);
  const pageEntities = useMemo(
    () =>
      collapseSeries
        ? entities.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE)
        : entities,
    [entities, collapseSeries, page],
  );

  // The works actually on screen — selection operates on these in both modes.
  const visibleWorks = useMemo(
    () => pageEntities.flatMap((e) => (e.kind === "work" ? [e.work] : e.works)),
    [pageEntities],
  );

  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const toggleGroup = useCallback((key: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const handleToggleCollapseSeries = useCallback(() => {
    setCollapseSeries(!collapseSeries);
    setPage(1);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collapseSeries, setCollapseSeries]);

  const allSelected =
    visibleWorks.length > 0 && visibleWorks.every((w) => selectedIds.has(w.id));

  const toggleSelectAll = useCallback(() => {
    if (allSelected) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(visibleWorks.map((w) => w.id)));
    }
  }, [allSelected, visibleWorks]);

  // Group checkbox in editor mode: select/deselect every member at once.
  const toggleSelectMany = useCallback((ids: number[]) => {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      const allIn = ids.every((id) => next.has(id));
      for (const id of ids) {
        if (allIn) next.delete(id);
        else next.add(id);
      }
      return next;
    });
  }, []);

  const handleBulkDelete = async () => {
    const ids = Array.from(selectedIds);
    const results = await Promise.allSettled(
      ids.map((id) => deleteWork(id)),
    );
    const succeeded = results.filter((r) => r.status === "fulfilled").length;
    const failed = results.filter((r) => r.status === "rejected").length;
    if (failed === 0) {
      toast.success(`Deleted ${succeeded} work${succeeded !== 1 ? "s" : ""}`);
    } else {
      toast.warning(
        `Deleted ${succeeded}, failed ${failed} of ${ids.length} works`,
      );
    }
    setSelectedIds(new Set());
    queryClient.invalidateQueries({ queryKey: ["works"] });
  };

  const handleBulkRefresh = async () => {
    const ids = Array.from(selectedIds);
    // If all filtered works are selected, use refreshAll
    if (allSelected && visibleWorks.length === (works?.length ?? 0)) {
      try {
        await refreshAllWorks({
          language: languageFilter || undefined,
          mediaType: mediaTypeFilter || undefined,
        });
        toast.success("Refreshing all works");
      } catch {
        toast.error("Failed to refresh works");
      }
    } else {
      const results = await Promise.allSettled(
        ids.map((id) => refreshWork(id)),
      );
      const succeeded = results.filter((r) => r.status === "fulfilled").length;
      const failed = results.filter((r) => r.status === "rejected").length;
      if (failed === 0) {
        toast.success(
          `Refreshing ${succeeded} work${succeeded !== 1 ? "s" : ""}`,
        );
      } else {
        toast.warning(
          `Refreshed ${succeeded}, failed ${failed} of ${ids.length} works`,
        );
      }
    }
    queryClient.invalidateQueries({ queryKey: ["works"] });
  };

  const handleSort = (field: WorkSortField) => {
    if (worksSort === field) {
      setWorksSort(field, worksSortDir === "asc" ? "desc" : "asc");
    } else {
      setWorksSort(field, "asc");
    }
    setPage(1);
  };

  if (collapseSeries ? allLoading && !allWorks : isLoading && !worksData)
    return <PageLoading />;
  if (!collapseSeries && error)
    return <ErrorState error={error} onRetry={() => refetch()} />;
  if (collapseSeries && allError)
    return <ErrorState error={allError} onRetry={() => refetchAll()} />;

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-2">
          <button
            onClick={toggleEditorMode}
            className={cn(
              "inline-flex items-center gap-1.5",
              editorMode ? "btn-primary" : "btn-secondary",
            )}
            title="Toggle editor mode"
          >
            <Pencil size={14} />
            <span className="hidden sm:inline">{editorMode ? "Editing" : "Edit"}</span>
          </button>
          <button
            onClick={() => rssSyncMutation.mutate()}
            disabled={rssSyncMutation.isPending}
            className="btn-secondary inline-flex items-center gap-1.5"
            title="Trigger RSS sync"
          >
            <Rss
              size={14}
              className={cn(rssSyncMutation.isPending && "animate-spin")}
            />
            <span className="hidden sm:inline">RSS Sync</span>
          </button>
          <button
            onClick={() => refreshMutation.mutate()}
            disabled={refreshMutation.isPending}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <RefreshCw
              size={14}
              className={cn(refreshMutation.isPending && "animate-spin")}
            />
            <span className="hidden sm:inline">Refresh All</span>
          </button>
          <button
            onClick={() => retryMutation.mutate()}
            disabled={retryMutation.isPending}
            className="btn-secondary inline-flex items-center gap-1.5"
          >
            <RotateCcw
              size={14}
              className={cn(retryMutation.isPending && "animate-spin")}
            />
            <span className="hidden sm:inline">Retry Incomplete</span>
          </button>
          <Link
            to="/work/add"
            className="btn-primary inline-flex items-center gap-1.5"
          >
            <Plus size={14} />
            <span className="hidden sm:inline">Add New</span>
          </Link>
        </div>
        <div className="flex items-center gap-2">
          <div className="relative flex-1 sm:flex-none">
            <Search
              size={14}
              className="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted"
            />
            <input
              type="text"
              placeholder={collapseSeries ? "Filter library..." : "Filter this page..."}
              value={searchQuery}
              onChange={(e) => {
                setSearchQuery(e.target.value);
                // Collapsed mode filters the whole library and paginates
                // client-side — a search from page N>1 would slice past the
                // (now smaller) result set into an empty dead-end.
                if (collapseSeries && page !== 1) setPage(1);
              }}
              className="h-8 w-full sm:w-auto rounded border border-border bg-zinc-800 pl-8 pr-3 text-sm text-zinc-100 placeholder:text-muted focus:border-brand focus:outline-none"
            />
          </div>
          <button
            onClick={handleToggleCollapseSeries}
            title={collapseSeries ? "Ungroup series" : "Group works by series"}
            className={cn(
              "inline-flex items-center gap-1.5",
              collapseSeries ? "btn-primary" : "btn-secondary",
            )}
          >
            <Layers size={14} />
            <span>{collapseSeries ? "Grouped" : "Group Series"}</span>
          </button>
          {worksView === "poster" && (
            <div className="hidden sm:flex items-center gap-1.5">
              <button
                onClick={() => setPosterZoom(Math.max(2, posterZoom - 1))}
                className="rounded p-0.5 text-muted hover:text-zinc-100 disabled:opacity-30"
                disabled={posterZoom <= 2}
              >
                <ZoomOut size={14} />
              </button>
              <input
                type="range"
                min={2}
                max={8}
                value={posterZoom}
                onChange={(e) => setPosterZoom(Number(e.target.value))}
                className="h-1 w-20 cursor-pointer appearance-none rounded bg-zinc-700 accent-brand"
              />
              <button
                onClick={() => setPosterZoom(Math.min(8, posterZoom + 1))}
                className="rounded p-0.5 text-muted hover:text-zinc-100 disabled:opacity-30"
                disabled={posterZoom >= 8}
              >
                <ZoomIn size={14} />
              </button>
            </div>
          )}
          <ViewToggle active={worksView} onChange={setWorksView} />
        </div>
      </PageToolbar>

      <PageContent>
        {/* Bulk action toolbar */}
        {editorMode && selectedIds.size > 0 && (
          <div className="mb-4 flex items-center gap-3 rounded-lg border border-brand/30 bg-zinc-800/80 px-4 py-2">
            <span className="text-sm text-zinc-300">
              {selectedIds.size} selected
            </span>
            <button
              onClick={() => setShowDeleteModal(true)}
              className="btn-danger inline-flex items-center gap-1.5 text-sm"
            >
              <Trash2 size={14} />
              Delete Selected
            </button>
            <button
              onClick={handleBulkRefresh}
              className="btn-secondary inline-flex items-center gap-1.5 text-sm"
            >
              <RefreshCw size={14} />
              Refresh Selected
            </button>
          </div>
        )}

        {/* Filter bar */}
        <div className="mb-4 flex flex-wrap items-center gap-2 sm:gap-3 overflow-x-auto">
          <select
            value={mediaTypeFilter}
            onChange={(e) => {
              setMediaTypeFilter(e.target.value as MediaType | "");
              setPage(1);
            }}
            className="h-8 rounded border border-border bg-zinc-800 px-2 text-sm text-zinc-100"
          >
            <option value="">All Media</option>
            <option value="ebook">Ebook</option>
            <option value="audiobook">Audiobook</option>
          </select>
          <select
            value={languageFilter}
            onChange={(e) => {
              setLanguageFilter(e.target.value);
              setPage(1);
            }}
            className="h-8 rounded border border-border bg-zinc-800 px-2 text-sm text-zinc-100"
          >
            <option value="">All Languages</option>
            {SUPPORTED_LANGUAGES.map((lang) => (
              <option key={lang.code} value={lang.code}>
                {lang.flag} {lang.englishName}
              </option>
            ))}
          </select>
          <SortDropdown
            active={worksSort}
            dir={worksSortDir}
            onChange={handleSort}
          />
          <div className="ml-auto">
            <Pagination
              page={page}
              totalPages={totalPages}
              total={total}
              pageSize={PAGE_SIZE}
              onPageChange={setPage}
            />
          </div>
        </div>

        {pageEntities.length === 0 ? (
          <EmptyState
            icon={<Book size={32} />}
            title="No works found"
            description={
              total > 0
                ? "Try adjusting your filters."
                : "Add your first work to get started."
            }
            action={
              total === 0 ? (
                <Link
                  to="/work/add"
                  className="btn-primary inline-flex items-center gap-1.5"
                >
                  <Plus size={14} />
                  Add Work
                </Link>
              ) : undefined
            }
          />
        ) : (
          <>
            {worksView === "table" && (
              <TableView
                entities={pageEntities}
                sort={worksSort}
                dir={worksSortDir}
                onSort={handleSort}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={toggleSelection}
                onToggleMany={toggleSelectMany}
                allSelected={allSelected}
                onToggleAll={toggleSelectAll}
                activeGrabs={activeGrabs}
                coverMediaType={mediaTypeFilter === "audiobook" ? "audiobook" as const : undefined}
                onToggleMonitor={handleToggleMonitor}
                expandedGroups={expandedGroups}
                onToggleGroup={toggleGroup}
              />
            )}
            {worksView === "poster" && (
              <PosterView
                entities={pageEntities}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={toggleSelection}
                onToggleMany={toggleSelectMany}
                columns={posterZoom}
                activeGrabs={activeGrabs}
                coverMediaType={mediaTypeFilter === "audiobook" ? "audiobook" as const : undefined}
                onToggleMonitor={handleToggleMonitor}
                expandedGroups={expandedGroups}
                onToggleGroup={toggleGroup}
              />
            )}
            {worksView === "overview" && (
              <OverviewView
                entities={pageEntities}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={toggleSelection}
                onToggleMany={toggleSelectMany}
                activeGrabs={activeGrabs}
                coverMediaType={mediaTypeFilter === "audiobook" ? "audiobook" as const : undefined}
                onToggleMonitor={handleToggleMonitor}
                expandedGroups={expandedGroups}
                onToggleGroup={toggleGroup}
              />
            )}

            <div className="mt-4">
              <Pagination
                page={page}
                totalPages={totalPages}
                total={total}
                pageSize={PAGE_SIZE}
                onPageChange={setPage}
              />
            </div>
          </>
        )}
      </PageContent>

      {/* Bulk delete confirmation modal */}
      <ConfirmModal
        open={showDeleteModal}
        onOpenChange={setShowDeleteModal}
        title={`Delete ${selectedIds.size} work${selectedIds.size !== 1 ? "s" : ""}?`}
        description={`Permanently delete ${selectedIds.size} work${selectedIds.size !== 1 ? "s" : ""} and all associated files on disk. This cannot be undone.`}
        confirmLabel="Delete"
        variant="danger"
        onConfirm={handleBulkDelete}
      />
    </>
  );
}

// --- View Toggle ---

function ViewToggle({
  active,
  onChange,
}: {
  active: string;
  onChange: (view: "table" | "poster" | "overview") => void;
}) {
  const views = [
    { key: "table" as const, icon: TableProperties, label: "Table" },
    { key: "poster" as const, icon: LayoutGrid, label: "Poster" },
    { key: "overview" as const, icon: LayoutList, label: "Overview" },
  ];

  return (
    <div className="flex rounded border border-border">
      {views.map(({ key, icon: Icon, label }) => (
        <button
          key={key}
          onClick={() => onChange(key)}
          title={label}
          className={cn(
            "inline-flex h-8 w-8 items-center justify-center text-sm",
            active === key
              ? "bg-brand text-white"
              : "text-muted hover:text-zinc-100",
          )}
        >
          <Icon size={14} />
        </button>
      ))}
    </div>
  );
}

// --- Sort Dropdown ---

function SortDropdown({
  active,
  dir,
  onChange,
}: {
  active: string;
  dir: "asc" | "desc";
  onChange: (field: WorkSortField) => void;
}) {
  const fields: { key: WorkSortField; label: string }[] = [
    { key: "recentlyDownloaded", label: "Recent" },
    { key: "title", label: "Title" },
    { key: "authorName", label: "Author" },
    { key: "year", label: "Year" },
    { key: "addedAt", label: "Date Added" },
  ];

  return (
    <div className="flex items-center gap-1 text-sm text-muted">
      <span>Sort:</span>
      {fields.map(({ key, label }) => (
        <button
          key={key}
          onClick={() => onChange(key)}
          className={cn(
            "rounded px-2 py-0.5",
            active === key
              ? "bg-zinc-700 text-zinc-100"
              : "hover:text-zinc-100",
          )}
        >
          {label}
          {active === key && (dir === "asc" ? " \u2191" : " \u2193")}
        </button>
      ))}
    </div>
  );
}

// --- Checkbox component ---

function SelectCheckbox({
  checked,
  onChange,
  className,
}: {
  checked: boolean;
  onChange: () => void;
  className?: string;
}) {
  return (
    <button
      onClick={(e) => {
        e.preventDefault();
        e.stopPropagation();
        onChange();
      }}
      className={cn(
        "inline-flex h-5 w-5 flex-shrink-0 items-center justify-center rounded border",
        checked
          ? "border-brand bg-brand text-white"
          : "border-zinc-500 bg-zinc-900 text-transparent hover:border-zinc-400",
        className,
      )}
    >
      <CheckSquare size={12} />
    </button>
  );
}

// --- Table View ---

function TableView({
  entities,
  sort,
  dir,
  onSort,
  editorMode,
  selectedIds,
  onToggle,
  onToggleMany,
  allSelected,
  onToggleAll,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
  expandedGroups,
  onToggleGroup,
}: {
  entities: WorkEntity[];
  sort: WorkSortField;
  dir: "asc" | "desc";
  onSort: (field: WorkSortField) => void;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleMany: (ids: number[]) => void;
  allSelected: boolean;
  onToggleAll: () => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
  expandedGroups: Set<string>;
  onToggleGroup: (key: string) => void;
}) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead className="border-b border-border">
          <tr>
            {editorMode && (
              <th className="w-10 px-3 py-2">
                <SelectCheckbox checked={allSelected} onChange={onToggleAll} />
              </th>
            )}
            <th className="w-10 px-3 py-2" />
            <SortHeader field="title" activeField={sort} dir={dir} onSort={onSort}>Title</SortHeader>
            <SortHeader field="authorName" activeField={sort} dir={dir} onSort={onSort} className="hidden sm:table-cell">Author</SortHeader>
            <SortHeader field="year" activeField={sort} dir={dir} onSort={onSort} className="hidden md:table-cell">Year</SortHeader>
            <th className="hidden md:table-cell px-3 py-2 text-left text-xs font-medium uppercase text-muted">
              Library
            </th>
            <th className="hidden md:table-cell px-3 py-2 text-left text-xs font-medium uppercase text-muted">
              Progress
            </th>
            <SortHeader field="addedAt" activeField={sort} dir={dir} onSort={onSort} className="hidden lg:table-cell">Added</SortHeader>
          </tr>
        </thead>
        <tbody className="divide-y divide-border">
          {entities.map((entity) =>
            entity.kind === "work" ? (
              <WorkTableRow
                key={entity.work.id}
                work={entity.work}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={onToggle}
                activeGrabs={activeGrabs}
                coverMediaType={coverMediaType}
                onToggleMonitor={onToggleMonitor}
              />
            ) : (
              <SeriesGroupRows
                key={entity.key}
                group={entity}
                expanded={expandedGroups.has(entity.key)}
                onToggleGroup={onToggleGroup}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={onToggle}
                onToggleMany={onToggleMany}
                activeGrabs={activeGrabs}
                coverMediaType={coverMediaType}
                onToggleMonitor={onToggleMonitor}
              />
            ),
          )}
        </tbody>
      </table>
    </div>
  );
}

function WorkTableRow({
  work,
  editorMode,
  selectedIds,
  onToggle,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
  member,
}: {
  work: WorkDetailResponse;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
  member?: boolean;
}) {
  return (
    <tr
      className={cn(
        "hover:bg-zinc-800/50",
        member && "bg-zinc-800/30",
        editorMode && selectedIds.has(work.id) && "bg-brand/10",
      )}
    >
      {editorMode && (
        <td className="px-3 py-2">
          <SelectCheckbox
            checked={selectedIds.has(work.id)}
            onChange={() => onToggle(work.id)}
          />
        </td>
      )}
      <td className="px-3 py-2">
        <BookCover
          workId={work.id}
          title={work.title}
          authorName={work.authorName}
          coverVersion={work.coverMtime ?? undefined}
          mediaType={coverMediaType}
          className="h-8 w-8"
          iconSize={12}
        />
      </td>
      <td className={cn("px-3 py-2", member && "pl-8")}>
        <Link
          to={`/work/${work.id}`}
          className="font-medium text-zinc-100 hover:text-brand"
        >
          {work.title}
        </Link>
        {member && work.seriesPosition != null && (
          <span className="ml-2 text-xs text-zinc-500">#{work.seriesPosition}</span>
        )}
      </td>
      <td className="hidden sm:table-cell px-3 py-2 text-muted">
        {work.authorId ? (
          <Link to={`/author/${work.authorId}`} className="hover:text-brand">
            {work.authorName}
          </Link>
        ) : (
          work.authorName
        )}
      </td>
      <td className="hidden md:table-cell px-3 py-2 text-muted">
        {work.year ?? "\u2014"}
      </td>
      <td className="hidden md:table-cell px-3 py-2">
        <MediaStatusRow work={work} activeGrabs={activeGrabs} onToggleMonitor={onToggleMonitor} />
      </td>
      <td className="hidden md:table-cell px-3 py-2">
        {(() => {
          const bp = bestProgress(work.libraryItems);
          if (!bp) return null;
          return (
            <div className="flex items-center gap-2">
              <div className="w-16 h-1.5 bg-zinc-700 rounded-full overflow-hidden">
                <div
                  className="h-full bg-brand rounded-full"
                  style={{ width: `${Math.min((bp.progressPct ?? 0) * 100, 100)}%` }}
                />
              </div>
              <ProgressBadge
                progressPct={bp.progressPct}
                mediaType={bp.mediaType}
                durationSeconds={bp.durationSeconds}
                finishedAt={bp.finishedAt}
              />
            </div>
          );
        })()}
      </td>
      <td className="hidden lg:table-cell px-3 py-2 text-muted">
        {formatRelativeDate(work.addedAt)}
      </td>
    </tr>
  );
}

function SeriesGroupRows({
  group,
  expanded,
  onToggleGroup,
  editorMode,
  selectedIds,
  onToggle,
  onToggleMany,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
}: {
  group: SeriesGroup;
  expanded: boolean;
  onToggleGroup: (key: string) => void;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleMany: (ids: number[]) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
}) {
  const first = group.works[0];
  const inLibrary = groupLibraryCount(group);
  const memberIds = group.works.map((w) => w.id);
  const allMembersSelected = memberIds.every((id) => selectedIds.has(id));
  const Chevron = expanded ? ChevronDown : ChevronRight;
  if (!first) return null;
  const latestAdded = group.works.reduce(
    (max, w) => (w.addedAt > max ? w.addedAt : max),
    first.addedAt,
  );

  return (
    <>
      <tr
        onClick={() => onToggleGroup(group.key)}
        className="cursor-pointer hover:bg-zinc-800/50"
      >
        {editorMode && (
          <td className="px-3 py-2" onClick={(e) => e.stopPropagation()}>
            <SelectCheckbox
              checked={allMembersSelected}
              onChange={() => onToggleMany(memberIds)}
            />
          </td>
        )}
        <td className="px-3 py-2">
          <BookCover
            workId={first.id}
            title={first.title}
            authorName={first.authorName}
            coverVersion={first.coverMtime ?? undefined}
            mediaType={coverMediaType}
            className="h-8 w-8"
            iconSize={12}
          />
        </td>
        <td className="px-3 py-2">
          <span className="inline-flex items-center gap-1.5 font-medium text-zinc-100">
            <Chevron size={14} className="text-muted" />
            {group.seriesId != null ? (
              <Link
                to={`/series/${group.seriesId}`}
                onClick={(e) => e.stopPropagation()}
                className="hover:text-brand"
              >
                {group.seriesName}
              </Link>
            ) : (
              group.seriesName
            )}
            <span className="rounded bg-zinc-700/70 px-1.5 py-0.5 text-xs font-normal text-zinc-300">
              {group.works.length} books
            </span>
          </span>
        </td>
        <td className="hidden sm:table-cell px-3 py-2 text-muted">
          {first.authorId ? (
            <Link
              to={`/author/${first.authorId}`}
              onClick={(e) => e.stopPropagation()}
              className="hover:text-brand"
            >
              {first.authorName}
            </Link>
          ) : (
            first.authorName
          )}
        </td>
        <td className="hidden md:table-cell px-3 py-2 text-muted">{"\u2014"}</td>
        <td className="hidden md:table-cell px-3 py-2 text-xs text-muted">
          {inLibrary > 0 ? (
            <span className="text-green-600">
              {inLibrary}/{group.works.length} downloaded
            </span>
          ) : (
            <span>0/{group.works.length} downloaded</span>
          )}
        </td>
        <td className="hidden md:table-cell px-3 py-2" />
        <td className="hidden lg:table-cell px-3 py-2 text-muted">
          {formatRelativeDate(latestAdded)}
        </td>
      </tr>
      {expanded &&
        group.works.map((work) => (
          <WorkTableRow
            key={work.id}
            work={work}
            editorMode={editorMode}
            selectedIds={selectedIds}
            onToggle={onToggle}
            activeGrabs={activeGrabs}
            coverMediaType={coverMediaType}
            onToggleMonitor={onToggleMonitor}
            member
          />
        ))}
    </>
  );
}

// --- Shared media status row ---

// --- Poster View ---

function PosterView({
  entities,
  editorMode,
  selectedIds,
  onToggle,
  onToggleMany,
  columns,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
  expandedGroups,
  onToggleGroup,
}: {
  entities: WorkEntity[];
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleMany: (ids: number[]) => void;
  columns: number;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
  expandedGroups: Set<string>;
  onToggleGroup: (key: string) => void;
}) {
  return (
    <div className="grid gap-3 sm:gap-4 grid-cols-2 sm:grid-cols-3 md:grid-cols-4" style={{ gridTemplateColumns: window.innerWidth >= 640 ? `repeat(${columns}, minmax(0, 1fr))` : undefined }}>
      {entities.map((entity) => {
        if (entity.kind === "work") {
          return (
            <WorkPosterCard
              key={entity.work.id}
              work={entity.work}
              editorMode={editorMode}
              selectedIds={selectedIds}
              onToggle={onToggle}
              activeGrabs={activeGrabs}
              coverMediaType={coverMediaType}
              onToggleMonitor={onToggleMonitor}
            />
          );
        }
        const expanded = expandedGroups.has(entity.key);
        return (
          <SeriesPosterCards
            key={entity.key}
            group={entity}
            expanded={expanded}
            onToggleGroup={onToggleGroup}
            editorMode={editorMode}
            selectedIds={selectedIds}
            onToggle={onToggle}
            onToggleMany={onToggleMany}
            activeGrabs={activeGrabs}
            coverMediaType={coverMediaType}
            onToggleMonitor={onToggleMonitor}
          />
        );
      })}
    </div>
  );
}

function WorkPosterCard({
  work,
  editorMode,
  selectedIds,
  onToggle,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
  member,
}: {
  work: WorkDetailResponse;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
  member?: boolean;
}) {
  const navigate = useNavigate();
  const isSelected = selectedIds.has(work.id);

  return (
    <div className="relative">
      {editorMode && (
        <div className="absolute left-2 top-2 z-10">
          <SelectCheckbox
            checked={isSelected}
            onChange={() => onToggle(work.id)}
          />
        </div>
      )}
      <div
        onClick={() => navigate(`/work/${work.id}`)}
        className={cn(
          "group block cursor-pointer overflow-hidden rounded-lg border bg-zinc-800",
          editorMode && isSelected ? "border-brand" : member ? "border-brand/40" : "border-border",
        )}
      >
        <div className="aspect-[2/3] overflow-hidden relative">
          <BookCover
            workId={work.id}
            title={work.title}
            authorName={work.authorName}
            coverVersion={work.coverMtime ?? undefined}
            mediaType={coverMediaType}
            className="h-full w-full"
            iconSize={24}
          />
          <MediaOverlay work={work} />
          <ProgressBar progressPct={bestProgress(work.libraryItems)?.progressPct ?? null} />
        </div>
        <div className="p-2.5 space-y-1">
          <p className="truncate text-sm font-medium text-zinc-100">
            {work.title}
            {(work.year || work.language) && (
              <span className="text-xs text-muted font-normal">
                {" "}({[work.year, work.language?.toUpperCase()].filter(Boolean).join(" / ")})
              </span>
            )}
          </p>
          <p className="truncate text-xs text-zinc-400">
            {work.authorId ? (
              <Link to={`/author/${work.authorId}`} onClick={(e) => e.stopPropagation()} className="hover:text-brand">
                {work.authorName}
              </Link>
            ) : work.authorName}
          </p>
          <p className="min-h-4 truncate text-xs text-zinc-500">
            {work.seriesName && (
              <>
                {work.seriesId ? (
                  <Link to={`/series/${work.seriesId}`} onClick={(e) => e.stopPropagation()} className="hover:text-brand">
                    {work.seriesName}
                  </Link>
                ) : work.seriesName}
                {work.seriesPosition != null && ` #${work.seriesPosition}`}
              </>
            )}
          </p>
          {(() => {
            const bp = bestProgress(work.libraryItems);
            return bp ? (
              <ProgressBadge
                progressPct={bp.progressPct}
                mediaType={bp.mediaType}
                durationSeconds={bp.durationSeconds}
                finishedAt={bp.finishedAt}
              />
            ) : (
              <span className="text-xs text-zinc-500">Not started</span>
            );
          })()}
          <MediaStatusRow work={work} activeGrabs={activeGrabs} onToggleMonitor={onToggleMonitor} />
        </div>
      </div>
    </div>
  );
}

function SeriesPosterCards({
  group,
  expanded,
  onToggleGroup,
  editorMode,
  selectedIds,
  onToggle,
  onToggleMany,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
}: {
  group: SeriesGroup;
  expanded: boolean;
  onToggleGroup: (key: string) => void;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleMany: (ids: number[]) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
}) {
  const first = group.works[0];
  const inLibrary = groupLibraryCount(group);
  const memberIds = group.works.map((w) => w.id);
  const allMembersSelected = memberIds.every((id) => selectedIds.has(id));
  const Chevron = expanded ? ChevronDown : ChevronRight;
  if (!first) return null;

  return (
    <>
      <div className="relative">
        {editorMode && (
          <div className="absolute left-2 top-2 z-10">
            <SelectCheckbox
              checked={allMembersSelected}
              onChange={() => onToggleMany(memberIds)}
            />
          </div>
        )}
        <div
          onClick={() => onToggleGroup(group.key)}
          className={cn(
            "group block cursor-pointer overflow-hidden rounded-lg border bg-zinc-800",
            expanded ? "border-brand" : "border-border",
          )}
        >
          <div className="aspect-[2/3] overflow-hidden relative">
            <BookCover
              workId={first.id}
              title={first.title}
              authorName={first.authorName}
              coverVersion={first.coverMtime ?? undefined}
              mediaType={coverMediaType}
              className="h-full w-full"
              iconSize={24}
            />
            <div className="absolute right-2 top-2 flex h-6 min-w-[24px] items-center justify-center rounded bg-brand px-1 text-sm font-bold text-white shadow-md shadow-black/50">
              {group.works.length}
            </div>
          </div>
          <div className="p-2.5 space-y-1">
            <p className="truncate text-sm font-medium text-zinc-100">
              <Chevron size={12} className="mr-1 inline text-muted" />
              {group.seriesName}
            </p>
            <p className="truncate text-xs text-zinc-400">
              {first.authorId ? (
                <Link to={`/author/${first.authorId}`} onClick={(e) => e.stopPropagation()} className="hover:text-brand">
                  {first.authorName}
                </Link>
              ) : first.authorName}
            </p>
            <p className="truncate text-xs text-zinc-500">
              {group.works.length} books
              {inLibrary > 0 && (
                <span className="text-green-600"> · {inLibrary} downloaded</span>
              )}
            </p>
            {group.seriesId != null && (
              <Link
                to={`/series/${group.seriesId}`}
                onClick={(e) => e.stopPropagation()}
                className="inline-block text-xs text-brand hover:underline"
              >
                Series page
              </Link>
            )}
          </div>
        </div>
      </div>
      {expanded &&
        group.works.map((work) => (
          <WorkPosterCard
            key={work.id}
            work={work}
            editorMode={editorMode}
            selectedIds={selectedIds}
            onToggle={onToggle}
            activeGrabs={activeGrabs}
            coverMediaType={coverMediaType}
            onToggleMonitor={onToggleMonitor}
            member
          />
        ))}
    </>
  );
}

// --- Overview View ---

function OverviewView({
  entities,
  editorMode,
  selectedIds,
  onToggle,
  onToggleMany,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
  expandedGroups,
  onToggleGroup,
}: {
  entities: WorkEntity[];
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleMany: (ids: number[]) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
  expandedGroups: Set<string>;
  onToggleGroup: (key: string) => void;
}) {
  return (
    <div className="space-y-4">
      {entities.map((entity) =>
        entity.kind === "work" ? (
          <WorkOverviewCard
            key={entity.work.id}
            work={entity.work}
            editorMode={editorMode}
            selectedIds={selectedIds}
            onToggle={onToggle}
            activeGrabs={activeGrabs}
            coverMediaType={coverMediaType}
            onToggleMonitor={onToggleMonitor}
          />
        ) : (
          <SeriesOverviewSection
            key={entity.key}
            group={entity}
            expanded={expandedGroups.has(entity.key)}
            onToggleGroup={onToggleGroup}
            editorMode={editorMode}
            selectedIds={selectedIds}
            onToggle={onToggle}
            onToggleMany={onToggleMany}
            activeGrabs={activeGrabs}
            coverMediaType={coverMediaType}
            onToggleMonitor={onToggleMonitor}
          />
        ),
      )}
    </div>
  );
}

function SeriesOverviewSection({
  group,
  expanded,
  onToggleGroup,
  editorMode,
  selectedIds,
  onToggle,
  onToggleMany,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
}: {
  group: SeriesGroup;
  expanded: boolean;
  onToggleGroup: (key: string) => void;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  onToggleMany: (ids: number[]) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
}) {
  const first = group.works[0];
  const inLibrary = groupLibraryCount(group);
  const memberIds = group.works.map((w) => w.id);
  const allMembersSelected = memberIds.every((id) => selectedIds.has(id));
  const Chevron = expanded ? ChevronDown : ChevronRight;
  if (!first) return null;

  return (
    <div>
      <div
        onClick={() => onToggleGroup(group.key)}
        className={cn(
          "flex cursor-pointer items-center gap-4 rounded-lg border bg-zinc-800 p-4",
          expanded ? "border-brand" : "border-border hover:border-zinc-600",
        )}
      >
        {editorMode && (
          <div className="flex flex-shrink-0 items-start" onClick={(e) => e.stopPropagation()}>
            <SelectCheckbox
              checked={allMembersSelected}
              onChange={() => onToggleMany(memberIds)}
            />
          </div>
        )}
        <BookCover
          workId={first.id}
          title={first.title}
          authorName={first.authorName}
          coverVersion={first.coverMtime ?? undefined}
          mediaType={coverMediaType}
          className="h-20 w-14"
          iconSize={18}
        />
        <div className="min-w-0 flex-1">
          <h3 className="flex items-center gap-1.5 font-medium text-zinc-100">
            <Chevron size={14} className="text-muted" />
            {group.seriesId != null ? (
              <Link
                to={`/series/${group.seriesId}`}
                onClick={(e) => e.stopPropagation()}
                className="hover:text-brand"
              >
                {group.seriesName}
              </Link>
            ) : (
              group.seriesName
            )}
          </h3>
          <p className="text-sm text-muted">
            {first.authorId ? (
              <Link
                to={`/author/${first.authorId}`}
                onClick={(e) => e.stopPropagation()}
                className="hover:text-brand"
              >
                {first.authorName}
              </Link>
            ) : (
              first.authorName
            )}
          </p>
          <p className="mt-1 text-xs text-zinc-500">
            {group.works.length} books
            {inLibrary > 0 && (
              <span className="text-green-600"> · {inLibrary} downloaded</span>
            )}
          </p>
        </div>
      </div>
      {expanded && (
        <div className="ml-6 mt-3 space-y-3">
          {group.works.map((work) => (
            <WorkOverviewCard
              key={work.id}
              work={work}
              editorMode={editorMode}
              selectedIds={selectedIds}
              onToggle={onToggle}
              activeGrabs={activeGrabs}
              coverMediaType={coverMediaType}
              onToggleMonitor={onToggleMonitor}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function WorkOverviewCard({
  work,
  editorMode,
  selectedIds,
  onToggle,
  activeGrabs,
  coverMediaType,
  onToggleMonitor,
}: {
  work: WorkDetailResponse;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  activeGrabs: Set<string>;
  coverMediaType?: "ebook" | "audiobook";
  onToggleMonitor: (workId: number, field: "monitorEbook" | "monitorAudiobook") => void;
}) {
  const navigate = useNavigate();
  const isSelected = selectedIds.has(work.id);

  return (
          <div
            onClick={() => navigate(`/work/${work.id}`)}
            className={cn(
              "flex cursor-pointer gap-4 rounded-lg border bg-zinc-800 p-4",
              editorMode && isSelected
                ? "border-brand"
                : "border-border hover:border-zinc-600",
            )}
          >
            {editorMode && (
              <div className="flex flex-shrink-0 items-start pt-1">
                <SelectCheckbox
                  checked={isSelected}
                  onChange={() => onToggle(work.id)}
                />
              </div>
            )}
            <div className="flex min-w-0 flex-1 gap-3 sm:gap-4">
              <div className="relative flex-shrink-0">
                <BookCover
                  workId={work.id}
                  title={work.title}
                  authorName={work.authorName}
                  coverVersion={work.coverMtime ?? undefined}
                  mediaType={coverMediaType}
                  className="h-20 w-14 sm:h-28 sm:w-20"
                  iconSize={18}
                />
                <ProgressBar progressPct={bestProgress(work.libraryItems)?.progressPct ?? null} />
              </div>
              <div className="min-w-0 flex-1">
                <h3 className="font-medium text-zinc-100">
                  {work.title}
                  {(work.year || work.language) && (
                    <span className="text-sm text-muted font-normal">
                      {" "}({[work.year, work.language?.toUpperCase()].filter(Boolean).join(" / ")})
                    </span>
                  )}
                </h3>
                <p className="text-sm text-muted">
                  {work.authorId ? (
                    <Link to={`/author/${work.authorId}`} onClick={(e) => e.stopPropagation()} className="hover:text-brand">
                      {work.authorName}
                    </Link>
                  ) : work.authorName}
                  {work.seriesName && (
                    <span className="ml-2 text-zinc-500">
                      {work.seriesName}
                      {work.seriesPosition != null && ` #${work.seriesPosition}`}
                    </span>
                  )}
                </p>
                <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-xs text-zinc-500">
                  {work.rating != null && (
                    <span className="inline-flex items-center gap-0.5">
                      <Star size={11} className="text-amber-400 fill-amber-400" />
                      {work.rating.toFixed(1)}
                    </span>
                  )}
                  {work.pageCount != null && (
                    <span>{work.pageCount}p</span>
                  )}
                  {work.durationSeconds != null && (
                    <span>{formatDuration(work.durationSeconds)}</span>
                  )}
                  {work.narrator && work.narrator.length > 0 && (
                    <span>Narrated by {work.narrator.slice(0, 2).join(", ")}</span>
                  )}
                </div>
                <div className="mt-1.5 flex items-center gap-2">
                  <MediaStatusRow work={work} activeGrabs={activeGrabs} onToggleMonitor={onToggleMonitor} />
                  {(() => {
                    const bp = bestProgress(work.libraryItems);
                    return bp ? (
                      <ProgressBadge
                        progressPct={bp.progressPct}
                        mediaType={bp.mediaType}
                        durationSeconds={bp.durationSeconds}
                        finishedAt={bp.finishedAt}
                      />
                    ) : null;
                  })()}
                </div>
                {work.genres && work.genres.length > 0 && (
                  <div className="mt-1.5 flex flex-wrap gap-1">
                    {work.genres.slice(0, 3).map((g) => (
                      <span key={g} className="rounded bg-zinc-700/60 px-1.5 py-0.5 text-[10px] text-zinc-400">
                        {g}
                      </span>
                    ))}
                  </div>
                )}
                {work.description && (
                  <p className="mt-2 line-clamp-2 text-sm text-zinc-400">
                    {work.description}
                  </p>
                )}
              </div>
            </div>
          </div>
  );
}

function MediaOverlay({ work }: { work: WorkDetailResponse }) {
  const ebookItem = work.libraryItems?.find((li) => li.mediaType === "ebook");
  const audioItem = work.libraryItems?.find((li) => li.mediaType === "audiobook");
  if (!ebookItem && !audioItem) return null;

  const isTouch = window.matchMedia("(pointer: coarse)").matches;

  if (isTouch) {
    return (
      <div className="absolute bottom-2 right-2 flex gap-1.5 z-10">
        {ebookItem && (
          <Link
            to={`/read/${ebookItem.id}`}
            onClick={(e) => e.stopPropagation()}
            className="rounded-full bg-black/60 backdrop-blur-sm p-2 text-zinc-200 hover:text-white min-h-[44px] min-w-[44px] flex items-center justify-center"
          >
            <BookOpen size={18} />
          </Link>
        )}
        {audioItem && (
          <Link
            to={`/listen/${audioItem.id}?workId=${work.id}`}
            onClick={(e) => e.stopPropagation()}
            className="rounded-full bg-black/60 backdrop-blur-sm p-2 text-zinc-200 hover:text-white min-h-[44px] min-w-[44px] flex items-center justify-center"
          >
            <Headphones size={18} />
          </Link>
        )}
      </div>
    );
  }

  return (
    <div className="absolute inset-0 flex items-center justify-center gap-3 opacity-0 group-hover:opacity-100 transition-opacity bg-black/40">
      {ebookItem && (
        <Link
          to={`/read/${ebookItem.id}`}
          onClick={(e) => e.stopPropagation()}
          className="rounded-full bg-black/60 p-2.5 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
        >
          <BookOpen size={20} />
        </Link>
      )}
      {audioItem && (
        <Link
          to={`/listen/${audioItem.id}?workId=${work.id}`}
          onClick={(e) => e.stopPropagation()}
          className="rounded-full bg-black/60 p-2.5 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
        >
          <Headphones size={20} />
        </Link>
      )}
    </div>
  );
}
