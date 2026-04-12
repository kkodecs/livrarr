import { useMemo, useState } from "react";
import { Link } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Plus, LayoutGrid, List, Rows3, Trash2 } from "lucide-react";
import { toast } from "sonner";
import { listAuthors, updateAuthor, deleteAuthor } from "@/api";
import { useUIStore } from "@/stores/ui";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { formatRelativeDate } from "@/utils/format";
import { cn } from "@/utils/cn";
import { HelpTip } from "@/components/HelpTip";
import type { AuthorResponse } from "@/types/api";

type FilterStatus = "all" | "monitored" | "unmonitored";

function sortAuthors(
  authors: AuthorResponse[],
  field: string,
  dir: "asc" | "desc",
): AuthorResponse[] {
  const sorted = [...authors].sort((a, b) => {
    switch (field) {
      case "name":
        return a.name.localeCompare(b.name);
      case "dateAdded":
        return a.addedAt.localeCompare(b.addedAt);
      default:
        return a.name.localeCompare(b.name);
    }
  });
  return dir === "desc" ? sorted.reverse() : sorted;
}

export default function AuthorsPage() {
  const queryClient = useQueryClient();
  const {
    authorsView,
    setAuthorsView,
    authorsSort,
    authorsSortDir,
    setAuthorsSort,
  } = useUIStore();
  const [filter, setFilter] = useState<FilterStatus>("all");

  const {
    data: authors,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["authors"],
    queryFn: listAuthors,
  });

  const toggleMonitored = useMutation({
    mutationFn: ({ id, monitored }: { id: number; monitored: boolean }) =>
      updateAuthor(id, { monitored }),
    onSuccess: (_data, { id, monitored }) => {
      queryClient.setQueryData<AuthorResponse[]>(["authors"], (old) =>
        old?.map((a) => (a.id === id ? { ...a, monitored } : a)),
      );
    },
    onError: (err: Error) => {
      if (err.message?.includes("OL linkage")) {
        toast.error("Cannot monitor — author not linked to OpenLibrary");
      } else {
        toast.error("Failed to update author");
      }
    },
  });

  const toggleMonitorNew = useMutation({
    mutationFn: ({
      id,
      monitorNewItems,
    }: {
      id: number;
      monitorNewItems: boolean;
    }) => updateAuthor(id, { monitorNewItems }),
    onSuccess: (_data, { id, monitorNewItems }) => {
      queryClient.setQueryData<AuthorResponse[]>(["authors"], (old) =>
        old?.map((a) => (a.id === id ? { ...a, monitorNewItems } : a)),
      );
    },
    onError: () => {
      toast.error("Failed to update author");
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: number) => deleteAuthor(id),
    onSuccess: (_data, id) => {
      queryClient.setQueryData<AuthorResponse[]>(["authors"], (old) =>
        old?.filter((a) => a.id !== id),
      );
      toast.success("Author deleted");
    },
    onError: () => toast.error("Failed to delete author"),
  });

  const filtered = useMemo(() => {
    if (!authors) return [];
    let result = authors;
    if (filter === "monitored") result = result.filter((a) => a.monitored);
    if (filter === "unmonitored") result = result.filter((a) => !a.monitored);
    return sortAuthors(result, authorsSort, authorsSortDir);
  }, [authors, filter, authorsSort, authorsSortDir]);

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error as Error} onRetry={refetch} />;

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-3">
          <Link
            to="/author/add"
            className="inline-flex items-center gap-1.5 rounded bg-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-brand-hover"
          >
            <Plus size={14} />
            Add New
          </Link>
        </div>

        <div className="flex items-center gap-2 sm:gap-3">
          {/* Sort */}
          <select
            value={`${authorsSort}:${authorsSortDir}`}
            onChange={(e) => {
              const [field = "name", dir = "asc"] = e.target.value.split(":");
              setAuthorsSort(field, dir as "asc" | "desc");
            }}
            className="rounded border border-border bg-zinc-800 px-2 py-1 text-sm text-zinc-200"
          >
            <option value="name:asc">Name A-Z</option>
            <option value="name:desc">Name Z-A</option>
            <option value="dateAdded:desc">Date Added (Newest)</option>
            <option value="dateAdded:asc">Date Added (Oldest)</option>
          </select>

          {/* Filter */}
          <select
            value={filter}
            onChange={(e) => setFilter(e.target.value as FilterStatus)}
            className="rounded border border-border bg-zinc-800 px-2 py-1 text-sm text-zinc-200"
          >
            <option value="all">All</option>
            <option value="monitored">Monitored</option>
            <option value="unmonitored">Unmonitored</option>
          </select>

          {/* View toggle */}
          <div className="flex rounded border border-border">
            <button
              onClick={() => setAuthorsView("table")}
              className={cn(
                "p-1.5",
                authorsView === "table"
                  ? "bg-brand text-white"
                  : "text-muted hover:text-zinc-200",
              )}
              title="Table"
            >
              <List size={16} />
            </button>
            <button
              onClick={() => setAuthorsView("poster")}
              className={cn(
                "p-1.5",
                authorsView === "poster"
                  ? "bg-brand text-white"
                  : "text-muted hover:text-zinc-200",
              )}
              title="Poster"
            >
              <LayoutGrid size={16} />
            </button>
            <button
              onClick={() => setAuthorsView("overview")}
              className={cn(
                "p-1.5",
                authorsView === "overview"
                  ? "bg-brand text-white"
                  : "text-muted hover:text-zinc-200",
              )}
              title="Overview"
            >
              <Rows3 size={16} />
            </button>
          </div>
        </div>
      </PageToolbar>

      <PageContent>
        {filtered.length === 0 ? (
          <EmptyState
            icon={<BookOpen size={32} />}
            title="No authors found"
            description={
              filter !== "all"
                ? "Try changing your filter."
                : "Add an author to get started."
            }
            action={
              <Link
                to="/author/add"
                className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover"
              >
                <Plus size={14} />
                Add Author
              </Link>
            }
          />
        ) : authorsView === "table" ? (
          <TableView
            authors={filtered}
            onToggleMonitored={(id, val) =>
              toggleMonitored.mutate({ id, monitored: val })
            }
            onToggleMonitorNew={(id, val) =>
              toggleMonitorNew.mutate({ id, monitorNewItems: val })
            }
            onDelete={(id) => deleteMutation.mutate(id)}
          />
        ) : authorsView === "poster" ? (
          <PosterView authors={filtered} />
        ) : (
          <OverviewView authors={filtered} />
        )}
      </PageContent>
    </>
  );
}

function TableView({
  authors,
  onToggleMonitored,
  onToggleMonitorNew,
  onDelete,
}: {
  authors: AuthorResponse[];
  onToggleMonitored: (id: number, val: boolean) => void;
  onToggleMonitorNew: (id: number, val: boolean) => void;
  onDelete: (id: number) => void;
}) {
  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr className="border-b border-border text-left text-xs font-medium uppercase text-muted">
            <th className="px-3 py-2">Name</th>
            <th className="w-20 px-3 py-2"><span className="flex items-center gap-1">Monitor <HelpTip text="Monitor indexers for new uploads of all content by author." /></span></th>
            <th className="hidden sm:table-cell w-24 px-3 py-2"><span className="flex items-center gap-1">Monitor New <HelpTip text="Monitor indexers for new content by author." /></span></th>
            <th className="hidden md:table-cell px-3 py-2">Added</th>
            <th className="w-10 px-3 py-2" />
          </tr>
        </thead>
        <tbody>
          {authors.map((author) => (
            <tr
              key={author.id}
              className="border-b border-border/50 hover:bg-surface-hover"
            >
              <td className="px-3 py-2">
                <Link
                  to={`/author/${author.id}`}
                  className="font-medium text-zinc-100 hover:text-brand"
                >
                  {author.name}
                </Link>
              </td>
              <td className="px-3 py-2">
                <button
                  onClick={() =>
                    onToggleMonitored(author.id, !author.monitored)
                  }
                  className={cn(
                    "h-4 w-4 rounded border",
                    author.monitored
                      ? "border-brand bg-brand"
                      : "border-zinc-600 bg-transparent",
                  )}
                  title={author.monitored ? "Monitored" : "Unmonitored"}
                >
                  {author.monitored && (
                    <svg
                      viewBox="0 0 16 16"
                      className="text-white"
                      fill="currentColor"
                    >
                      <path d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z" />
                    </svg>
                  )}
                </button>
              </td>
              <td className="hidden sm:table-cell px-3 py-2">
                <button
                  onClick={() =>
                    onToggleMonitorNew(author.id, !author.monitorNewItems)
                  }
                  className={cn(
                    "h-4 w-4 rounded border",
                    author.monitorNewItems
                      ? "border-brand bg-brand"
                      : "border-zinc-600 bg-transparent",
                  )}
                  title={
                    author.monitorNewItems
                      ? "Monitoring new items"
                      : "Not monitoring new items"
                  }
                >
                  {author.monitorNewItems && (
                    <svg
                      viewBox="0 0 16 16"
                      className="text-white"
                      fill="currentColor"
                    >
                      <path d="M13.78 4.22a.75.75 0 010 1.06l-7.25 7.25a.75.75 0 01-1.06 0L2.22 9.28a.75.75 0 011.06-1.06L6 10.94l6.72-6.72a.75.75 0 011.06 0z" />
                    </svg>
                  )}
                </button>
              </td>
              <td className="hidden md:table-cell px-3 py-2 text-muted">
                {formatRelativeDate(author.addedAt)}
              </td>
              <td className="px-3 py-2">
                <button
                  onClick={() => {
                    if (confirm(`Delete ${author.name}?`)) onDelete(author.id);
                  }}
                  className="rounded p-1 text-muted hover:text-red-400"
                  title="Delete author"
                >
                  <Trash2 size={14} />
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function PosterView({ authors }: { authors: AuthorResponse[] }) {
  return (
    <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6">
      {authors.map((author) => (
        <Link
          key={author.id}
          to={`/author/${author.id}`}
          className="group rounded-lg border border-border bg-surface p-4 text-center hover:border-brand"
        >
          <div className="mx-auto mb-3 flex h-16 w-16 items-center justify-center rounded-full bg-zinc-700 text-xl font-bold text-zinc-300">
            {author.name.charAt(0).toUpperCase()}
          </div>
          <p className="truncate text-sm font-medium text-zinc-100 group-hover:text-brand">
            {author.name}
          </p>
        </Link>
      ))}
    </div>
  );
}

function OverviewView({ authors }: { authors: AuthorResponse[] }) {
  return (
    <div className="space-y-3">
      {authors.map((author) => (
        <Link
          key={author.id}
          to={`/author/${author.id}`}
          className="flex items-center gap-4 rounded-lg border border-border bg-surface p-4 hover:border-brand"
        >
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full bg-zinc-700 text-sm font-bold text-zinc-300">
            {author.name.charAt(0).toUpperCase()}
          </div>
          <div className="min-w-0 flex-1">
            <p className="truncate font-medium text-zinc-100">{author.name}</p>
          </div>
          <span
            className={cn(
              "h-2.5 w-2.5 shrink-0 rounded-full",
              author.monitored ? "bg-green-500" : "bg-zinc-600",
            )}
            title={author.monitored ? "Monitored" : "Unmonitored"}
          />
        </Link>
      ))}
    </div>
  );
}
