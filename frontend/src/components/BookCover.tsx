import { useState } from "react";
import { BookOpen } from "lucide-react";
import { cn } from "@/utils/cn";
import { getCoverUrl } from "@/utils/format";

interface BookCoverProps {
  workId: number;
  title?: string;
  authorName?: string;
  className?: string;
  iconSize?: number;
  coverVersion?: number;
  mediaType?: "ebook" | "audiobook";
}

const FAUX_COLORS = [
  "from-indigo-900 to-indigo-700",
  "from-emerald-900 to-emerald-700",
  "from-amber-900 to-amber-700",
  "from-rose-900 to-rose-700",
  "from-cyan-900 to-cyan-700",
  "from-purple-900 to-purple-700",
  "from-teal-900 to-teal-700",
  "from-orange-900 to-orange-700",
];

export function BookCover({
  workId,
  title,
  authorName,
  className = "h-16 w-11",
  iconSize = 16,
  coverVersion,
  mediaType,
}: BookCoverProps) {
  const [failed, setFailed] = useState(false);
  const src = getCoverUrl(workId, coverVersion, mediaType);

  if (!failed) {
    return (
      <div
        className={cn(
          "relative shrink-0 rounded overflow-hidden bg-zinc-800",
          className,
        )}
      >
        <img
          src={src}
          alt=""
          aria-hidden
          className="absolute inset-0 h-full w-full object-cover blur-xl scale-125"
        />
        <img
          src={src}
          alt={title ?? ""}
          className="relative h-full w-full object-contain"
          onError={() => setFailed(true)}
        />
      </div>
    );
  }

  const colorClass = FAUX_COLORS[workId % FAUX_COLORS.length];

  return (
    <div
      className={cn(
        "shrink-0 rounded overflow-hidden flex flex-col items-center justify-center p-2 bg-gradient-to-b border border-zinc-700 gap-1",
        colorClass,
        className,
      )}
    >
      {title ? (
        <>
          <span className="text-[0.45em] font-semibold leading-tight text-zinc-100 text-center line-clamp-3">
            {title}
          </span>
          {authorName && (
            <span className="text-[0.35em] leading-tight text-zinc-300 text-center line-clamp-1">
              {authorName}
            </span>
          )}
        </>
      ) : (
        <BookOpen size={iconSize} className="text-zinc-500" />
      )}
    </div>
  );
}
