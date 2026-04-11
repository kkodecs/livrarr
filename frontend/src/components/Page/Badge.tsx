import { cn } from "@/utils/cn";
import type { EnrichmentStatus, NarrationType, QueueStatus } from "@/types/api";

const enrichmentColors: Record<EnrichmentStatus, string> = {
  enriched: "bg-enrichment-enriched/20 text-enrichment-enriched",
  partial: "bg-enrichment-partial/20 text-enrichment-partial",
  pending: "bg-enrichment-pending/20 text-enrichment-pending",
  failed: "bg-enrichment-failed/20 text-enrichment-failed",
  exhausted: "bg-enrichment-exhausted/20 text-enrichment-exhausted",
  skipped: "bg-zinc-700/30 text-zinc-400",
};

export function EnrichmentBadge({ status }: { status: EnrichmentStatus }) {
  return (
    <span
      className={cn(
        "inline-flex rounded-full px-2 py-0.5 text-xs font-medium capitalize",
        enrichmentColors[status],
      )}
    >
      {status}
    </span>
  );
}

const narrationLabels: Record<NarrationType, string> = {
  human: "Human Narrated",
  ai: "AI Narrated",
  ai_authorized_replica: "AI Authorized Replica",
};

const narrationColors: Record<NarrationType, string> = {
  human: "bg-narration-human/20 text-narration-human",
  ai: "bg-narration-ai/20 text-narration-ai",
  ai_authorized_replica:
    "bg-narration-ai-authorized/20 text-narration-ai-authorized",
};

export function NarrationBadge({ type }: { type: NarrationType }) {
  return (
    <span
      className={cn(
        "inline-flex rounded-full px-2 py-0.5 text-xs font-medium",
        narrationColors[type],
      )}
    >
      {narrationLabels[type]}
    </span>
  );
}

const queueColors: Record<QueueStatus, string> = {
  downloading: "text-status-downloading",
  queued: "text-status-queued",
  paused: "text-status-paused",
  completed: "text-status-completed",
  warning: "text-status-warning",
  error: "text-status-error",
};

export function QueueStatusBadge({ status }: { status: QueueStatus }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 text-xs font-medium capitalize",
        queueColors[status],
      )}
    >
      <span
        className={cn(
          "h-2 w-2 rounded-full",
          status === "downloading" ? "animate-pulse bg-current" : "bg-current",
        )}
      />
      {status}
    </span>
  );
}

export function MediaTypeBadge({ type }: { type: "ebook" | "audiobook" }) {
  return (
    <span className="inline-flex rounded bg-zinc-700 px-1.5 py-0.5 text-xs text-zinc-300">
      {type === "ebook" ? "E" : "A"}
    </span>
  );
}
