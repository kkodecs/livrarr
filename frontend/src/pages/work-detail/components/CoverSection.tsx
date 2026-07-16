import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ImagePlus, ChevronDown, Loader2, RefreshCw, Upload } from "lucide-react";
import { cn } from "@/utils/cn";
import { BookCover } from "@/components/BookCover";
import {
  getCoverAlternatives,
  selectCover,
  uploadWorkCover,
  type CoverCandidate,
} from "@/api";
import type { WorkDetailResponse } from "@/types/api";
import { CoverGrid } from "./CoverGrid";

export function CoverSection({
  work,
  onCoverUploaded,
  onClose,
}: {
  work: WorkDetailResponse;
  onCoverUploaded?: () => void;
  onClose: () => void;
}) {
  const [showAlternatives, setShowAlternatives] = useState(false);
  const queryClient = useQueryClient();

  const altQuery = useQuery({
    queryKey: ["coverAlternatives", work.id],
    queryFn: () => getCoverAlternatives(work.id),
    enabled: showAlternatives,
    staleTime: 60_000,
  });

  const selectMutation = useMutation({
    mutationFn: (candidate: CoverCandidate) =>
      selectCover(work.id, candidate.candidateId, candidate.mediaType),
    onSuccess: () => {
      toast.success("Cover updated");
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["coverAlternatives", work.id] });
      onCoverUploaded?.();
      onClose();
    },
    onError: () => toast.error("Failed to select cover"),
  });

  const uploadMutation = useMutation({
    mutationFn: ({ file, mediaType }: { file: Blob; mediaType: string }) =>
      uploadWorkCover(work.id, file, mediaType),
    onSuccess: () => {
      toast.success("Cover uploaded");
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["coverAlternatives", work.id] });
      onCoverUploaded?.();
      onClose();
    },
    onError: (err) => {
      const msg = err instanceof Error ? err.message : "Failed to upload cover";
      toast.error(msg);
    },
  });

  const ebookCandidates = altQuery.data?.filter((c) => c.mediaType === "ebook") ?? [];
  const audioCandidates = altQuery.data?.filter((c) => c.mediaType === "audiobook") ?? [];

  return (
    <div>
      <span className="mb-1 block text-sm font-medium text-zinc-300">Cover</span>

      {/* Current covers */}
      <div className="flex gap-4 mb-2">
        <div className="flex items-start gap-2">
          <div className="h-20 w-14 flex-shrink-0">
            <BookCover
              workId={work.id}
              coverVersion={work.coverMtime ?? undefined}
              className="h-full w-full rounded"
            />
          </div>
          <div className="text-xs text-zinc-400 space-y-0.5">
            <p className="text-[10px] text-zinc-500 uppercase tracking-wider">Ebook</p>
            <p>Trust: <span className="text-zinc-200">{prettyCoverTrust(work.coverTrust)}</span></p>
            {work.coverSource && (
              <p>Source: <span className="text-zinc-200">{prettyCoverSource(work.coverSource)}</span></p>
            )}
            {(work.coverWidth > 0 || work.coverHeight > 0) && (
              <p>Size: <span className="text-zinc-200">{work.coverWidth}&times;{work.coverHeight}</span></p>
            )}
            {work.coverTrust === "user" && (
              <p className="text-amber-500 text-[10px]">Locked</p>
            )}
          </div>
        </div>
        <div className="flex items-start gap-2">
          <div className="h-20 w-20 flex-shrink-0">
            <BookCover
              workId={work.id}
              coverVersion={work.audiobookCoverMtime ?? work.coverMtime ?? undefined}
              mediaType="audiobook"
              className="h-full w-full rounded"
            />
          </div>
          <div className="text-xs text-zinc-400 space-y-0.5">
            <p className="text-[10px] text-zinc-500 uppercase tracking-wider">Audiobook</p>
            <p>Trust: <span className="text-zinc-200">{prettyCoverTrust(work.audiobookCoverTrust)}</span></p>
            {work.audiobookCoverSource && (
              <p>Source: <span className="text-zinc-200">{prettyCoverSource(work.audiobookCoverSource)}</span></p>
            )}
            {(work.audiobookCoverWidth > 0 || work.audiobookCoverHeight > 0) && (
              <p>Size: <span className="text-zinc-200">{work.audiobookCoverWidth}&times;{work.audiobookCoverHeight}</span></p>
            )}
            {work.audiobookCoverTrust === "user" && (
              <p className="text-amber-500 text-[10px]">Locked</p>
            )}
            {!work.audiobookCoverUrl && (
              <p className="text-zinc-600 text-[10px]">Falls back to ebook</p>
            )}
          </div>
        </div>
      </div>

      {/* Browse alternatives toggle */}
      <button
        type="button"
        onClick={() => setShowAlternatives(!showAlternatives)}
        className="flex items-center gap-1.5 text-xs text-brand hover:text-brand/80 mb-2"
      >
        <ImagePlus size={14} />
        {showAlternatives ? "Hide alternatives" : "Browse alternatives"}
        <ChevronDown
          size={12}
          className={cn("transition-transform", showAlternatives && "rotate-180")}
        />
      </button>

      {/* Alternatives grid */}
      {showAlternatives && (
        <div className="space-y-3 mb-3">
          {altQuery.isLoading && (
            <div className="flex items-center gap-2 text-xs text-zinc-400 py-4">
              <Loader2 size={14} className="animate-spin" />
              Loading alternatives...
            </div>
          )}

          {altQuery.isError && (
            <p className="text-xs text-red-400">Failed to load alternatives</p>
          )}

          {altQuery.data && (
            <>
              <CoverGrid
                label="Ebook"
                candidates={ebookCandidates}
                selecting={selectMutation.isPending}
                onSelect={(c) => selectMutation.mutate(c)}
                onUpload={(file) => uploadMutation.mutate({ file, mediaType: "ebook" })}
                current={
                  work.coverMtime != null
                    ? { workId: work.id, coverVersion: work.coverMtime }
                    : undefined
                }
              />
              <CoverGrid
                label="Audiobook"
                candidates={audioCandidates}
                selecting={selectMutation.isPending}
                onSelect={(c) => selectMutation.mutate(c)}
                onUpload={(file) => uploadMutation.mutate({ file, mediaType: "audiobook" })}
                current={
                  work.audiobookCoverMtime != null
                    ? {
                        workId: work.id,
                        coverVersion: work.audiobookCoverMtime,
                        mediaType: "audiobook",
                      }
                    : undefined
                }
              />
              {ebookCandidates.length === 0 && audioCandidates.length === 0 && (
                <p className="text-xs text-zinc-500 py-2">No alternative covers found</p>
              )}
              <button
                type="button"
                onClick={() => altQuery.refetch()}
                disabled={altQuery.isFetching}
                className="flex items-center gap-1 text-[11px] text-zinc-400 hover:text-zinc-200"
              >
                <RefreshCw size={10} className={altQuery.isFetching ? "animate-spin" : ""} />
                Refresh
              </button>
              <p className="text-[10px] text-zinc-600">
                Selecting a cover locks it from automatic updates.
              </p>
            </>
          )}
        </div>
      )}

      {/* Upload fallback when alternatives not shown */}
      {!showAlternatives && (
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-1.5 cursor-pointer rounded bg-zinc-700 px-3 py-1.5 text-xs text-zinc-100 hover:bg-zinc-600">
            <Upload size={12} />
            Upload ebook cover
            <input
              type="file"
              accept="image/jpeg,image/png,image/webp"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) uploadMutation.mutate({ file, mediaType: "ebook" });
              }}
              className="hidden"
            />
          </label>
          <label className="flex items-center gap-1.5 cursor-pointer rounded bg-zinc-700 px-3 py-1.5 text-xs text-zinc-100 hover:bg-zinc-600">
            <Upload size={12} />
            Upload audiobook cover
            <input
              type="file"
              accept="image/jpeg,image/png,image/webp"
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) uploadMutation.mutate({ file, mediaType: "audiobook" });
              }}
              className="hidden"
            />
          </label>
          <span className="text-[10px] text-zinc-500">JPEG, PNG, or WebP (max 5MB)</span>
        </div>
      )}
    </div>
  );
}


const COVER_SOURCE_NAMES: Record<string, string> = {
  add: "the search result picked at add",
  import: "the file import match",
  user: "your upload",
  openlibrary: "OpenLibrary",
  goodreads: "Goodreads",
  google_books: "Google Books",
  hardcover: "Hardcover",
  other: "an enrichment provider",
};

function prettyCoverSource(source: string): string {
  return COVER_SOURCE_NAMES[source] ?? source;
}

function prettyCoverTrust(trust: string): string {
  switch (trust) {
    case "validated":
      return "Validated (matched identity)";
    case "user":
      return "Your choice (locked)";
    case "unvalidated":
      return "Unvalidated";
    default:
      return trust;
  }
}
