import { RefreshCw } from "lucide-react";
import { cn } from "@/utils/cn";
import { formatRelativeDate, formatDuration } from "@/utils/format";
import type { WorkDetailResponse, EnrichmentStatus, IdentityStatus } from "@/types/api";
import { StatusBadge, type BadgeTone } from "./StatusBadge";
import { MetadataRow } from "./MetadataRow";

// Identity state machine (REQ-014) — "which book is this?". The section header
// supplies the context a bare floating badge lacks.
const IDENTITY_BADGE: Record<IdentityStatus, { tone: BadgeTone; label: string; tip: string }> = {
  pending: { tone: "amber", label: "Pending", tip: "Still matching — only a fuzzy title/author guess so far." },
  confirmed: { tone: "green", label: "Confirmed", tip: "Locked to a master catalog record." },
  provisional: { tone: "blue", label: "Provisional", tip: "Identified by ISBN (barcode); no master record yet — may later upgrade to Confirmed." },
  conflict: { tone: "red", label: "Conflict", tip: "Sources disagree on the match; needs your review." },
  needs_review: { tone: "orange", label: "Needs Review", tip: "Couldn't match this book automatically; needs your review." },
  not_found: { tone: "amber", label: "Unverified", tip: "No source could confirm this match — every provider was rejected. Refresh to retry, or delete and re-add to pick a different match." },
};

// Enrichment (details) state machine — "what do we know about it?". The canonical
// outcomes are Pending/Enriched/Sparse (+ Failed). Identity outcomes (incl. the
// "unverified" not-found case) live on the Identity badge above, not here.
const DETAILS_BADGE: Record<EnrichmentStatus, { tone: BadgeTone; label: string; tip: string }> = {
  unenriched: { tone: "amber", label: "Pending", tip: "Details haven't been fetched yet." },
  enriched: { tone: "green", label: "Enriched", tip: "Real information is present. A cover is a separate lazy asset and isn't required." },
  thin: { tone: "zinc", label: "Sparse", tip: "Known book, but providers returned almost nothing — a settled result, not still loading." },
  failed: { tone: "red", label: "Failed", tip: "A lookup error occurred while fetching details. Try refreshing." },
};

export function BookInformationTab({
  work,
  onRefresh,
  refreshing,
}: {
  work: WorkDetailResponse;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const identity = IDENTITY_BADGE[work.identityStatus];
  const details = DETAILS_BADGE[work.enrichmentStatus];
  const missing = <span className="text-zinc-600">—</span>;

  return (
    <div className="max-w-2xl">
      {/* Identity — which book this is */}
      <section>
        <div className="flex items-center justify-between gap-4">
          <h3 className="text-sm font-medium text-zinc-100">Identity</h3>
          <button
            onClick={onRefresh}
            disabled={refreshing}
            className="btn-secondary inline-flex items-center gap-1.5 text-xs"
          >
            <RefreshCw size={12} className={cn(refreshing && "animate-spin")} />
            Refresh
          </button>
        </div>
        <p className="mt-0.5 mb-3 text-xs text-muted">Which book this is — catalog match &amp; identifiers.</p>
        <StatusBadge tone={identity.tone} label={identity.label} tip={identity.tip} />
        <dl className="mt-4">
          <MetadataRow label="Open Library" value={work.olKey || missing} />
          <MetadataRow label="Hardcover" value={work.hcKey || missing} />
          <MetadataRow label="Goodreads" value={work.grKey || missing} />
          <MetadataRow label="ISBN-13" value={work.isbn13 || missing} />
          <MetadataRow label="ASIN" value={work.asin || missing} />
        </dl>
      </section>

      {/* Details — what we know about it */}
      <section className="mt-7 pt-6 border-t border-border">
        <h3 className="text-sm font-medium text-zinc-100">Details</h3>
        <p className="mt-0.5 mb-3 text-xs text-muted">What we know about it — series, genres, publisher, cover.</p>
        <StatusBadge tone={details.tone} label={details.label} tip={details.tip} />
        <dl className="mt-4">
          {work.originalTitle && <MetadataRow label="Original title" value={work.originalTitle} />}
          <MetadataRow label="Year" value={work.year} />
          {(work.seriesName || work.enriching) && (
            <MetadataRow
              label="Series"
              value={
                work.seriesName
                  ? `${work.seriesName}${work.seriesPosition != null ? ` #${work.seriesPosition}` : ""}`
                  : null
              }
              skeleton={work.enriching}
            />
          )}
          {(work.genres && work.genres.length > 0) || work.enriching ? (
            <MetadataRow
              label="Genres"
              value={work.genres && work.genres.length > 0 ? work.genres.join(", ") : null}
              skeleton={work.enriching}
            />
          ) : null}
          <MetadataRow label="Publisher" value={work.publisher} skeleton={work.enriching} />
          <MetadataRow label="Publish date" value={work.publishDate} />
          <MetadataRow label="Language" value={work.language?.toUpperCase()} skeleton={work.enriching} />
          <MetadataRow label="Pages" value={work.pageCount} />
          {work.durationSeconds && (
            <MetadataRow label="Duration" value={formatDuration(work.durationSeconds)} />
          )}
          {(work.narrator && work.narrator.length > 0) || work.enriching ? (
            <MetadataRow
              label="Narrator"
              value={work.narrator && work.narrator.length > 0 ? work.narrator.join(", ") : null}
              skeleton={work.enriching}
            />
          ) : null}
          {work.narrationType && <MetadataRow label="Narration" value={work.narrationType} />}
          {work.abridged && <MetadataRow label="Abridged" value="Yes" />}
          {work.rating != null && (
            <MetadataRow label="Rating" value={
              `${work.rating.toFixed(1)}/5${work.ratingCount != null ? ` (${work.ratingCount} ratings)` : ""}`
            } />
          )}
          <MetadataRow label="Source" value={work.enrichmentSource} />
          {work.enrichedAt && (
            <MetadataRow label="Last enriched" value={formatRelativeDate(work.enrichedAt)} />
          )}
        </dl>
        <p className="mt-3 text-xs text-muted">
          The cover is fetched separately and may appear after this — it never affects the Enriched status.
        </p>
      </section>
    </div>
  );
}
