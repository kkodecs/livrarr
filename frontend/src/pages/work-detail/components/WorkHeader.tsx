import { Link } from "react-router";
import { Book, BookOpen, Headphones, ExternalLink } from "lucide-react";
import { BookCover } from "@/components/BookCover";
import { cn } from "@/utils/cn";
import { formatBytes } from "@/utils/format";
import type { WorkDetailResponse } from "@/types/api";
import { SkeletonBlock } from "./SkeletonBlock";
import { EnrichmentPill } from "./EnrichmentPill";

export function WorkHeader({
  work,
  activeGrabs,
  onToggleMonitor,
  onEditCover,
  pillEnriching,
  onRefresh,
  refreshing,
}: {
  work: WorkDetailResponse;
  activeGrabs: Set<string>;
  onToggleMonitor: (field: "monitorEbook" | "monitorAudiobook") => void;
  onEditCover: () => void;
  pillEnriching: boolean;
  onRefresh: () => void;
  refreshing: boolean;
}) {
  const ebookItems = work.libraryItems?.filter((li) => li.mediaType === "ebook") ?? [];
  const audioItems = work.libraryItems?.filter((li) => li.mediaType === "audiobook") ?? [];
  const ebookSize = ebookItems.reduce((acc, li) => acc + li.fileSize, 0);
  const audioSize = audioItems.reduce((acc, li) => acc + li.fileSize, 0);
  const ebookDownloading = activeGrabs.has(`${work.id}-ebook`);
  const audioDownloading = activeGrabs.has(`${work.id}-audiobook`);

  function monitorStatus(
    monitored: boolean,
    hasFile: boolean,
    fileSize: number,
    downloading: boolean,
  ): { color: string; label: string } {
    if (!monitored) return { color: "text-zinc-600", label: "Not Monitored" };
    if (hasFile) return { color: "text-green-400", label: formatBytes(fileSize) };
    if (downloading) return { color: "text-purple-400", label: "Downloading" };
    return { color: "text-amber-500", label: "Missing" };
  }

  const ebook = monitorStatus(work.monitorEbook, ebookItems.length > 0, ebookSize, ebookDownloading);
  const audio = monitorStatus(work.monitorAudiobook, audioItems.length > 0, audioSize, audioDownloading);

  const hasAudioFiles = audioItems.length > 0;
  const hasDedicatedAudioCover = !!work.audiobookCoverUrl;
  const showSeparateAudioCover = hasAudioFiles && hasDedicatedAudioCover;

  return (
    <div className="flex flex-col items-center gap-4 sm:flex-row sm:items-start sm:gap-6">
      <div className="flex gap-3 flex-shrink-0 order-first sm:order-last">
        <div className="flex flex-col items-center gap-1">
          <div className="relative group h-[200px] w-[133px] sm:h-[300px] sm:w-[200px]">
            <BookCover
              workId={work.id}
              title={work.title}
              authorName={work.authorName}
              coverVersion={work.coverMtime ?? undefined}
              className="h-full w-full rounded-lg shadow-lg"
              iconSize={32}
              variant="full"
            />
            {(ebookItems.length > 0 || (audioItems.length > 0 && !showSeparateAudioCover)) && (
              <div className="absolute inset-0 flex items-center justify-center gap-4 opacity-0 group-hover:opacity-100 transition-opacity bg-black/40 rounded-lg">
                {ebookItems[0] && (
                  <Link
                    to={`/read/${ebookItems[0].id}`}
                    className="rounded-full bg-black/60 p-3 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
                  >
                    <BookOpen size={24} />
                  </Link>
                )}
                {audioItems[0] && !showSeparateAudioCover && (
                  <Link
                    to={`/listen/${audioItems[0].id}?workId=${work.id}`}
                    className="rounded-full bg-black/60 p-3 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
                  >
                    <Headphones size={24} />
                  </Link>
                )}
              </div>
            )}
          </div>
          <span className="text-[10px] text-zinc-500">{hasAudioFiles && !hasDedicatedAudioCover ? "Ebook/Audiobook Cover" : "Ebook Cover"}</span>
          <button
            type="button"
            onClick={onEditCover}
            className="text-[10px] text-brand hover:text-brand/80"
          >
            Update
          </button>
        </div>
        {showSeparateAudioCover && (
          <div className="hidden sm:flex flex-col items-center gap-1">
            <div className="relative group h-[200px] w-[133px] sm:h-[300px] sm:w-[200px]">
              <BookCover
                workId={work.id}
                title={work.title}
                coverVersion={work.audiobookCoverMtime ?? work.coverMtime ?? undefined}
                mediaType="audiobook"
                className="h-full w-full rounded-lg shadow-lg"
                iconSize={32}
                variant="full"
              />
              {audioItems[0] && (
                <div className="absolute inset-0 flex items-center justify-center opacity-0 group-hover:opacity-100 transition-opacity bg-black/40 rounded-lg">
                  <Link
                    to={`/listen/${audioItems[0].id}?workId=${work.id}`}
                    className="rounded-full bg-black/60 p-3 text-zinc-200 hover:text-white hover:bg-brand/80 transition-colors"
                  >
                    <Headphones size={24} />
                  </Link>
                </div>
              )}
            </div>
            <span className="text-[10px] text-zinc-500">Audiobook Cover</span>
            <button
              type="button"
              onClick={onEditCover}
              className="text-[10px] text-brand hover:text-brand/80"
            >
              Update
            </button>
          </div>
        )}
      </div>
      <div className="min-w-0 flex-1 text-center sm:text-left">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-baseline gap-2">
            <h1 className="text-2xl font-bold text-zinc-100">{work.title}</h1>
            <span className="text-xs text-zinc-600">#{work.id}</span>
          </div>
          <EnrichmentPill
            enriching={pillEnriching}
            enrichmentStatus={work.enrichmentStatus}
            onRefresh={onRefresh}
            refreshing={refreshing}
          />
        </div>

        <div className="mt-2 flex flex-wrap items-center gap-2 text-sm">
          {work.authorId ? (
            <Link
              to={`/author/${work.authorId}`}
              className="text-brand hover:underline"
            >
              {work.authorName}
            </Link>
          ) : (
            <span className="text-muted">{work.authorName}</span>
          )}
          {work.year && <span className="text-muted">({work.year})</span>}
          {work.seriesName ? (
            <span className="text-muted">
              {work.seriesName}
              {work.seriesPosition != null && ` #${work.seriesPosition}`}
            </span>
          ) : work.enriching ? (
            <SkeletonBlock className="h-3.5 w-24" />
          ) : null}
        </div>

        <div className="mt-3 flex items-center gap-4">
          <button
            onClick={() => onToggleMonitor("monitorEbook")}
            className={cn("inline-flex items-center gap-1.5 text-sm transition-colors hover:opacity-80", ebook.color)}
            title={`Ebook: ${ebook.label}. Click to ${work.monitorEbook ? "stop" : "start"} monitoring.`}
          >
            <Book size={16} />
            <span>{ebook.label}</span>
          </button>
          <button
            onClick={() => onToggleMonitor("monitorAudiobook")}
            className={cn("inline-flex items-center gap-1.5 text-sm transition-colors hover:opacity-80", audio.color)}
            title={`Audiobook: ${audio.label}. Click to ${work.monitorAudiobook ? "stop" : "start"} monitoring.`}
          >
            <Headphones size={16} />
            <span>{audio.label}</span>
          </button>
        </div>

        {work.detailUrl && (
          <a
            href={work.detailUrl}
            target="_blank"
            rel="noopener noreferrer"
            className="mt-2 inline-flex items-center gap-1 text-sm text-brand hover:underline"
          >
            <ExternalLink size={14} />
            View on Goodreads
          </a>
        )}

        {work.genres && work.genres.length > 0 ? (
          <div className="mt-3 flex flex-wrap gap-1.5">
            {work.genres.map((genre) => (
              <span
                key={genre}
                className="rounded bg-zinc-700 px-2 py-0.5 text-xs text-zinc-300"
              >
                {genre}
              </span>
            ))}
          </div>
        ) : work.enriching ? (
          <div className="mt-3 flex flex-wrap gap-1.5">
            <SkeletonBlock className="h-5 w-16" />
            <SkeletonBlock className="h-5 w-20" />
            <SkeletonBlock className="h-5 w-14" />
          </div>
        ) : null}

        {work.description ? (
          <p className="mt-4 line-clamp-4 text-sm text-zinc-400">
            {work.description}
          </p>
        ) : work.enriching ? (
          <div className="mt-4 space-y-1.5">
            <SkeletonBlock className="h-3 w-full" />
            <SkeletonBlock className="h-3 w-full" />
            <SkeletonBlock className="h-3 w-2/3" />
          </div>
        ) : null}

      </div>
    </div>
  );
}
