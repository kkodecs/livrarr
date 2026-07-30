import { Link } from "react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { reResolveAuthor } from "@/api";
import { FormModal } from "@/components/Page/FormModal";
import type { SeriesResponse } from "@/types/api";

/** Ambiguity step of useSeriesPromote's flow: multiple same-named series on
 * the author's Goodreads page — the user picks which one this row is. */
export function SeriesPickerModal({
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

/**
 * Author-resolution step of useSeriesPromote's flow: the author has no
 * Goodreads link yet, so there's no series list to resolve against.
 *
 * Picking a Goodreads author by name here is gone. A name match on its own
 * was never proof, and this door wrote the link with nothing behind it. The
 * author goes into the linking queue instead; anything uncertain arrives on
 * the review page with its evidence, for the user to approve.
 */
export function AuthorResolveModal({
  authorId,
  authorName,
  onCancel,
}: {
  authorId: number;
  authorName: string;
  onCancel: () => void;
}) {
  const queryClient = useQueryClient();

  const lookAgain = useMutation({
    mutationFn: () => reResolveAuthor(authorId),
    onSuccess: () => {
      toast.success("Queued — we'll look this author up in the background");
      queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
      queryClient.invalidateQueries({ queryKey: ["author-link-sweep"] });
      onCancel();
    },
    onError: () => toast.error("Could not queue this author"),
  });

  return (
    <FormModal
      open
      onOpenChange={(o) => !o && onCancel()}
      title="Link Author to Goodreads"
    >
      <p className="mb-3 text-sm text-muted">
        <span className="text-zinc-200">{authorName}</span> has no Goodreads
        link yet, so there is no series list to work from.
      </p>
      <p className="mb-4 text-xs text-muted">
        Links come from books of theirs we have already matched, or from a
        suggestion you approve on the review page. Queue them for another look,
        or check the review page for suggestions already waiting.
      </p>
      <div className="flex flex-wrap justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
        >
          Cancel
        </button>
        <Link
          to="/review"
          onClick={onCancel}
          className="rounded border border-border px-4 py-2 text-sm text-zinc-200 hover:bg-surface-hover"
        >
          Review suggestions
        </Link>
        <button
          type="button"
          disabled={lookAgain.isPending}
          onClick={() => lookAgain.mutate()}
          className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
        >
          {lookAgain.isPending && <Loader2 size={14} className="animate-spin" />}
          Look again
        </button>
      </div>
    </FormModal>
  );
}
