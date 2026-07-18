import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Sparkles } from "lucide-react";
import { toast } from "sonner";
import { affirmPendingAnchor, getPendingAnchors } from "@/api";
import { HelpTip } from "@/components/HelpTip";
import type { PendingAnchorDTO } from "@/types/api";

// Friendly source name for a pending anchor's type; falls back to the raw type.
const SOURCE_LABELS: Record<string, string> = {
  gr_work: "Goodreads",
  ol_work: "Open Library",
  hc_work: "Hardcover",
  isbn_13: "ISBN",
  asin: "Audible / Amazon",
};

function sourceLabel(anchorType: string): string {
  return SOURCE_LABELS[anchorType] ?? anchorType;
}

// Public page for a pending anchor's id, so a guess can be eyeballed before
// confirming. Hardcover is absent: its key is a numeric API id with no public
// URL, so that id renders as plain text.
const SOURCE_URLS: Record<string, (value: string) => string> = {
  gr_work: (v) => `https://www.goodreads.com/book/show/${encodeURIComponent(v)}`,
  ol_work: (v) => `https://openlibrary.org/works/${encodeURIComponent(v)}`,
  isbn_13: (v) => `https://isbnsearch.org/isbn/${encodeURIComponent(v)}`,
  asin: (v) => `https://www.amazon.com/dp/${encodeURIComponent(v)}`,
};

/**
 * Inline, non-blocking banner listing fuzzy-matched identifier guesses for a
 * work. Each guess can be confirmed with one click, which promotes it to a real
 * identifier and unlocks that source's enrichment. Renders nothing when the work
 * has no pending guesses; the work is fully usable while a guess is unconfirmed.
 */
export function PendingAnchorBanner({ workId }: { workId: number }) {
  const queryClient = useQueryClient();

  const { data: pending } = useQuery({
    queryKey: ["work", String(workId), "pending-anchors"],
    queryFn: () => getPendingAnchors(workId),
  });

  const affirm = useMutation({
    mutationFn: (anchorType: string) => affirmPendingAnchor(workId, anchorType),
    onSuccess: (_data, anchorType) => {
      toast.success(`Confirmed ${sourceLabel(anchorType)} match`);
      queryClient.invalidateQueries({
        queryKey: ["work", String(workId), "pending-anchors"],
      });
      queryClient.invalidateQueries({ queryKey: ["work", String(workId)] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Could not confirm the match"),
  });

  // hc_work is omitted: Hardcover's key is an internal numeric id with no
  // public page, so a guess for it cannot be checked before confirming.
  const visible = (pending ?? []).filter((p) => p.anchorType !== "hc_work");
  if (visible.length === 0) return null;

  return (
    <div className="mt-4 rounded-lg border border-border bg-zinc-900/60 px-4 py-3">
      <div className="flex items-center gap-2">
        <Sparkles size={15} className="text-amber-400" />
        <span className="text-sm font-medium text-zinc-100">
          Possible matches
        </span>
        <HelpTip text="We found these by matching the title and author. They're guesses, so we don't use them until you confirm." />
      </div>
      <p className="mb-2 mt-1 text-sm text-muted">
        Confirm one to use it for richer metadata and covers.
      </p>
      <ul className="flex flex-col gap-0.5">
        {visible.map((p: PendingAnchorDTO) => (
          <li
            key={`${p.anchorType}:${p.value}`}
            className="flex items-center gap-3 rounded px-2 py-1.5 hover:bg-zinc-800/50"
          >
            <span className="min-w-[7rem] rounded bg-zinc-800 px-2 py-0.5 text-center text-xs font-medium text-zinc-200">
              {sourceLabel(p.anchorType)}
            </span>
            {SOURCE_URLS[p.anchorType] ? (
              <a
                href={SOURCE_URLS[p.anchorType](p.value)}
                target="_blank"
                rel="noopener noreferrer"
                className="font-mono text-xs text-zinc-500 underline decoration-zinc-700 underline-offset-2 hover:text-zinc-300"
              >
                {p.value}
              </a>
            ) : (
              <span className="font-mono text-xs text-zinc-500">{p.value}</span>
            )}
            <span className="flex-1" />
            <button
              onClick={() => affirm.mutate(p.anchorType)}
              disabled={affirm.isPending}
              className="rounded bg-brand px-3 py-1 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
            >
              Confirm
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}
