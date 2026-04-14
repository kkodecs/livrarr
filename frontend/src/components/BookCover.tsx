import { useState } from "react";
import { BookOpen } from "lucide-react";
import { cn } from "@/utils/cn";
import { getCoverUrl } from "@/utils/format";

interface BookCoverProps {
  workId: number;
  title?: string;
  className?: string;
  iconSize?: number;
  coverVersion?: number;
}

export function BookCover({
  workId,
  title,
  className = "h-16 w-11",
  iconSize = 16,
  coverVersion,
}: BookCoverProps) {
  const [failed, setFailed] = useState(false);

  if (failed) {
    return (
      <div
        className={cn(
          "shrink-0 rounded bg-zinc-800 flex flex-col items-center justify-center overflow-hidden border border-zinc-700",
          className,
        )}
        title={title}
      >
        <BookOpen size={iconSize} className="text-zinc-600 mb-0.5" />
        {title && (
          <span className="text-[7px] leading-tight text-zinc-500 text-center px-0.5 line-clamp-2">
            {title}
          </span>
        )}
      </div>
    );
  }

  return (
    <img
      src={getCoverUrl(workId, coverVersion)}
      alt={title ?? ""}
      className={cn("shrink-0 rounded bg-zinc-700 object-cover", className)}
      loading="lazy"
      onError={() => setFailed(true)}
    />
  );
}
