import { useCallback, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Book,
  Plus,
  RefreshCw,
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
} from "lucide-react";
import { listWorks, refreshAllWorks, deleteWork, refreshWork, triggerRssSync, getQueue } from "@/api";
import { sortWorks } from "@/utils/works";
import type { WorkSortField } from "@/utils/works";
import { useUIStore } from "@/stores/ui";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
// EnrichmentBadge removed — metadata moved to work detail Metadata tab.
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { cn } from "@/utils/cn";
import { SortHeader } from "@/components/Page/SortHeader";
import { formatRelativeDate } from "@/utils/format";
import { MediaStatusRow } from "@/components/MediaStatusRow";
import { BookCover } from "@/components/BookCover";
import type {
  WorkDetailResponse,
  MediaType,
} from "@/types/api";

const ALPHA = "ABCDEFGHIJKLMNOPQRSTUVWXYZ#".split("");

export function WorksPage() {
  const queryClient = useQueryClient();

  const {
    data: works,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["works"],
    queryFn: listWorks,
    select: (res) => res.items,
  });

  const refreshMutation = useMutation({
    mutationFn: refreshAllWorks,
    onSuccess: () => toast.success("Refreshing all works"),
    onError: () => toast.error("Failed to refresh works"),
  });

  const rssSyncMutation = useMutation({
    mutationFn: triggerRssSync,
    onSuccess: () => toast.success("RSS sync started"),
    onError: () => toast.error("RSS sync already running"),
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

  const worksView = useUIStore((s) => s.worksView);
  const setWorksView = useUIStore((s) => s.setWorksView);
  const worksSort = useUIStore((s) => s.worksSort) as WorkSortField;
  const worksSortDir = useUIStore((s) => s.worksSortDir);
  const setWorksSort = useUIStore((s) => s.setWorksSort);
  const posterZoom = useUIStore((s) => s.posterZoom);
  const setPosterZoom = useUIStore((s) => s.setPosterZoom);

  const mediaTypeFilter = useUIStore((s) => s.worksMediaFilter) as MediaType | "";
  const setMediaTypeFilter = useUIStore((s) => s.setWorksMediaFilter);
  const [searchQuery, setSearchQuery] = useState("");

  // Editor mode state
  const [editorMode, setEditorMode] = useState(false);
  const [selectedIds, setSelectedIds] = useState<Set<number>>(new Set());
  const [showDeleteModal, setShowDeleteModal] = useState(false);
  const [deleteFiles, setDeleteFiles] = useState(false);

  const toggleEditorMode = useCallback(() => {
    setEditorMode((prev) => {
      if (prev) {
        // Turning off — clear selection
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
    let result = works;
    if (mediaTypeFilter) {
      result = result.filter((w) =>
        w.libraryItems.some((item) => item.mediaType === mediaTypeFilter),
      );
    }
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      result = result.filter(
        (w) =>
          w.title.toLowerCase().includes(q) ||
          w.authorName.toLowerCase().includes(q),
      );
    }
    return sortWorks(result, worksSort, worksSortDir);
  }, [
    works,
    mediaTypeFilter,
    searchQuery,
    worksSort,
    worksSortDir,
  ]);

  const allSelected =
    filtered.length > 0 && filtered.every((w) => selectedIds.has(w.id));

  const toggleSelectAll = useCallback(() => {
    if (allSelected) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(filtered.map((w) => w.id)));
    }
  }, [allSelected, filtered]);

  const handleBulkDelete = async () => {
    const ids = Array.from(selectedIds);
    const results = await Promise.allSettled(
      ids.map((id) => deleteWork(id, deleteFiles)),
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
    setDeleteFiles(false);
    queryClient.invalidateQueries({ queryKey: ["works"] });
  };

  const handleBulkRefresh = async () => {
    const ids = Array.from(selectedIds);
    // If all filtered works are selected, use refreshAll
    if (allSelected && filtered.length === (works?.length ?? 0)) {
      try {
        await refreshAllWorks();
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
  };

  const jumpTo = (letter: string) => {
    const el = document.getElementById(`jump-${letter}`);
    el?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

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
              placeholder="Filter..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="h-8 w-full sm:w-auto rounded border border-border bg-zinc-800 pl-8 pr-3 text-sm text-zinc-100 placeholder:text-muted focus:border-brand focus:outline-none"
            />
          </div>
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
            onChange={(e) =>
              setMediaTypeFilter(e.target.value as MediaType | "")
            }
            className="h-8 rounded border border-border bg-zinc-800 px-2 text-sm text-zinc-100"
          >
            <option value="">All Media</option>
            <option value="ebook">Ebook</option>
            <option value="audiobook">Audiobook</option>
          </select>
          <SortDropdown
            active={worksSort}
            dir={worksSortDir}
            onChange={handleSort}
          />
        </div>

        {filtered.length === 0 ? (
          <EmptyState
            icon={<Book size={32} />}
            title="No works found"
            description={
              works?.length
                ? "Try adjusting your filters."
                : "Add your first work to get started."
            }
            action={
              !works?.length ? (
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
            {/* Jump bar (title sort only) */}
            {worksSort === "title" && (
              <div className="mb-3 flex flex-wrap gap-1">
                {ALPHA.map((letter) => (
                  <button
                    key={letter}
                    onClick={() => jumpTo(letter)}
                    className="h-6 w-6 rounded text-xs text-muted hover:bg-zinc-700 hover:text-zinc-100"
                  >
                    {letter}
                  </button>
                ))}
              </div>
            )}

            {worksView === "table" && (
              <TableView
                works={filtered}
                sort={worksSort}
                dir={worksSortDir}
                onSort={handleSort}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={toggleSelection}
                allSelected={allSelected}
                onToggleAll={toggleSelectAll}
                activeGrabs={activeGrabs}
              />
            )}
            {worksView === "poster" && (
              <PosterView
                works={filtered}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={toggleSelection}
                columns={posterZoom}
                activeGrabs={activeGrabs}
              />
            )}
            {worksView === "overview" && (
              <OverviewView
                works={filtered}
                editorMode={editorMode}
                selectedIds={selectedIds}
                onToggle={toggleSelection}
                activeGrabs={activeGrabs}
              />
            )}
          </>
        )}
      </PageContent>

      {/* Bulk delete confirmation modal */}
      <ConfirmModal
        open={showDeleteModal}
        onOpenChange={setShowDeleteModal}
        title={`Delete ${selectedIds.size} work${selectedIds.size !== 1 ? "s" : ""}?`}
        description="This action cannot be undone."
        confirmLabel="Delete"
        variant="danger"
        onConfirm={handleBulkDelete}
      >
        <label className="mt-4 flex items-center gap-2 text-sm text-zinc-300">
          <input
            type="checkbox"
            checked={deleteFiles}
            onChange={(e) => setDeleteFiles(e.target.checked)}
            className="h-4 w-4 rounded border-border bg-zinc-900"
          />
          Also delete files from disk
        </label>
      </ConfirmModal>
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

// --- Jump anchor helper ---

function jumpLetter(title: string): string {
  const first = (title[0] ?? "").toUpperCase();
  return /[A-Z]/.test(first) ? first : "#";
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
  works,
  sort,
  dir,
  onSort,
  editorMode,
  selectedIds,
  onToggle,
  allSelected,
  onToggleAll,
  activeGrabs,
}: {
  works: WorkDetailResponse[];
  sort: WorkSortField;
  dir: "asc" | "desc";
  onSort: (field: WorkSortField) => void;
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  allSelected: boolean;
  onToggleAll: () => void;
  activeGrabs: Set<string>;
}) {
  let lastLetter = "";

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
            <SortHeader field="addedAt" activeField={sort} dir={dir} onSort={onSort} className="hidden lg:table-cell">Added</SortHeader>
          </tr>
        </thead>
        <tbody className="divide-y divide-border">
          {works.map((work) => {
            const letter = jumpLetter(work.sortTitle ?? work.title);
            const showAnchor = letter !== lastLetter;
            if (showAnchor) lastLetter = letter;
            return (
              <tr
                key={work.id}
                className={cn(
                  "hover:bg-zinc-800/50",
                  editorMode && selectedIds.has(work.id) && "bg-brand/10",
                )}
                id={showAnchor ? `jump-${letter}` : undefined}
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
                    className="h-8 w-8"
                    iconSize={12}
                  />
                </td>
                <td className="px-3 py-2">
                  <Link
                    to={`/work/${work.id}`}
                    className="font-medium text-zinc-100 hover:text-brand"
                  >
                    {work.title}
                  </Link>
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
                  <MediaStatusRow work={work} activeGrabs={activeGrabs} />
                </td>
                <td className="hidden lg:table-cell px-3 py-2 text-muted">
                  {formatRelativeDate(work.addedAt)}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

// --- Shared media status row ---

// --- Poster View ---

function PosterView({
  works,
  editorMode,
  selectedIds,
  onToggle,
  columns,
  activeGrabs,
}: {
  works: WorkDetailResponse[];
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  columns: number;
  activeGrabs: Set<string>;
}) {
  const navigate = useNavigate();

  return (
    <div className="grid gap-3 sm:gap-4 grid-cols-2 sm:grid-cols-3 md:grid-cols-4" style={{ gridTemplateColumns: window.innerWidth >= 640 ? `repeat(${columns}, minmax(0, 1fr))` : undefined }}>
      {works.map((work) => {
        const isSelected = selectedIds.has(work.id);

        return (
          <div key={work.id} className="relative">
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
                editorMode && isSelected ? "border-brand" : "border-border",
              )}
            >
              <div className="aspect-[2/3] overflow-hidden">
                <BookCover
                  workId={work.id}
                  title={work.title}
                  className="h-full w-full"
                  iconSize={24}
                />
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
                <MediaStatusRow work={work} activeGrabs={activeGrabs} />
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}

// --- Overview View ---

function OverviewView({
  works,
  editorMode,
  selectedIds,
  onToggle,
  activeGrabs,
}: {
  works: WorkDetailResponse[];
  editorMode: boolean;
  selectedIds: Set<number>;
  onToggle: (id: number) => void;
  activeGrabs: Set<string>;
}) {
  const navigate = useNavigate();

  return (
    <div className="space-y-4">
      {works.map((work) => {
        const isSelected = selectedIds.has(work.id);

        return (
          <div
            key={work.id}
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
              <BookCover
                workId={work.id}
                title={work.title}
                className="h-20 w-14 sm:h-28 sm:w-20 flex-shrink-0"
                iconSize={18}
              />
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
                </p>
                <div className="mt-1.5">
                  <MediaStatusRow work={work} activeGrabs={activeGrabs} />
                </div>
                {work.description && (
                  <p className="mt-2 line-clamp-2 text-sm text-zinc-400">
                    {work.description}
                  </p>
                )}
              </div>
            </div>
          </div>
        );
      })}
    </div>
  );
}
