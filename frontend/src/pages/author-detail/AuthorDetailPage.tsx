import { useState } from "react";
import { Link, useParams, useNavigate } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  ExternalLink,
  RefreshCw,
  Pencil,
  Trash2,
  BookOpen,
  Loader2,
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
} from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import { MediaTypeBadge } from "@/components/Page/Badge";
import { getCoverUrl, formatRelativeDate } from "@/utils/format";
import { cn } from "@/utils/cn";
import { HelpTip } from "@/components/HelpTip";
import type { AuthorDetailResponse } from "@/types/api";

export default function AuthorDetailPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const authorId = Number(id);

  const [editOpen, setEditOpen] = useState(false);
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
            onClick={() => setEditOpen(true)}
            className="inline-flex items-center gap-1.5 rounded border border-border px-3 py-1.5 text-sm text-zinc-200 hover:bg-surface-hover"
          >
            <Pencil size={14} />
            <span className="hidden sm:inline">Edit</span>
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
            <span className="font-medium text-zinc-200">{works.length}</span>{" "}
            {works.length === 1 ? "work" : "works"}
          </span>
          <span className="flex items-center gap-1.5">
            <span
              className={cn(
                "h-2 w-2 rounded-full",
                author.monitored ? "bg-green-500" : "bg-zinc-600",
              )}
            />
            {author.monitored ? "Monitor" : "Unmonitored"}
            <HelpTip text="Monitor indexers for new uploads of all content by author." />
          </span>
          <span className="flex items-center gap-1.5">
            <span
              className={cn(
                "h-2 w-2 rounded-full",
                author.monitorNewItems ? "bg-green-500" : "bg-zinc-600",
              )}
            />
            {author.monitorNewItems
              ? "Monitor new"
              : "Not monitoring new"}
            <HelpTip text="Monitor indexers for new content by author." />
          </span>
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
                <img
                  src={getCoverUrl(work.id)}
                  alt=""
                  className="h-12 w-8 sm:h-16 sm:w-11 shrink-0 rounded bg-zinc-700 object-cover"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = "none";
                  }}
                />
                <div className="min-w-0 flex-1">
                  <p className="truncate font-medium text-sm sm:text-base text-zinc-100">
                    {work.title}
                  </p>
                  <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                    {work.year && <span>{work.year}</span>}
                    {work.libraryItems.map((item) => (
                      <MediaTypeBadge key={item.id} type={item.mediaType} />
                    ))}
                  </div>
                </div>
                <span className="hidden sm:inline shrink-0 text-xs text-muted">
                  {work.libraryItems.length}{" "}
                  {work.libraryItems.length === 1 ? "file" : "files"}
                </span>
              </Link>
            ))}
          </div>
        )}
        {/* Bibliography */}
        <BibliographySection authorId={authorId} author={author} libraryOlKeys={new Set(works.map((w) => w.olKey).filter(Boolean) as string[])} />
      </PageContent>

      {/* Edit Modal */}
      <EditAuthorModal
        open={editOpen}
        onOpenChange={setEditOpen}
        author={author}
        onSaved={() => {
          queryClient.invalidateQueries({ queryKey: ["author", id] });
          queryClient.invalidateQueries({ queryKey: ["authors"] });
        }}
      />

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

  const { data: bib, isLoading } = useQuery({
    queryKey: ["bibliography", authorId],
    queryFn: () => getAuthorBibliography(authorId),
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

function EditAuthorModal({
  open,
  onOpenChange,
  author,
  onSaved,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  author: AuthorDetailResponse["author"];
  onSaved: () => void;
}) {
  const [monitored, setMonitored] = useState(author.monitored);
  const [monitorNewItems, setMonitorNewItems] = useState(
    author.monitorNewItems,
  );
  const [saving, setSaving] = useState(false);

  // Reset on open
  const handleOpenChange = (val: boolean) => {
    if (val) {
      setMonitored(author.monitored);
      setMonitorNewItems(author.monitorNewItems);
    }
    onOpenChange(val);
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await updateAuthor(author.id, { monitored, monitorNewItems });
      toast.success("Author updated");
      onSaved();
      onOpenChange(false);
    } catch {
      toast.error("Failed to update author");
    } finally {
      setSaving(false);
    }
  };

  return (
    <FormModal open={open} onOpenChange={handleOpenChange} title="Edit Author">
      <div className="space-y-4">
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={monitored}
              onChange={(e) => setMonitored(e.target.checked)}
              className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-brand"
            />
            <span className="text-sm text-zinc-200">Monitor</span>
          </label>
          <HelpTip text="Monitor indexers for new uploads of all content by author." />
        </div>
        <div className="flex items-center gap-3">
          <label className="flex items-center gap-3 cursor-pointer">
            <input
              type="checkbox"
              checked={monitorNewItems}
              onChange={(e) => setMonitorNewItems(e.target.checked)}
              className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-brand"
            />
            <span className="text-sm text-zinc-200">Monitor new</span>
          </label>
          <HelpTip text="Monitor indexers for new content by author." />
        </div>
        <div className="flex justify-end gap-3 pt-2">
          <button
            onClick={() => onOpenChange(false)}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
          <button
            onClick={handleSave}
            disabled={saving}
            className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {saving ? "Saving..." : "Save"}
          </button>
        </div>
      </div>
    </FormModal>
  );
}
