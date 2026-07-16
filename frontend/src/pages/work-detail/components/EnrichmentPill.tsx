import { useState, useRef, useEffect } from "react";
import { Loader2, AlertTriangle, Check } from "lucide-react";
import { cn } from "@/utils/cn";
import { deriveEnrichmentPillState } from "@/utils/enrichmentPill";
import type { EnrichmentStatus } from "@/types/api";
import { BADGE_TONE } from "./StatusBadge";

// Enrichment progress pill (spec REQ-005, design D-006) — header-only,
// enrichment-only signal. Never replaces or hides the Identity/Details
// StatusBadges above; those keep rendering exactly as today (AC-010).
export function EnrichmentPill({
  enriching,
  enrichmentStatus,
  onRefresh,
  refreshing,
}: {
  enriching: boolean;
  enrichmentStatus: EnrichmentStatus;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const pillState = deriveEnrichmentPillState(enriching, enrichmentStatus);
  const prevStateRef = useRef(pillState);
  const [showCompleteFlash, setShowCompleteFlash] = useState(false);

  // "complete" is shown only as a brief transition flash right after the
  // page itself observes fetching -> complete — never as a permanent badge
  // on every already-enriched book.
  useEffect(() => {
    if (prevStateRef.current === "fetching" && pillState === "complete") {
      setShowCompleteFlash(true);
      prevStateRef.current = pillState;
      const timer = setTimeout(() => setShowCompleteFlash(false), 4_000);
      return () => clearTimeout(timer);
    }
    prevStateRef.current = pillState;
  }, [pillState]);

  if (pillState === "fetching") {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium",
          BADGE_TONE.zinc.wrap,
        )}
      >
        <Loader2 size={12} className="animate-spin" />
        Fetching details…
      </span>
    );
  }

  if (pillState === "attention") {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center gap-2 rounded-full border px-3 py-1 text-xs font-medium",
          BADGE_TONE.amber.wrap,
        )}
      >
        <AlertTriangle size={12} />
        Couldn’t fetch everything
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          className="rounded bg-zinc-700/70 px-2 py-0.5 text-[11px] font-medium text-zinc-100 hover:bg-zinc-600/70 disabled:opacity-50"
        >
          Retry
        </button>
      </span>
    );
  }

  if (showCompleteFlash) {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center gap-1.5 rounded-full border px-3 py-1 text-xs font-medium",
          BADGE_TONE.green.wrap,
        )}
      >
        <Check size={12} />
        Details complete
      </span>
    );
  }

  return null;
}
