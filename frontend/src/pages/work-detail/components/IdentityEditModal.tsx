import { useState } from "react";
import { useMutation, useQueryClient, type QueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { AlertTriangle, Check, GitMerge, Loader2, Search, X } from "lucide-react";
import {
  clearIdentitySlot,
  commitIdentityEdit,
  getWork,
  previewIdentityEdit,
} from "@/api";
import { ApiError } from "@/api/client";
import type {
  IdentityPreviewResponse,
  IdentitySlot,
  SiblingAssessment,
} from "@/types/api";

const SLOT_LABELS: Record<IdentitySlot, string> = {
  gr_work: "Goodreads",
  ol_work: "Open Library",
  hc_work: "Hardcover",
  isbn_13: "ISBN-13",
  asin: "ASIN",
};

export function slotLabel(slot: IdentitySlot): string {
  return SLOT_LABELS[slot] ?? slot;
}

/**
 * Invalidate every mounted query an identity commit or clear affects, each in
 * its actual key type: work detail + pending anchors are keyed by the STRING
 * route param, the library list by ["works"], History by the NUMERIC work id.
 */
export function invalidateIdentityQueries(queryClient: QueryClient, workId: number): void {
  queryClient.invalidateQueries({ queryKey: ["work", String(workId)] });
  queryClient.invalidateQueries({ queryKey: ["works"] });
  queryClient.invalidateQueries({ queryKey: ["work", String(workId), "pending-anchors"] });
  queryClient.invalidateQueries({ queryKey: ["history", workId] });
}

/**
 * Bounded post-save poll: the save response already carries the updated rows;
 * this only catches the refresh the server spawned. At most 6 probes at 1.5s,
 * stopping early when `enriching` turns true (the page's own poll machinery
 * takes over from the cached data) or the identity status blocks enrichment
 * (conflict / needs review — nothing to wait for).
 */
export function startBoundedPostSavePoll(queryClient: QueryClient, workId: number): void {
  let probes = 0;
  const timer = setInterval(() => {
    probes += 1;
    void getWork(workId)
      .then((work) => {
        queryClient.setQueryData(["work", String(workId)], work);
        const blocking =
          work.identityStatus === "conflict" || work.identityStatus === "needs_review";
        if (work.enriching || blocking || probes >= 6) {
          clearInterval(timer);
        }
      })
      .catch(() => clearInterval(timer));
    if (probes >= 6) {
      clearInterval(timer);
    }
  }, 1500);
}

type ModalPhase = "input" | "previewing" | "previewed" | "confirming";

/**
 * Preview-confirm identity edit modal (design identity-edit r4 §Frontend).
 * `slot === null` is the Fix-match road (any pasted identifier/URL); a slot
 * makes the input slot-scoped (row pencil).
 */
export function IdentityEditModal({
  workId,
  slot,
  onClose,
  onMergeWorks,
}: {
  workId: number;
  slot: IdentitySlot | null;
  onClose: () => void;
  onMergeWorks?: (owningWorkId: number) => void;
}) {
  const queryClient = useQueryClient();
  const [input, setInput] = useState("");
  const [phase, setPhase] = useState<ModalPhase>("input");
  const [preview, setPreview] = useState<IdentityPreviewResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const previewMutation = useMutation({
    mutationFn: () => previewIdentityEdit(workId, { input: input.trim(), slot }),
    onMutate: () => {
      setPhase("previewing");
      setError(null);
    },
    onSuccess: (res) => {
      setPreview(res);
      setPhase("previewed");
    },
    onError: (e) => {
      setPhase("input");
      if (e instanceof ApiError && e.details?.code === "preview_capacity") {
        setError("The server is busy with other previews — try again in a moment.");
      } else {
        setError(e instanceof Error ? e.message : "Preview failed");
      }
    },
  });

  const commitMutation = useMutation({
    mutationFn: () => {
      const resolved = preview?.resolved;
      const previewId = preview?.previewId;
      if (!resolved || !previewId) {
        return Promise.reject(new Error("nothing certifiable to confirm"));
      }
      return commitIdentityEdit(workId, resolved.slot, previewId);
    },
    onMutate: () => setPhase("confirming"),
    onSuccess: (work) => {
      toast.success("Identifier updated");
      queryClient.setQueryData(["work", String(workId)], work);
      invalidateIdentityQueries(queryClient, workId);
      startBoundedPostSavePoll(queryClient, workId);
      onClose();
    },
    onError: (e) => {
      setPhase("previewed");
      if (e instanceof ApiError && e.details?.code === "preview_required") {
        // The snapshot went stale (identity changed underneath, or the token
        // expired/was used) — recovery is a fresh preview of the same input.
        setPreview(null);
        setPhase("input");
        setError("The book's identity changed while you were looking — preview again.");
        return;
      }
      if (e instanceof ApiError && e.details?.code === "anchor_collision") {
        setError(
          e.details.owningWorkTitle
            ? `That identifier already belongs to "${e.details.owningWorkTitle}".`
            : "That identifier already belongs to another book.",
        );
        return;
      }
      setError(e instanceof Error ? e.message : "Could not save the identifier");
    },
  });

  const resolved = preview?.resolved ?? null;
  const collision = preview?.collision ?? null;
  const certifiable = !!resolved && !!preview?.previewId && !collision;
  const busy = phase === "previewing" || phase === "confirming";
  const drops = (preview?.siblings ?? []).filter((s) => s.action === "drop");

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-4">
      <div className="w-full max-w-lg rounded-lg border border-border bg-zinc-900 p-5 shadow-xl">
        <div className="flex items-center justify-between">
          <h2 className="text-sm font-semibold text-zinc-100">
            {slot ? `Fix ${slotLabel(slot)} identifier` : "Fix match"}
          </h2>
          <button onClick={onClose} className="text-muted hover:text-zinc-200" aria-label="Close">
            <X size={16} />
          </button>
        </div>
        <p className="mt-1 text-xs text-muted">
          {slot
            ? `Paste the correct ${slotLabel(slot)} identifier or page URL.`
            : "Paste any identifier or provider URL — ISBN, Goodreads, Open Library, or Amazon."}
        </p>

        <div className="mt-3 flex gap-2">
          <input
            autoFocus
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && input.trim() && !busy) previewMutation.mutate();
            }}
            placeholder={slot === "gr_work" ? "e.g. 12345 or goodreads.com/book/show/…" : "Identifier or URL"}
            className="input flex-1 text-sm"
            disabled={busy}
          />
          <button
            onClick={() => previewMutation.mutate()}
            disabled={!input.trim() || busy}
            className="btn-primary inline-flex items-center gap-1.5 text-xs"
          >
            {phase === "previewing" ? <Loader2 size={12} className="animate-spin" /> : <Search size={12} />}
            Preview
          </button>
        </div>

        {error && (
          <p className="mt-2 flex items-start gap-1.5 text-xs text-red-400">
            <AlertTriangle size={13} className="mt-px shrink-0" /> {error}
          </p>
        )}

        {phase === "previewed" && preview && !resolved && (
          <div className="mt-4 rounded-md border border-amber-800/50 bg-amber-950/30 p-3 text-xs text-amber-200">
            {preview.reason === "not_found"
              ? "No book was found for that identifier — double-check the value."
              : "The provider couldn't be reached — nothing to certify. Try again shortly."}
          </div>
        )}

        {resolved && (
          <div className="mt-4 rounded-md border border-border bg-zinc-950/50 p-3">
            <p className="text-[11px] uppercase tracking-wide text-muted">This identifier is</p>
            <p className="mt-1 text-sm font-medium text-zinc-100">
              {resolved.title ?? "(untitled)"}
              {resolved.year != null && <span className="text-muted"> ({resolved.year})</span>}
            </p>
            <p className="text-xs text-muted">{resolved.author ?? "unknown author"}</p>
            <p className="mt-1 text-[11px] text-zinc-500">
              {slotLabel(resolved.slot)} · {resolved.canonicalValue}
            </p>
          </div>
        )}

        {collision && (
          <div className="mt-3 rounded-md border border-red-900/60 bg-red-950/30 p-3">
            <p className="text-xs text-red-200">
              This identifier already belongs to{" "}
              <span className="font-medium">“{collision.owningWorkTitle}”</span> in your library —
              you can merge the two books instead.
            </p>
            {onMergeWorks && (
              <button
                onClick={() => onMergeWorks(collision.owningWorkId)}
                className="btn-secondary mt-2 inline-flex items-center gap-1.5 text-xs"
              >
                <GitMerge size={12} /> Merge works
              </button>
            )}
          </div>
        )}

        {certifiable && (preview?.siblings.length ?? 0) > 0 && (
          <div className="mt-3">
            <p className="text-[11px] uppercase tracking-wide text-muted">Other identifiers</p>
            <div className="mt-1.5 flex flex-wrap gap-1.5">
              {preview!.siblings.map((s) => (
                <SiblingChip key={s.slot} assessment={s} />
              ))}
            </div>
            {drops.length > 0 && (
              <p className="mt-1.5 text-[11px] text-muted">
                Dropped identifiers aren't destroyed — they're cleared and re-matched
                against the corrected book.
              </p>
            )}
          </div>
        )}

        {certifiable &&
          preview!.bridgeWarnings.map((w) => (
            <p key={w.slot} className="mt-2 flex items-start gap-1.5 text-xs text-amber-300">
              <AlertTriangle size={13} className="mt-px shrink-0" /> {w.message}
            </p>
          ))}

        {preview?.conflictWarning && (
          <p className="mt-2 text-xs text-amber-300">
            This book has an open identity conflict — enrichment stays paused until the
            conflict is reviewed.
          </p>
        )}

        <div className="mt-5 flex justify-end gap-2">
          <button onClick={onClose} className="btn-secondary text-xs" disabled={phase === "confirming"}>
            Cancel
          </button>
          <button
            onClick={() => commitMutation.mutate()}
            disabled={!certifiable || busy}
            className="btn-primary inline-flex items-center gap-1.5 text-xs"
          >
            {phase === "confirming" ? (
              <Loader2 size={12} className="animate-spin" />
            ) : (
              <Check size={12} />
            )}
            This is the right book
          </button>
        </div>
      </div>
    </div>
  );
}

function SiblingChip({ assessment }: { assessment: SiblingAssessment }) {
  const keep = assessment.action === "keep";
  return (
    <span
      className={
        keep
          ? "inline-flex items-center gap-1 rounded-full border border-emerald-800/60 bg-emerald-950/40 px-2 py-0.5 text-[11px] text-emerald-200"
          : "inline-flex items-center gap-1 rounded-full border border-red-900/60 bg-red-950/40 px-2 py-0.5 text-[11px] text-red-200"
      }
      title={assessment.cause}
    >
      {keep ? <Check size={11} /> : <X size={11} />}
      {slotLabel(assessment.slot)}
      {!keep && assessment.cause ? ` · ${assessment.cause}` : ""}
    </span>
  );
}

/** Client-side confirm + DELETE for one identity slot (no preview needed). */
export function useClearIdentitySlot(workId: number) {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (slot: IdentitySlot) => clearIdentitySlot(workId, slot),
    onSuccess: (work) => {
      queryClient.setQueryData(["work", String(workId)], work);
      invalidateIdentityQueries(queryClient, workId);
      if (work.parkedByConflicts) {
        toast.info("Cleared — re-matching is paused until the open conflict is reviewed.");
      } else {
        toast.success("Identifier cleared — re-matching in the background.");
        startBoundedPostSavePoll(queryClient, workId);
      }
    },
    onError: (e) => {
      if (e instanceof ApiError && e.status === 404) {
        toast.info("Nothing to clear — that identifier is already empty.");
      } else {
        toast.error("Could not clear the identifier");
      }
    },
  });
}
