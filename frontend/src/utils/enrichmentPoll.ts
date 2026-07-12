/**
 * D-006 poll decay for the work-detail enrichment poll: 1.5s for the first
 * 15s, then 5s, hard cap 60s of total polling. Pure function of elapsed time
 * since the current enrichment run was first observed; the caller supplies
 * that elapsed value (e.g. via TanStack Query's `refetchInterval`).
 *
 * Returns the next poll interval in ms, or `false` once the cap is reached
 * — the caller should stop polling at that point and let the enrichment
 * pill degrade to "attention" rather than trust a frozen `enriching: true`.
 */
export function nextEnrichmentPollIntervalMs(elapsedMs: number): number | false {
  if (elapsedMs >= 60_000) return false;
  if (elapsedMs < 15_000) return 1_500;
  return 5_000;
}
