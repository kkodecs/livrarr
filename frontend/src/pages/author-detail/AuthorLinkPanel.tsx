import { useState } from "react";
import { Link } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ExternalLink, Loader2, RefreshCw, Trash2 } from "lucide-react";
import {
  getAuthorLinkSweepProgress,
  listAuthors,
  mergeAuthors,
  removeAuthorRoute,
  renameAuthor,
  reResolveAuthor,
  selectAuthorName,
} from "@/api";
import { HelpTip } from "@/components/HelpTip";
import { AuthorLinkTag } from "@/components/AuthorLinkBadge";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import { cn } from "@/utils/cn";
import { formatRelativeDate } from "@/utils/format";
import {
  MONITORABLE_HELP,
  NAME_SOURCE_LABELS,
  PROVENANCE_HELP,
  PROVENANCE_LABELS,
  PROVIDER_LABELS,
  authorGateMessage,
  invalidateAuthorLinkQueries,
  providerUrl,
} from "@/utils/authorLink";
import type { AuthorResponse, AuthorRouteResponse } from "@/types/api";

/** A route row: what it points at, where it came from, and how to drop it. */
function RouteRow({
  route,
  onRemove,
  busy,
}: {
  route: AuthorRouteResponse;
  onRemove: (route: AuthorRouteResponse) => void;
  busy: boolean;
}) {
  const href = providerUrl(route.provider, route.value);
  const removed = route.state === "removed";

  return (
    <tr className={cn("border-b border-border/50", removed && "text-zinc-500")}>
      <td className="px-2 py-1.5 whitespace-nowrap">
        {PROVIDER_LABELS[route.provider]}
      </td>
      <td className="px-2 py-1.5">
        <span className="inline-flex items-center gap-1.5">
          <span className={cn("font-mono text-xs", !removed && "text-zinc-200")}>
            {route.value}
          </span>
          {href && !removed && (
            <a
              href={href}
              target="_blank"
              rel="noopener noreferrer"
              className="text-muted hover:text-zinc-200"
              title={`Open on ${PROVIDER_LABELS[route.provider]}`}
            >
              <ExternalLink size={12} />
            </a>
          )}
        </span>
      </td>
      <td className="px-2 py-1.5">
        <span className="inline-flex items-center gap-1 text-xs">
          {PROVENANCE_LABELS[route.provenance]}
          <HelpTip text={PROVENANCE_HELP[route.provenance]} />
        </span>
      </td>
      <td className="px-2 py-1.5 text-xs">
        {removed ? (
          <span className="inline-flex items-center gap-1">
            Removed{route.removedAt ? ` ${formatRelativeDate(route.removedAt)}` : ""}
            <HelpTip text="Nothing automatic will add this link back. Only you can, by picking it again on the review page." />
          </span>
        ) : (
          <span className="text-green-600">In use</span>
        )}
      </td>
      <td className="px-2 py-1.5 text-right">
        {!removed && (
          <button
            type="button"
            onClick={() => onRemove(route)}
            disabled={busy}
            className="rounded p-1 text-muted hover:text-red-400 disabled:opacity-50"
            title="Remove this link"
          >
            <Trash2 size={13} />
          </button>
        )}
      </td>
    </tr>
  );
}

/** Rename, and pick among the spellings we have already seen. */
function NamePanel({ author }: { author: AuthorResponse }) {
  const queryClient = useQueryClient();
  const [renameOpen, setRenameOpen] = useState(false);
  const [draft, setDraft] = useState(author.name);

  const rename = useMutation({
    mutationFn: (name: string) => renameAuthor(author.id, name),
    onSuccess: (updated) => {
      toast.success(`Renamed to "${updated.name}"`);
      setRenameOpen(false);
      invalidateAuthorLinkQueries(queryClient, author.id);
    },
    onError: (err: unknown) =>
      toast.error(authorGateMessage(err, "Could not rename this author")),
  });

  const selectVariant = useMutation({
    mutationFn: (variantId: number) => selectAuthorName(author.id, variantId),
    onSuccess: (updated) => {
      toast.success(`Now shown as "${updated.name}"`);
      invalidateAuthorLinkQueries(queryClient, author.id);
    },
    onError: (err: unknown) =>
      toast.error(authorGateMessage(err, "Could not change the displayed name")),
  });

  const busy = rename.isPending || selectVariant.isPending;

  return (
    <div className="mb-4">
      <div className="mb-1.5 flex items-center gap-2">
        <h3 className="text-xs font-semibold uppercase tracking-wider text-muted">
          Name
        </h3>
        <button
          type="button"
          onClick={() => {
            setDraft(author.name);
            setRenameOpen(true);
          }}
          className="text-xs text-brand hover:underline"
        >
          Rename
        </button>
        <HelpTip text="Renaming changes the author here and on every one of their books. It does not change how books are matched." />
      </div>
      {author.nameVariants.length === 0 ? (
        <p className="text-xs text-zinc-500">
          No other spellings of this name have been seen yet.
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-1.5">
          {author.nameVariants.map((variant) => (
            <button
              key={variant.id}
              type="button"
              disabled={busy || variant.selected}
              onClick={() => selectVariant.mutate(variant.id)}
              title={`Seen from: ${NAME_SOURCE_LABELS[variant.source]}`}
              className={cn(
                "rounded border px-2 py-0.5 text-xs disabled:opacity-60",
                variant.selected
                  ? "border-brand bg-brand/15 text-zinc-100"
                  : "border-border text-zinc-300 hover:bg-surface-hover",
              )}
            >
              {variant.name}
              <span className="ml-1.5 text-[0.65rem] text-zinc-500">
                {NAME_SOURCE_LABELS[variant.source]}
              </span>
            </button>
          ))}
        </div>
      )}

      <FormModal
        open={renameOpen}
        onOpenChange={setRenameOpen}
        title="Rename author"
      >
        <div className="space-y-3">
          <p className="text-sm text-muted">
            This name replaces the author's name on all of their books.
          </p>
          <input
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            aria-label="Author name"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100"
          />
          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setRenameOpen(false)}
              className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={rename.isPending || draft.trim().length === 0}
              onClick={() => rename.mutate(draft.trim())}
              className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
            >
              Save
            </button>
          </div>
        </div>
      </FormModal>
    </div>
  );
}

/** Fold another author of the user's into this one. */
function MergePanel({ author }: { author: AuthorResponse }) {
  const queryClient = useQueryClient();
  const [open, setOpen] = useState(false);
  const [loserId, setLoserId] = useState<number | null>(null);

  const { data: authors } = useQuery({
    queryKey: ["authors"],
    queryFn: listAuthors,
    enabled: open,
  });

  const others = (authors ?? []).filter((a) => a.id !== author.id);
  const loser = others.find((a) => a.id === loserId) ?? null;

  const merge = useMutation({
    mutationFn: (id: number) => mergeAuthors(author.id, id),
    onSuccess: (report) => {
      toast.success(
        `Merged — ${report.worksMoved} book${report.worksMoved === 1 ? "" : "s"} moved to ${author.name}`,
      );
      setOpen(false);
      setLoserId(null);
      invalidateAuthorLinkQueries(queryClient, author.id);
    },
    onError: (err: unknown) =>
      toast.error(authorGateMessage(err, "Could not merge those authors")),
  });

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="text-xs text-zinc-500 hover:text-zinc-300"
      >
        Merge another author into this one
      </button>
      <FormModal open={open} onOpenChange={setOpen} title="Merge authors">
        <div className="space-y-3">
          <p className="text-sm text-muted">
            Pick the duplicate. Its books, series and links move to{" "}
            <span className="text-zinc-200">{author.name}</span>, and the
            duplicate is deleted.
          </p>
          <select
            value={loserId ?? ""}
            onChange={(e) =>
              setLoserId(e.target.value === "" ? null : Number(e.target.value))
            }
            aria-label="Author to merge in"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100"
          >
            <option value="">Choose an author…</option>
            {others.map((a) => (
              <option key={a.id} value={a.id}>
                {a.name}
              </option>
            ))}
          </select>
          <div className="flex justify-end gap-2">
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
            >
              Cancel
            </button>
            <button
              type="button"
              disabled={loser == null || merge.isPending}
              onClick={() => loser && merge.mutate(loser.id)}
              className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
            >
              {merge.isPending ? "Merging…" : "Merge"}
            </button>
          </div>
        </div>
      </FormModal>
    </>
  );
}

/**
 * Everything about who this author IS: the provider links, where each came
 * from, the names we know, and the ways to change them.
 */
export function AuthorLinkPanel({ author }: { author: AuthorResponse }) {
  const queryClient = useQueryClient();
  const [pendingRemoval, setPendingRemoval] =
    useState<AuthorRouteResponse | null>(null);
  // A re-resolve is queued work, not an answer. This only says the request
  // landed; the persisted sweep state below is what actually reports progress.
  const [queued, setQueued] = useState(false);

  const activeRoutes = author.routes.filter((r) => r.state === "active");
  const removedRoutes = author.routes.filter((r) => r.state === "removed");

  const remove = useMutation({
    mutationFn: (route: AuthorRouteResponse) =>
      removeAuthorRoute(author.id, route.id),
    onSuccess: (_data, route) => {
      toast.success(`Removed the ${PROVIDER_LABELS[route.provider]} link`);
      setPendingRemoval(null);
      invalidateAuthorLinkQueries(queryClient, author.id);
    },
    onError: (err: unknown) =>
      toast.error(authorGateMessage(err, "Could not remove that link")),
  });

  const reResolve = useMutation({
    mutationFn: () => reResolveAuthor(author.id),
    // 202 only: the work is queued and unattended. Never hold this open
    // waiting for a provider.
    onSuccess: () => {
      setQueued(true);
      toast.success("Queued — we'll look this author up in the background");
      invalidateAuthorLinkQueries(queryClient, author.id);
    },
    onError: (err: unknown) =>
      toast.error(authorGateMessage(err, "Could not queue this author")),
  });

  // Persisted sweep state, so a page reload shows the real position rather
  // than a memory of having clicked the button.
  const { data: sweep } = useQuery({
    queryKey: ["author-link-sweep"],
    queryFn: getAuthorLinkSweepProgress,
    refetchInterval: (query) => {
      const p = query.state.data;
      return p && (p.queued > 0 || p.running > 0) ? 5000 : false;
    },
  });

  const sweepBusy = sweep != null && (sweep.queued > 0 || sweep.running > 0);

  return (
    <section className="mt-8">
      <div className="mb-3 flex flex-wrap items-center gap-3">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-muted">
          Provider Links
        </h2>
        <AuthorLinkTag linkState={author.linkState} />
        <span className="inline-flex items-center gap-1 text-xs text-muted">
          {author.monitorable ? "Can be monitored" : "Cannot be monitored"}
          <HelpTip text={MONITORABLE_HELP} />
        </span>
        {reResolve.isPending ? (
          <span className="flex items-center gap-1.5 text-xs text-zinc-500">
            <Loader2 size={10} className="animate-spin" /> Queueing…
          </span>
        ) : (
          <button
            type="button"
            onClick={() => reResolve.mutate()}
            title="Queue this author for another look"
            className="inline-flex items-center gap-1 text-xs text-brand hover:underline"
          >
            <RefreshCw size={10} /> Look again
          </button>
        )}
        {author.linkState === "needs_review" && (
          <Link to="/review" className="text-xs text-amber-400 hover:underline">
            Suggestions are waiting on the review page
          </Link>
        )}
      </div>

      {(queued || sweepBusy) && (
        <p className="mb-3 text-xs text-zinc-500">
          {sweep
            ? `Linking sweep: ${sweep.completed} of ${sweep.total} authors done, ${sweep.queued} waiting, ${sweep.running} in progress.`
            : "Queued — this runs in the background and can take a while."}
        </p>
      )}

      <NamePanel author={author} />

      {activeRoutes.length === 0 && removedRoutes.length === 0 ? (
        <p className="text-sm text-zinc-500">
          This author is not linked to any provider page yet. "Look again" puts
          them back in the queue; anything we are unsure about lands on the
          review page.
        </p>
      ) : (
        <div className="overflow-x-auto rounded border border-border">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border text-left text-xs font-medium uppercase text-muted">
                <th className="px-2 py-1.5">Provider</th>
                <th className="px-2 py-1.5">Id</th>
                <th className="px-2 py-1.5">Where it came from</th>
                <th className="px-2 py-1.5">State</th>
                <th className="w-8 px-2 py-1.5" />
              </tr>
            </thead>
            <tbody>
              {[...activeRoutes, ...removedRoutes].map((route) => (
                <RouteRow
                  key={route.id}
                  route={route}
                  busy={remove.isPending}
                  onRemove={setPendingRemoval}
                />
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div className="mt-3">
        <MergePanel author={author} />
      </div>

      <ConfirmModal
        open={pendingRemoval != null}
        onOpenChange={(open) => !open && setPendingRemoval(null)}
        title="Remove this link?"
        description={
          pendingRemoval
            ? `Nothing automatic will add the ${PROVIDER_LABELS[pendingRemoval.provider]} link ${pendingRemoval.value} back. You can pick it again yourself from the review page.${
                pendingRemoval.provider === "open_library"
                  ? " This author will stop being monitorable."
                  : ""
              }`
            : ""
        }
        confirmLabel="Remove"
        variant="danger"
        // Fire and let the mutation's own handlers report: awaiting here would
        // hand a failure to the modal's generic catch, which shows the error
        // envelope instead of the server's actual reason.
        onConfirm={() => {
          if (pendingRemoval) remove.mutate(pendingRemoval);
        }}
      />
    </section>
  );
}
