import { useState, useMemo } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { HelpTip } from "@/components/HelpTip";
import { FormModal } from "@/components/Page/FormModal";
import { listWorks, previewMergeWorks, mergeWorks } from "@/api";
import type { WorkDetailResponse, MergeFieldChoice } from "@/types/api";

export function MergeDialog({
  work,
  open,
  onOpenChange,
}: {
  work: WorkDetailResponse;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const queryClient = useQueryClient();
  const [loserId, setLoserId] = useState<number | null>(null);
  const [search, setSearch] = useState("");
  const [choices, setChoices] = useState<Record<string, MergeFieldChoice>>({});

  const { data: allWorks } = useQuery({
    queryKey: ["works", "merge-picker"],
    queryFn: () => listWorks({ pageSize: 1000 }),
    select: (res) => res.items,
    enabled: open && loserId === null,
  });

  const candidates = useMemo(() => {
    if (!allWorks) return [];
    const term = search.trim().toLowerCase();
    return allWorks
      .filter((w) => w.id !== work.id)
      .filter(
        (w) =>
          term === "" ||
          w.title.toLowerCase().includes(term) ||
          w.authorName.toLowerCase().includes(term),
      )
      .slice(0, 25);
  }, [allWorks, search, work.id]);

  const {
    data: preview,
    isLoading: previewLoading,
    isError: previewErrored,
  } = useQuery({
    queryKey: ["merge-preview", work.id, loserId],
    queryFn: () => previewMergeWorks(work.id, loserId as number),
    enabled: loserId !== null,
  });

  function close() {
    setLoserId(null);
    setSearch("");
    setChoices({});
    onOpenChange(false);
  }

  const mergeMutation = useMutation({
    mutationFn: () => {
      if (loserId === null || !preview) {
        return Promise.reject(new Error("no work selected"));
      }
      // Every conflict gets an explicit entry — defaulting to keep_survivor
      // when the user never touched that field's radios — so the backend
      // (which refuses a merge with any conflict left unresolved) always
      // receives a complete answer.
      return mergeWorks(work.id, loserId, {
        choices: preview.conflicts.map((c) => ({
          field: c.field,
          choice: choices[c.field] ?? "keep_survivor",
        })),
      });
    },
    onSuccess: (result) => {
      toast.success(
        result.warnings.length > 0
          ? `Merged, with ${result.warnings.length} file warning(s)`
          : "Works merged",
      );
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
      close();
    },
    onError: (err) => {
      toast.error(err instanceof Error ? err.message : "Merge failed");
    },
  });

  return (
    <FormModal
      open={open}
      onOpenChange={(next) => (next ? onOpenChange(true) : close())}
      title={loserId === null ? "Merge a Duplicate Work" : `Merge Into "${work.title}"`}
    >
      {loserId === null ? (
        <div>
          <p className="text-sm text-muted">
            Pick the duplicate to combine into this work. Its files move here
            and it is removed — no file is ever deleted.
          </p>
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Search by title or author..."
            className="mt-3 w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            autoFocus
          />
          <div className="mt-3 max-h-64 overflow-y-auto rounded border border-border">
            {candidates.length === 0 ? (
              <div className="p-3 text-sm text-muted">No matching works</div>
            ) : (
              candidates.map((w) => (
                <button
                  key={w.id}
                  type="button"
                  onClick={() => setLoserId(w.id)}
                  className="flex w-full items-center justify-between border-b border-border px-3 py-2 text-left text-sm last:border-b-0 hover:bg-zinc-700"
                >
                  <span className="text-zinc-100">{w.title}</span>
                  <span className="text-muted">{w.authorName}</span>
                </button>
              ))
            )}
          </div>
        </div>
      ) : previewLoading ? (
        <div className="text-sm text-muted">Loading preview...</div>
      ) : previewErrored || !preview ? (
        <div className="text-sm text-red-400">
          Could not load the merge preview.{" "}
          <button
            type="button"
            onClick={() => setLoserId(null)}
            className="text-brand hover:text-brand/80"
          >
            Pick a different work
          </button>
        </div>
      ) : (
        <div className="space-y-4">
          <div className="text-sm text-zinc-200">
            <div className="flex items-center gap-1">
              {preview.libraryItemsMoving} file(s) and {preview.grabsMoving} grab(s) move
              into this work.
              <HelpTip text="Bookmarks, reading progress, and .kash audio links stay attached to those files and are unaffected." />
            </div>
            <div className="mt-1 text-muted">
              The records merge first — the duplicate&apos;s files and grabs
              transfer to this work and the duplicate is removed. Files are
              then reorganized into place; any that can&apos;t be moved are
              reported as warnings. No file is ever deleted.
            </div>
          </div>

          {preview.conflicts.length > 0 && (
            <div className="space-y-3 rounded border border-amber-700/40 bg-amber-950/20 p-3">
              <div className="flex items-center gap-1 text-sm font-medium text-amber-400">
                Choose a value for each conflicting field
                <HelpTip text="Both works have a different value set for this field. Pick which one to keep — the other is shown, never silently discarded." />
              </div>
              {preview.conflicts.map((c) => (
                <div key={c.field} className="text-sm">
                  <div className="mb-1 text-zinc-300">
                    {c.field === "series_name" ? "Series name" : "Series position"}
                  </div>
                  <label className="flex items-center gap-2 py-0.5">
                    <input
                      type="radio"
                      name={`merge-choice-${c.field}`}
                      checked={(choices[c.field] ?? "keep_survivor") === "keep_survivor"}
                      onChange={() =>
                        setChoices((prev) => ({ ...prev, [c.field]: "keep_survivor" }))
                      }
                    />
                    <span className="text-zinc-200">Keep &quot;{c.survivorValue}&quot;</span>
                  </label>
                  <label className="flex items-center gap-2 py-0.5">
                    <input
                      type="radio"
                      name={`merge-choice-${c.field}`}
                      checked={choices[c.field] === "take_loser"}
                      onChange={() =>
                        setChoices((prev) => ({ ...prev, [c.field]: "take_loser" }))
                      }
                    />
                    <span className="text-zinc-200">Use &quot;{c.loserValue}&quot; instead</span>
                  </label>
                </div>
              ))}
            </div>
          )}

          <div className="flex items-center justify-between">
            <button
              type="button"
              onClick={() => setLoserId(null)}
              className="rounded px-3 py-2 text-sm text-muted hover:text-zinc-100"
            >
              Back
            </button>
            <div className="flex gap-3">
              <button
                type="button"
                onClick={close}
                className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
              >
                Cancel
              </button>
              <button
                type="button"
                onClick={() => mergeMutation.mutate()}
                disabled={mergeMutation.isPending}
                className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
              >
                {mergeMutation.isPending ? "Merging..." : "Merge"}
              </button>
            </div>
          </div>
        </div>
      )}
    </FormModal>
  );
}
