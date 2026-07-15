import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Loader2 } from "lucide-react";
import { toast } from "sonner";
import { resolveGr, updateAuthor } from "@/api";
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

/** Author-resolution step of useSeriesPromote's flow: the author has no
 * Goodreads link yet, so there's no series list to resolve against. */
export function AuthorResolveModal({
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
