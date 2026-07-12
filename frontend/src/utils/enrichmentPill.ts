/**
 * Work-detail enrichment pill state, derived purely from the backend's
 * in-progress signal and settled enrichment status (spec REQ-005).
 */
export type EnrichmentPillState = "fetching" | "complete" | "attention";

/**
 * Rules (spec REQ-005):
 *   - fetching  iff `enriching` is true, regardless of `enrichmentStatus`.
 *   - complete  for a settled (non-enriching) work with status "enriched"
 *               or "thin" — Thin presents as complete, never as an error.
 *   - attention for a settled "failed" work, and for a settled "unenriched"
 *               work (needs recovery, not a spinner) — shown with Retry.
 *
 * `enrichmentStatus` is typed as `string` rather than the app's narrower
 * union so this function stays a small, dependency-free, exhaustively
 * testable unit; any unrecognized status is treated as `attention` rather
 * than silently masquerading as complete.
 */
export function deriveEnrichmentPillState(
  enriching: boolean,
  enrichmentStatus: string,
): EnrichmentPillState {
  if (enriching) return "fetching";
  if (enrichmentStatus === "enriched" || enrichmentStatus === "thin") {
    return "complete";
  }
  return "attention";
}
