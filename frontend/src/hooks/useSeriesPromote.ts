import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { monitorSeries, promoteSeries } from "@/api";
import type { PromoteSeriesResponse, SeriesResponse } from "@/types/api";

export type PromoteFlags = { monitorEbook: boolean; monitorAudiobook: boolean };

export type PromoteFlow =
  | { step: "picker"; candidates: SeriesResponse[]; flags: PromoteFlags }
  | { step: "author"; authorId: number; flags: PromoteFlags }
  | null;

/**
 * One authority for "start monitoring a series" — every series list (the
 * Series page, the Author page, the author-picker "Add Series" modal) goes
 * through this. Handles every shape a row can be in:
 *  - seriesId == null: no DB row yet (fresh Goodreads entry) — creates one.
 *  - seriesId set, unresolved (stub): resolves the real Goodreads id first
 *    (silently, or via the picker/author-link flow on ambiguity), then monitors.
 *  - seriesId set, already resolved: monitors directly.
 */
export function useSeriesPromote(params: {
  authorId: number;
  seriesId: number | null;
  language: string;
  onMonitoring: () => void;
}) {
  const { authorId, seriesId, language, onMonitoring } = params;
  const [flow, setFlow] = useState<PromoteFlow>(null);

  const mutation = useMutation({
    mutationFn: async (input: {
      grKey?: string;
      flags: PromoteFlags;
    }): Promise<PromoteSeriesResponse> => {
      if (seriesId != null) {
        return promoteSeries(seriesId, {
          grKey: input.grKey ?? null,
          monitorEbook: input.flags.monitorEbook,
          monitorAudiobook: input.flags.monitorAudiobook,
          language,
        });
      }
      // No DB row yet, so there's nothing to promote — this is a plain
      // create+monitor, and the caller must supply the row's real grKey.
      const series = await monitorSeries(authorId, {
        grKey: input.grKey ?? "",
        monitorEbook: input.flags.monitorEbook,
        monitorAudiobook: input.flags.monitorAudiobook,
        language,
      });
      return { status: "monitoring", authorId, series };
    },
    onSuccess: (resp, input) => {
      if (resp.status === "monitoring") {
        setFlow(null);
        toast.success("Series monitoring started");
        onMonitoring();
      } else if (resp.status === "needsPicker") {
        setFlow({ step: "picker", candidates: resp.candidates ?? [], flags: input.flags });
      } else if (resp.status === "needsAuthorResolution") {
        setFlow({ step: "author", authorId: resp.authorId, flags: input.flags });
      }
    },
    onError: () => {
      toast.error("Failed to start monitoring");
    },
  });

  return {
    promote: (input: { grKey?: string; flags: PromoteFlags }) => mutation.mutate(input),
    isPending: mutation.isPending,
    flow,
    cancelFlow: () => setFlow(null),
  };
}
