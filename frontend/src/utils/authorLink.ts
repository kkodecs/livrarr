import type { QueryClient } from "@tanstack/react-query";
import { ApiError } from "@/api/client";
import type {
  AuthorCandidateCatalogState,
  AuthorLinkCandidate,
  AuthorLinkCandidateReason,
  AuthorLinkState,
  AuthorNameSource,
  AuthorProvider,
  AuthorRouteProvenance,
  AuthorVerdict,
} from "@/types/api";

export const PROVIDER_LABELS: Record<AuthorProvider, string> = {
  open_library: "Open Library",
  goodreads: "Goodreads",
  hardcover: "Hardcover",
};

export const LINK_STATE_LABELS: Record<AuthorLinkState, string> = {
  linked: "Linked",
  needs_review: "Needs review",
  unlinked: "Unlinked",
};

export const PROVENANCE_LABELS: Record<AuthorRouteProvenance, string> = {
  legacy_unguarded: "From before linking existed",
  tier1_inherited: "Inherited from a matched book",
  readarr_guarded: "From a Readarr import",
  user_picked: "You picked it",
  merge_coalesced: "Kept through an author merge",
};

export const PROVENANCE_HELP: Record<AuthorRouteProvenance, string> = {
  legacy_unguarded:
    "This link was already on the author before Livrarr checked names. It still works, and it is re-checked when new evidence arrives.",
  tier1_inherited:
    "One of your books is matched to this provider, and the provider's name for this author matches yours.",
  readarr_guarded:
    "Came in with a Readarr import and passed the same name check as everything else.",
  user_picked: "You chose this link on the review page.",
  merge_coalesced: "Carried over when two author records were merged.",
};

export const NAME_SOURCE_LABELS: Record<AuthorNameSource, string> = {
  user: "You",
  goodreads: "Goodreads",
  hardcover: "Hardcover",
  google_books: "Google Books",
  open_library: "Open Library",
  readarr: "Readarr",
  import: "Imported file",
  legacy: "Existing record",
};

export const VERDICT_LABELS: Record<AuthorVerdict, string> = {
  Agree: "name matches",
  Grey: "name partly matches",
  Disagree: "name does not match",
  Abstain: "no name to compare",
};

export const CANDIDATE_REASON_LABELS: Record<AuthorLinkCandidateReason, string> =
  {
    tier2_name_search: "Found by searching Open Library for this name",
    name_guard_failed: "The provider's name for this author did not match",
    readarr_name_guard_failed:
      "Came from a Readarr import, but the names did not match",
    tombstoned: "You removed this link before",
    legacy_contradiction:
      "Disagrees with a link this author already has for the same provider",
    ownership_collision: "Another author already holds this link",
    invalid_legacy_route: "The stored value is not a usable provider id",
  };

/** The single place the monitorability rule is worded for the user. */
export const MONITORABLE_HELP =
  "Author monitoring reads Open Library's list of an author's books, so it needs an Open Library link. Goodreads or Hardcover links make an author linked, but not monitorable.";

/**
 * The server's own words for a rejected author change.
 *
 * The monitor gate is the server's call, not the UI's: a validation refusal
 * carries its real reason in `fieldErrors`, while the envelope message is only
 * "Validation failed". Showing the envelope hides the reason.
 */
export function authorGateMessage(err: unknown, fallback: string): string {
  if (err instanceof ApiError) {
    const first = err.fieldErrors?.[0];
    if (first?.message) return first.message;
    if (err.message && err.message !== "Validation failed") return err.message;
  }
  return fallback;
}

/**
 * Why this candidate is a question, in the most concrete words available.
 *
 * A credit read off one of the user's own books can say so by name — which
 * provider, which spelling, which book — and that is far more use than a
 * category. Everything else, including a question whose book has since been
 * deleted, falls back to the category label rather than inventing a book.
 */
export function candidateEvidenceText(
  candidate: AuthorLinkCandidate,
  authorName: string,
): string {
  if (candidate.reason === "name_guard_failed" && candidate.evidence_work_title) {
    const provider = PROVIDER_LABELS[candidateProvider(candidate)];
    return (
      `${provider} credits "${candidate.candidate_name}" as an author of ` +
      `"${candidate.evidence_work_title}". It doesn't match any name you have ` +
      `for ${authorName}.`
    );
  }
  return CANDIDATE_REASON_LABELS[candidate.reason];
}

/** Only Tier-2 name-search candidates ever get a catalogue read. */
function expectsCatalogRead(candidate: AuthorLinkCandidate): boolean {
  return candidate.reason === "tier2_name_search";
}

/**
 * How much of this candidate's catalogue we actually looked at, in words.
 *
 * A read that failed or has not observed anything yet is never rendered as a
 * count of zero — "we found none" and "we could not look" are different
 * answers, and only the first is a finding. `null` means say nothing at all.
 */
export function catalogEvidenceText(
  candidate: AuthorLinkCandidate,
): string | null {
  const n = candidate.corroborated_title_count;
  const state: AuthorCandidateCatalogState = candidate.catalog_evidence_state;
  switch (state) {
    case "complete":
      return n > 0
        ? `${n} of your books by this author are in their catalogue`
        : "None of your books by this author are in their catalogue";
    case "partial":
    case "retrying":
      return n > 0
        ? `At least ${n} of your books are in their catalogue — still reading`
        : "Still reading their catalogue — no count yet";
    case "unavailable":
      return "Their catalogue could not be read — no count available";
    case "pending":
      // A candidate that never gets a catalogue read must not be presented as
      // one that has a read scheduled.
      return expectsCatalogRead(candidate)
        ? "Their catalogue has not been read yet"
        : null;
  }
}

/** The route this candidate would write, as a user-readable id. */
export function candidateRouteValue(candidate: AuthorLinkCandidate): string {
  const key = candidate.key;
  if ("open_library" in key) return key.open_library;
  if ("goodreads" in key) return String(key.goodreads);
  return String(key.hardcover);
}

export function candidateProvider(
  candidate: AuthorLinkCandidate,
): AuthorProvider {
  const key = candidate.key;
  if ("open_library" in key) return "open_library";
  if ("goodreads" in key) return "goodreads";
  return "hardcover";
}

/** The provider's own page for a route, where one exists to link out to. */
export function providerUrl(
  provider: AuthorProvider,
  value: string,
): string | null {
  switch (provider) {
    case "open_library":
      return `https://openlibrary.org/authors/${value}`;
    case "goodreads":
      return `https://www.goodreads.com/author/show/${value}`;
    // Hardcover author ids are internal numbers with no public page.
    case "hardcover":
      return null;
  }
}

/**
 * Everything an author-route change can move, invalidated in one place.
 *
 * Picking, removing, re-resolving, renaming, choosing a display name and
 * merging all change the same downstream answers — what the author is linked
 * to, whether it can be monitored, what its books are called, and how much of
 * the sweep is left — so they all invalidate the same set.
 */
export function invalidateAuthorLinkQueries(
  queryClient: QueryClient,
  authorId?: number,
): void {
  // Author identity and the lists that show it.
  queryClient.invalidateQueries({ queryKey: ["authors"] });
  queryClient.invalidateQueries({ queryKey: ["author-link-review"] });
  queryClient.invalidateQueries({ queryKey: ["author-link-sweep"] });
  if (authorId != null) {
    // The detail page keys its author by the string route param.
    queryClient.invalidateQueries({ queryKey: ["author", String(authorId)] });
    // Monitor and discovery surfaces that read this author's routes.
    queryClient.invalidateQueries({ queryKey: ["series", authorId] });
    queryClient.invalidateQueries({ queryKey: ["bibliography", authorId] });
  } else {
    queryClient.invalidateQueries({ queryKey: ["author"] });
    queryClient.invalidateQueries({ queryKey: ["series"] });
    queryClient.invalidateQueries({ queryKey: ["bibliography"] });
  }
  queryClient.invalidateQueries({ queryKey: ["series-all"] });
  // A rename cascades the displayed author string onto every work.
  queryClient.invalidateQueries({ queryKey: ["works"] });
}
