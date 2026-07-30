import type {
  AuthorLinkCandidate,
  AuthorNameVariantResponse,
  AuthorResponse,
  AuthorRouteResponse,
  AuthorSweepProgress,
} from "@/types/api";

/** Author payloads shaped exactly as the handlers serialize them. */

export function makeRoute(
  over: Partial<AuthorRouteResponse> = {},
): AuthorRouteResponse {
  return {
    id: 1,
    provider: "open_library",
    value: "OL111A",
    state: "active",
    provenance: "tier1_inherited",
    removedAt: null,
    ...over,
  };
}

export function makeVariant(
  over: Partial<AuthorNameVariantResponse> = {},
): AuthorNameVariantResponse {
  return {
    id: 1,
    name: "Ursula K. Le Guin",
    source: "open_library",
    selected: false,
    ...over,
  };
}

export function makeAuthor(over: Partial<AuthorResponse> = {}): AuthorResponse {
  return {
    id: 7,
    name: "Ursula K. Le Guin",
    sortName: "Le Guin, Ursula K.",
    olKey: null,
    grKey: null,
    hcKey: null,
    routes: [],
    nameVariants: [],
    linkState: "unlinked",
    monitorable: false,
    monitored: false,
    monitorNewItems: false,
    monitorLanguage: null,
    addedAt: "2026-07-01T00:00:00.000Z",
    ...over,
  };
}

export function makeCandidate(
  over: Partial<AuthorLinkCandidate> = {},
): AuthorLinkCandidate {
  return {
    id: 41,
    author_id: 7,
    key: { open_library: "OL111A" },
    candidate_name: "Ursula K. Le Guin",
    reason: "tier2_name_search",
    name_verdict: "Grey",
    primary_name_verdict: "Grey",
    alternate_name_evidence: [],
    top_work_preview: null,
    catalog_evidence_state: "complete",
    corroborated_title_count: 0,
    settled_work_count: 3,
    previously_removed: false,
    status: "pending",
    evidence_generation: 4,
    observed_at: "2026-07-29T10:00:00.000Z",
    ...over,
  };
}

export function makeSweep(
  over: Partial<AuthorSweepProgress> = {},
): AuthorSweepProgress {
  return {
    total: 0,
    completed: 0,
    queued: 0,
    running: 0,
    parked: 0,
    needs_review: 0,
    retryable_failures: 0,
    key_retryable: 0,
    key_skipped: 0,
    key_layout_drift: 0,
    would_have_linked_at_090: 0,
    oldest_due_at: null,
    ...over,
  };
}

/** A bibliography payload the author detail page can render without error. */
export const EMPTY_BIBLIOGRAPHY = {
  authorId: 7,
  entries: [],
  llmFiltered: false,
  rawAvailable: false,
  filteredCount: 0,
  rawCount: 0,
  fetchedAt: "2026-07-29T10:00:00.000Z",
};
