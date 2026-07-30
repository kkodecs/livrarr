import { Link } from "react-router";
import { cn } from "@/utils/cn";
import { LINK_STATE_LABELS } from "@/utils/authorLink";
import type { AuthorLinkState } from "@/types/api";

const STATE_STYLES: Record<AuthorLinkState, string> = {
  linked: "bg-green-900/25 text-green-400",
  needs_review: "bg-amber-900/25 text-amber-400",
  unlinked: "bg-zinc-700/40 text-zinc-400",
};

const STATE_TITLES: Record<AuthorLinkState, string> = {
  linked: "This author is linked to at least one provider page.",
  needs_review: "We have suggestions for this author waiting for your answer.",
  unlinked: "This author is not linked to any provider page yet.",
};

const BADGE_BASE =
  "inline-flex rounded-full px-2 py-0.5 text-xs font-medium";

/**
 * The author's link state, and the way to fix it.
 *
 * An author with suggestions waiting goes to the review page; anything else
 * goes to the author's own routes panel, where re-resolving lives.
 */
export function AuthorLinkBadge({
  authorId,
  linkState,
  className,
}: {
  authorId: number;
  linkState: AuthorLinkState;
  className?: string;
}) {
  const to = linkState === "needs_review" ? "/review" : `/author/${authorId}`;
  return (
    <Link
      to={to}
      title={STATE_TITLES[linkState]}
      className={cn(
        BADGE_BASE,
        "hover:brightness-125",
        STATE_STYLES[linkState],
        className,
      )}
    >
      {LINK_STATE_LABELS[linkState]}
    </Link>
  );
}

/** The same badge without its own link, for rows that are already a link. */
export function AuthorLinkTag({
  linkState,
  className,
}: {
  linkState: AuthorLinkState;
  className?: string;
}) {
  return (
    <span
      title={STATE_TITLES[linkState]}
      className={cn(BADGE_BASE, STATE_STYLES[linkState], className)}
    >
      {LINK_STATE_LABELS[linkState]}
    </span>
  );
}
