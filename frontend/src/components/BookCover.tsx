import { useState, useRef } from "react";
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

const MAX_RETRIES = 2;
const RETRY_DELAY_MS = 2000;

export function BookCover({
  workId,
  title,
  className = "h-16 w-11",
  iconSize = 16,
  coverVersion,
}: BookCoverProps) {
  const [failed, setFailed] = useState(false);
  const retries = useRef(0);

  const handleError = () => {
    if (retries.current < MAX_RETRIES) {
      retries.current += 1;
      setTimeout(() => {
        setFailed(false);
      }, RETRY_DELAY_MS);
    }
    setFailed(true);
  };

  if (failed && retries.current >= MAX_RETRIES) {
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

  const src = getCoverUrl(workId, coverVersion);
  const retrySrc = retries.current > 0 ? `${src}&_r=${retries.current}` : src;

  return (
    <div
      className={cn(
        "relative shrink-0 rounded overflow-hidden bg-zinc-800",
        className,
      )}
    >
      <img
        src={retrySrc}
        alt=""
        aria-hidden
        className="absolute inset-0 h-full w-full object-cover blur-xl scale-125"
        loading="lazy"
      />
      <img
        src={retrySrc}
        alt={title ?? ""}
        className="relative h-full w-full object-contain"
        loading="lazy"
        onError={handleError}
      />
    </div>
  );
}
