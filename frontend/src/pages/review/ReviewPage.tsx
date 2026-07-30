import { useEffect, useState } from "react";
import { Link } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { AlertTriangle, Sparkles } from "lucide-react";
import {
  listIdentityReview,
  resolveIdentityReview,
  dismissIdentityReview,
  listIdentityConflicts,
  resolveIdentityConflict,
  dismissIdentityConflict,
  listAuthorLinkReview,
  getWork,
} from "@/api";
import { AuthorLinkReviewCard } from "./AuthorLinkReviewCard";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { EmptyState } from "@/components/Page/EmptyState";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { HelpTip } from "@/components/HelpTip";
import type {
  IdentityReviewPark,
  IdentityReviewCandidate,
  IdentityConflictSummary,
  ConflictResolutionAction,
} from "@/types/api";

// Friendly source name for a candidate's contributing providers; falls back
// to the raw snake_case value for anything not in the map.
const PROVIDER_LABELS: Record<string, string> = {
  hardcover: "Hardcover",
  open_library: "Open Library",
  goodreads: "Goodreads",
  audnexus: "Audnexus",
  llm: "AI cleanup",
  readarr: "Readarr import",
  google_books: "Google Books",
  audible: "Audible",
};

function sourceLabel(sources: string[]): string {
  if (sources.length === 0) return "Unknown source";
  return sources.map((s) => PROVIDER_LABELS[s] ?? s).join(", ");
}

const CONFLICT_ACTIONS: {
  action: ConflictResolutionAction;
  label: string;
  help: string;
}[] = [
  {
    action: "keep_existing",
    label: "Keep Existing",
    help: "Keep the identifier this book already has. The new one is ignored.",
  },
  {
    action: "accept_separate",
    label: "Treat as Separate",
    help: "These are different books. This book's identifier is not changed.",
  },
  {
    action: "replace_anchor",
    label: "Use New Match",
    help: "Replace this book's identifier with the new one that was found.",
  },
  {
    action: "merge",
    label: "Combine Both",
    help: "Adopt the new identifiers alongside what this book already has.",
  },
];

function ExistingWorkLabel({ workId }: { workId: number }) {
  const { data } = useQuery({
    queryKey: ["work", String(workId)],
    queryFn: () => getWork(workId),
  });
  return (
    <Link to={`/work/${workId}`} className="font-medium text-zinc-100 hover:underline">
      {data?.title ?? `Work #${workId}`}
    </Link>
  );
}

function CandidateRow({
  candidate,
  onChoose,
  disabled,
}: {
  candidate: IdentityReviewCandidate;
  onChoose: () => void;
  disabled: boolean;
}) {
  const pct = Math.round(candidate.titleJaccard * 100);
  return (
    <li className="flex items-center gap-3 rounded px-2 py-1.5 hover:bg-zinc-800/50">
      <span className="min-w-[3rem] rounded bg-zinc-800 px-2 py-0.5 text-center text-xs font-medium text-zinc-200">
        {pct}%
      </span>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm text-zinc-200">{candidate.title}</p>
        <p className="truncate text-xs text-muted">
          {candidate.authorName} — {sourceLabel(candidate.sources)}
        </p>
      </div>
      {candidate.existingWorkId != null && (
        <span className="inline-flex items-center gap-1">
          <Link
            to={`/work/${candidate.existingWorkId}`}
            className="text-xs text-amber-400 hover:underline"
          >
            already in library
          </Link>
          <HelpTip text="This candidate's identity is already used by another work in your library. Choosing it does not merge the two works." />
        </span>
      )}
      <button
        onClick={onChoose}
        disabled={disabled}
        className="rounded bg-brand px-3 py-1 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
      >
        Choose
      </button>
    </li>
  );
}

function ParkedWorkCard({ park }: { park: IdentityReviewPark }) {
  const queryClient = useQueryClient();

  const resolve = useMutation({
    mutationFn: (candidateId: string) =>
      resolveIdentityReview(park.workId, { candidateId }),
    onSuccess: () => {
      toast.success(`Matched "${park.title}"`);
      queryClient.invalidateQueries({ queryKey: ["identity-review"] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Could not apply that match"),
  });

  const dismiss = useMutation({
    mutationFn: () => dismissIdentityReview(park.workId),
    onSuccess: () => {
      toast.success(`Dismissed — "${park.title}" stands alone`);
      queryClient.invalidateQueries({ queryKey: ["identity-review"] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Could not dismiss"),
  });

  const isPending = resolve.isPending || dismiss.isPending;

  return (
    <div className="rounded-lg border border-amber-900/40 bg-zinc-900/60 px-4 py-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <Sparkles size={15} className="shrink-0 text-amber-400" />
          <Link
            to={`/work/${park.workId}`}
            className="font-medium text-zinc-100 hover:underline"
          >
            {park.title}
          </Link>
          <span className="text-sm text-muted">by {park.authorName}</span>
          <HelpTip text="We found possible matches for this book but weren't confident enough to pick automatically. Choose the right one below, or dismiss if none of them match." />
        </div>
        <button
          onClick={() => dismiss.mutate()}
          disabled={isPending}
          className="text-xs text-muted hover:text-red-400 disabled:opacity-50"
        >
          None of these — dismiss
        </button>
      </div>
      {park.candidates.length === 0 ? (
        <p className="mt-2 text-sm text-muted">
          No candidates were recorded for this pick.
        </p>
      ) : (
        <ul className="mt-2 flex flex-col gap-0.5">
          {park.candidates.map((c) => (
            <CandidateRow
              key={c.candidateId}
              candidate={c}
              disabled={isPending}
              onChoose={() => resolve.mutate(c.candidateId)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function ConflictCard({ conflict }: { conflict: IdentityConflictSummary }) {
  const queryClient = useQueryClient();

  const resolve = useMutation({
    mutationFn: (action: ConflictResolutionAction) =>
      resolveIdentityConflict(conflict.id, { action }),
    onSuccess: () => {
      toast.success("Conflict resolved");
      queryClient.invalidateQueries({ queryKey: ["identity-conflicts"] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
    },
    onError: () => toast.error("Could not resolve the conflict"),
  });

  const dismiss = useMutation({
    mutationFn: () => dismissIdentityConflict(conflict.id),
    onSuccess: () => {
      toast.success("Conflict dismissed");
      queryClient.invalidateQueries({ queryKey: ["identity-conflicts"] });
    },
    onError: () => toast.error("Could not dismiss the conflict"),
  });

  const isPending = resolve.isPending || dismiss.isPending;

  return (
    <div className="rounded-lg border border-red-900/40 bg-zinc-900/60 px-4 py-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <AlertTriangle size={15} className="shrink-0 text-red-400" />
          <ExistingWorkLabel workId={conflict.existingWorkId} />
          <HelpTip text="Two different sources disagree about what this book's identifier should be. Choose how to resolve it." />
        </div>
        <button
          onClick={() => dismiss.mutate()}
          disabled={isPending}
          className="text-xs text-muted hover:text-red-400 disabled:opacity-50"
        >
          Dismiss
        </button>
      </div>
      <p className="mt-1 text-sm text-muted">
        New match found: <span className="text-zinc-200">{conflict.incomingTitle}</span>{" "}
        by {conflict.incomingAuthor}
      </p>
      <div className="mt-2 flex flex-wrap gap-3">
        {CONFLICT_ACTIONS.map(({ action, label, help }) => (
          <span key={action} className="inline-flex items-center gap-1">
            <button
              onClick={() => resolve.mutate(action)}
              disabled={isPending}
              className="rounded bg-zinc-800 px-3 py-1 text-xs font-medium text-zinc-200 hover:bg-zinc-700 disabled:opacity-50"
            >
              {label}
            </button>
            <HelpTip text={help} />
          </span>
        ))}
      </div>
    </div>
  );
}

/**
 * The books half of the page: the two work-identity queries, exactly as
 * before. Its loading and error states are its own, so a books outage leaves
 * the authors section on screen and usable.
 */
function BookReviewSections({ onEmpty }: { onEmpty: (empty: boolean) => void }) {
  const {
    data: parks,
    isLoading: parksLoading,
    error: parksError,
    refetch: refetchParks,
  } = useQuery({
    queryKey: ["identity-review"],
    queryFn: listIdentityReview,
  });

  const {
    data: conflicts,
    isLoading: conflictsLoading,
    error: conflictsError,
    refetch: refetchConflicts,
  } = useQuery({
    queryKey: ["identity-conflicts"],
    queryFn: listIdentityConflicts,
  });

  const parkList = parks ?? [];
  const conflictList = conflicts ?? [];
  const settled = !parksLoading && !conflictsLoading;
  const failed = parksError != null || conflictsError != null;

  useEffect(() => {
    onEmpty(settled && !failed && parkList.length === 0 && conflictList.length === 0);
  }, [onEmpty, settled, failed, parkList.length, conflictList.length]);

  if (parksLoading || conflictsLoading) return <PageLoading />;
  if (parksError) {
    return <ErrorState error={parksError} onRetry={() => refetchParks()} />;
  }
  if (conflictsError) {
    return (
      <ErrorState error={conflictsError} onRetry={() => refetchConflicts()} />
    );
  }

  return (
    <>
      {parkList.length > 0 && (
        <section>
          <h2 className="mb-2 text-sm font-semibold text-zinc-100">
            Needs Your Pick ({parkList.length})
          </h2>
          <div className="flex flex-col gap-3">
            {parkList.map((p) => (
              <ParkedWorkCard key={p.workId} park={p} />
            ))}
          </div>
        </section>
      )}
      {conflictList.length > 0 && (
        <section>
          <h2 className="mb-2 text-sm font-semibold text-zinc-100">
            Conflicting Matches ({conflictList.length})
          </h2>
          <div className="flex flex-col gap-3">
            {conflictList.map((c) => (
              <ConflictCard key={c.id} conflict={c} />
            ))}
          </div>
        </section>
      )}
    </>
  );
}

/**
 * The authors half: its own query, its own retry. Authors park for a different
 * reason than books do, and one list being unreachable says nothing about the
 * other, so neither waits on nor hides the other.
 */
function AuthorReviewSection({
  onEmpty,
}: {
  onEmpty: (empty: boolean) => void;
}) {
  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["author-link-review"],
    queryFn: listAuthorLinkReview,
  });

  const reviews = data ?? [];
  const empty = !isLoading && error == null && reviews.length === 0;

  useEffect(() => {
    onEmpty(empty);
  }, [onEmpty, empty]);

  if (isLoading) return <PageLoading />;
  if (error) {
    return (
      <section>
        <h2 className="mb-2 text-sm font-semibold text-zinc-100">Authors</h2>
        <ErrorState error={error} onRetry={() => refetch()} />
      </section>
    );
  }
  if (reviews.length === 0) return null;

  return (
    <section>
      <h2 className="mb-2 text-sm font-semibold text-zinc-100">
        Authors ({reviews.length})
      </h2>
      <div className="flex flex-col gap-3">
        {reviews.map((r) => (
          <AuthorLinkReviewCard key={r.author.id} review={r} />
        ))}
      </div>
    </section>
  );
}

export default function ReviewPage() {
  // Only when BOTH halves have loaded and found nothing is the page truly
  // empty; either half still working or failing is not an all-clear.
  const [booksEmpty, setBooksEmpty] = useState(false);
  const [authorsEmpty, setAuthorsEmpty] = useState(false);

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Needs Review</h1>
      </PageToolbar>
      <PageContent>
        {/* Both halves stay mounted: unmounting them to show the empty state
            would take their queries away, and nothing would bring the page
            back when new work arrives. Each renders nothing when it is empty. */}
        <div className="space-y-6">
          {booksEmpty && authorsEmpty && (
            <EmptyState
              title="Nothing needs review right now"
              description="Books and authors with uncertain or conflicting matches will show up here."
            />
          )}
          <BookReviewSections onEmpty={setBooksEmpty} />
          <AuthorReviewSection onEmpty={setAuthorsEmpty} />
        </div>
      </PageContent>
    </>
  );
}
