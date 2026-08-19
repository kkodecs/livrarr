import { RefreshCw } from "lucide-react";
import { Link } from "react-router";
import { BookCover } from "@/components/BookCover";
import { HelpTip } from "@/components/HelpTip";
import { cn } from "@/utils/cn";
import { formatRelativeDate, formatDuration } from "@/utils/format";
import type {
  CoverSourceLabel,
  CoverSlotUiState,
  WorkDetailResponse,
  WorkCoverUiState,
  EnrichmentStatus,
  IdentitySiblingPresentation,
  IdentityStatus,
} from "@/types/api";
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
  onMergeWorks?: (owningWorkId: number) => void;
}) {
  const identity = IDENTITY_BADGE[work.identityStatus];
  const details = DETAILS_BADGE[work.enrichmentStatus];
  const missing = <span className="text-zinc-600">—</span>;
  const identityValue = (value: string | null) => value || missing;

  return (
    <div className="max-w-2xl">
      {/* Identity — which book this is */}
      <section>
        <div className="flex items-center justify-between gap-4">
          <h3 className="text-sm font-medium text-zinc-100">Identity</h3>
          <div className="flex items-center gap-2">
            <button
              onClick={onRefresh}
              disabled={refreshing}
              className="btn-secondary inline-flex items-center gap-1.5 text-xs"
            >
              <RefreshCw size={12} className={cn(refreshing && "animate-spin")} />
              Refresh
            </button>
          </div>
        </div>
        <p className="mt-0.5 mb-3 text-xs text-muted">Which book this is — catalog match &amp; identifiers.</p>
        <StatusBadge tone={identity.tone} label={identity.label} tip={identity.tip} />
        {work.parkedByConflicts && (
          <p className="mt-2 text-xs text-amber-300">
            Re-matching is paused until the open identity conflict is reviewed.
          </p>
        )}
        <IdentitySiblingPanel siblings={work.identitySiblings} />
        <dl className="mt-4">
          <MetadataRow label="Open Library" value={identityValue(work.olKey)} />
          <MetadataRow label="Hardcover" value={identityValue(work.hcKey)} />
          <MetadataRow label="Goodreads" value={identityValue(work.grKey)} />
          <MetadataRow label="ASIN" value={identityValue(work.asin)} />
        </dl>
      </section>

      {/* Details — what we know about it */}
      <section className="mt-7 pt-6 border-t border-border">
        <h3 className="text-sm font-medium text-zinc-100">Details</h3>
        <p className="mt-0.5 mb-3 text-xs text-muted">What we know about it — series, genres, publisher, cover.</p>
        <StatusBadge tone={details.tone} label={details.label} tip={details.tip} />
        <WorkCoverState work={work} state={work.coverUiState} />
        <dl className="mt-4">
          <MetadataRow label="ISBN-13" value={identityValue(work.isbn13)} />
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

function IdentitySiblingPanel({
  siblings,
}: {
  siblings: IdentitySiblingPresentation[];
}) {
  return (
    <div
      data-testid="identity-sibling-panel"
      className="mt-4 rounded-lg border border-border bg-zinc-900/40 p-3"
    >
      <div className="flex items-center gap-1.5">
        <h4 className="text-xs font-medium text-zinc-200">Other books by this author</h4>
        <HelpTip text="This list is informational. Open a book to change that book on its own page." />
      </div>
      <p className="mt-1 text-xs text-muted">
        Confirming this book's identity affects only this book. Other books by this author stay exactly as they are.
      </p>
      {siblings.length > 0 ? (
        <ul className="mt-2 divide-y divide-border/60">
          {siblings.map((sibling) => (
            <li key={sibling.workId}>
              <Link
                data-sibling-affordance
                to={`/work/${sibling.workId}?tab=metadata`}
                className="block py-2 text-xs transition-colors hover:text-zinc-100"
              >
                <span className="block font-medium text-zinc-200">{sibling.title}</span>
                <span className="text-muted">
                  {[sibling.authorName, sibling.edition, sibling.route]
                    .filter(Boolean)
                    .join(" · ")}
                </span>
              </Link>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-2 text-xs text-zinc-600">No related library books to show.</p>
      )}
    </div>
  );
}

function WorkCoverState({
  work,
  state,
}: {
  work: WorkDetailResponse;
  state: WorkCoverUiState;
}) {
  const sourceLabels: Record<CoverSourceLabel, string> = {
    Provider: "Provider",
    "Your file": "Your file",
    Yours: "Yours",
  };
  const renderSlot = (label: string, slot: CoverSlotUiState, audiobook: boolean) => {
    let content;
    if (slot.state === "Selected") {
      content = (
        <div className="mt-2 flex items-center gap-3">
          <BookCover
            workId={work.id}
            title={work.title}
            authorName={work.authorName}
            mediaType={audiobook ? "audiobook" : undefined}
            coverVersion={
              audiobook
                ? (work.audiobookCoverMtime ?? work.coverMtime ?? undefined)
                : (work.coverMtime ?? undefined)
            }
            className={audiobook ? "h-16 w-16" : "h-16 w-11"}
          />
          <p className="text-xs text-zinc-300">
            Source: <span className="font-medium">{sourceLabels[slot.source]}</span>
          </p>
        </div>
      );
    } else if (slot.state === "Searching") {
      content = <p className="mt-2 text-xs text-blue-300">Searching for a cover</p>;
    } else if (slot.state === "NoCoverFound") {
      content = <p className="mt-2 text-xs text-amber-300">No cover found</p>;
    } else if (slot.state === "NowhereToLook") {
      content = <p className="mt-2 text-xs text-muted">Nowhere to look</p>;
    }

    return (
      <div data-cover-slot={audiobook ? "audiobook" : "ebook"} data-cover-state={slot.state}>
        <p className="text-[10px] uppercase tracking-wide text-zinc-500">{label}</p>
        {content}
      </div>
    );
  };

  return (
    <div data-testid="work-cover-state" className="mt-4 rounded-lg border border-border p-3">
      <div className="flex items-center gap-1.5">
        <h4 className="text-xs font-medium text-zinc-200">Covers</h4>
        <HelpTip text="Cover sources describe where the image came from. They do not grade the book's identity." />
      </div>
      {state.formatNeeded && (
        <div
          data-cover-panel="FormatNeeded"
          className="mt-3 rounded-md border border-amber-800/50 bg-amber-950/20 p-3"
        >
          <p className="text-xs font-medium text-amber-200">Cover found — format needed</p>
          <p className="mt-1 text-xs text-muted">
            Choose the edition format before these covers can fill a format slot.
          </p>
          <ul className="mt-2 flex flex-wrap gap-2">
            {state.formatNeeded.candidates.map((candidate) => (
              <li
                key={candidate.id}
                className="rounded border border-border bg-zinc-900 px-2 py-1 text-xs text-zinc-300"
              >
                Source: {sourceLabels[candidate.source]}
              </li>
            ))}
          </ul>
        </div>
      )}
      <div className="mt-3 grid gap-3 sm:grid-cols-2">
        {renderSlot("Ebook", state.ebook, false)}
        {renderSlot("Audiobook", state.audiobook, true)}
      </div>
    </div>
  );
}
