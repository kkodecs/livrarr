import { Link } from "react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ExternalLink, UserSearch } from "lucide-react";
import { dismissAuthorLinkCandidate, pickAuthorLinkCandidate } from "@/api";
import { HelpTip } from "@/components/HelpTip";
import {
  MONITORABLE_HELP,
  PROVIDER_LABELS,
  VERDICT_LABELS,
  candidateEvidenceText,
  candidateProvider,
  candidateRouteValue,
  catalogEvidenceText,
  invalidateAuthorLinkQueries,
  providerUrl,
} from "@/utils/authorLink";
import type { AuthorLinkCandidate, AuthorLinkReview } from "@/types/api";

/** One parked candidate: what it is, why we are unsure, and the two answers. */
function CandidateRow({
  candidate,
  authorId,
  authorName,
}: {
  candidate: AuthorLinkCandidate;
  authorId: number;
  authorName: string;
}) {
  const queryClient = useQueryClient();
  const provider = candidateProvider(candidate);
  const value = candidateRouteValue(candidate);
  const href = providerUrl(provider, value);
  const catalog = catalogEvidenceText(candidate);

  const pick = useMutation({
    mutationFn: () => pickAuthorLinkCandidate(candidate.id),
    onSuccess: () => {
      toast.success(`Linked ${authorName} to ${PROVIDER_LABELS[provider]}`);
      invalidateAuthorLinkQueries(queryClient, authorId);
    },
    onError: () => toast.error("Could not use that link"),
  });

  const dismiss = useMutation({
    mutationFn: () => dismissAuthorLinkCandidate(candidate.id),
    onSuccess: () => {
      toast.success("Dismissed — the author stays unlinked");
      invalidateAuthorLinkQueries(queryClient, authorId);
    },
    onError: () => toast.error("Could not dismiss that suggestion"),
  });

  const busy = pick.isPending || dismiss.isPending;

  return (
    <li className="rounded border border-border/60 px-3 py-2">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-center gap-2 text-sm text-zinc-200">
            <span className="font-medium">{candidate.candidate_name}</span>
            <span className="rounded bg-zinc-800 px-1.5 py-0.5 text-xs text-zinc-400">
              {PROVIDER_LABELS[provider]} {value}
            </span>
            {href && (
              <a
                href={href}
                target="_blank"
                rel="noopener noreferrer"
                className="text-muted hover:text-zinc-200"
                title={`Open on ${PROVIDER_LABELS[provider]}`}
              >
                <ExternalLink size={12} />
              </a>
            )}
            {candidate.previously_removed && (
              <span className="rounded bg-amber-900/30 px-1.5 py-0.5 text-xs text-amber-400">
                previously removed
              </span>
            )}
          </p>
          <p className="mt-1 text-xs text-muted">
            {candidateEvidenceText(candidate, authorName)}
          </p>
          <ul className="mt-1 space-y-0.5 text-xs text-muted">
            <li>
              Main name: {VERDICT_LABELS[candidate.primary_name_verdict]}
            </li>
            {candidate.alternate_name_evidence.map((alt) => (
              <li key={`${alt.name}-${alt.verdict}`}>
                Also known as "{alt.name}": {VERDICT_LABELS[alt.verdict]}
              </li>
            ))}
            {candidate.top_work_preview && (
              <li>Best known for "{candidate.top_work_preview}"</li>
            )}
            {catalog && <li>{catalog}</li>}
          </ul>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            onClick={() => pick.mutate()}
            disabled={busy}
            className="rounded bg-brand px-3 py-1 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            Use this
          </button>
          <button
            onClick={() => dismiss.mutate()}
            disabled={busy}
            className="text-xs text-muted hover:text-red-400 disabled:opacity-50"
          >
            Not this one
          </button>
        </div>
      </div>
    </li>
  );
}

/** One parked author and every candidate we have for them. */
export function AuthorLinkReviewCard({ review }: { review: AuthorLinkReview }) {
  const { author, candidates } = review;

  return (
    <div className="rounded-lg border border-sky-900/40 bg-zinc-900/60 px-4 py-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="flex items-center gap-2">
          <UserSearch size={15} className="shrink-0 text-sky-400" />
          <Link
            to={`/author/${author.id}`}
            className="font-medium text-zinc-100 hover:underline"
          >
            {author.name}
          </Link>
          <HelpTip text="We found possible provider pages for this author but the names did not match closely enough to link automatically. Pick the right one, or dismiss them all." />
        </div>
        <span className="inline-flex items-center gap-1 text-xs text-muted">
          {author.monitorable ? "Can be monitored" : "Cannot be monitored yet"}
          <HelpTip text={MONITORABLE_HELP} />
        </span>
      </div>
      {candidates.length === 0 ? (
        <p className="mt-2 text-sm text-muted">
          No suggestions were recorded for this author.
        </p>
      ) : (
        <ul className="mt-2 flex flex-col gap-2">
          {candidates.map((c) => (
            <CandidateRow
              key={c.id}
              candidate={c}
              authorId={author.id}
              authorName={author.name}
            />
          ))}
        </ul>
      )}
    </div>
  );
}
