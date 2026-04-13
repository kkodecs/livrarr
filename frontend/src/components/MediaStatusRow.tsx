import { Link } from "react-router";
import { Book, Headphones, Search } from "lucide-react";
import { formatMB } from "@/utils/format";
import type { WorkDetailResponse } from "@/types/api";

export function MediaStatusRow({
  work,
  activeGrabs,
}: {
  work: WorkDetailResponse;
  activeGrabs?: Set<string>;
}) {
  const grabs = activeGrabs ?? new Set<string>();
  const ebookItems =
    work.libraryItems?.filter((li) => li.mediaType === "ebook") ?? [];
  const audioItems =
    work.libraryItems?.filter((li) => li.mediaType === "audiobook") ?? [];
  const ebookSize = ebookItems.reduce((acc, li) => acc + li.fileSize, 0);
  const audioSize = audioItems.reduce((acc, li) => acc + li.fileSize, 0);
  const ebookDownloading = grabs.has(`${work.id}-ebook`);
  const audioDownloading = grabs.has(`${work.id}-audiobook`);

  function typeStatus(
    monitored: boolean,
    hasFile: boolean,
    fileSize: number,
    downloading: boolean,
  ): { color: string; label: string } {
    if (!monitored) return { color: "text-zinc-600", label: "unmonitored" };
    if (hasFile) return { color: "text-green-400", label: formatMB(fileSize) };
    if (downloading) return { color: "text-purple-400", label: "downloading" };
    return { color: "text-amber-500", label: "Missing" };
  }

  const ebook = typeStatus(
    work.monitorEbook,
    ebookItems.length > 0,
    ebookSize,
    ebookDownloading,
  );
  const audio = typeStatus(
    work.monitorAudiobook,
    audioItems.length > 0,
    audioSize,
    audioDownloading,
  );

  return (
    <div className="flex items-center gap-3 text-xs">
      <span className="inline-flex items-center gap-1">
        <Book size={12} className={ebook.color} />
        <span className={ebook.color}>{ebook.label}</span>
      </span>
      <span className="inline-flex items-center gap-1">
        <Headphones size={12} className={audio.color} />
        <span className={audio.color}>{audio.label}</span>
      </span>
      <Link
          to={`/work/${work.id}?tab=releases`}
          onClick={(e) => e.stopPropagation()}
          className="inline-flex items-center text-zinc-500 hover:text-brand transition-colors"
          title="Search releases"
        >
          <Search size={12} />
      </Link>
    </div>
  );
}
