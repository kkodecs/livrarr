import { useState, useEffect, useRef } from "react";
import { BookOpen, Loader2 } from "lucide-react";
import { cn } from "@/utils/cn";
import { getCoverUrl } from "@/utils/format";

interface BookCoverProps {
  workId: number;
  title?: string;
  authorName?: string;
  className?: string;
  iconSize?: number;
  coverVersion?: number;
}

const MAX_RETRIES = 8;
const RETRY_DELAYS = [1000, 2000, 3000, 5000, 8000, 12000, 20000, 30000];

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

type CoverState = "loading" | "loaded" | "retrying" | "failed";

export function BookCover({
  workId,
  title,
  authorName,
  className = "h-16 w-11",
  iconSize = 16,
  coverVersion,
}: BookCoverProps) {
  const [state, setState] = useState<CoverState>("loading");
  const [resolvedSrc, setResolvedSrc] = useState<string | null>(null);
  const retryCount = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let cancelled = false;
    retryCount.current = 0;
    setState("loading");
    setResolvedSrc(null);

    function attempt() {
      const cacheBust =
        retryCount.current > 0
          ? `&_r=${retryCount.current}&_t=${Date.now()}`
          : "";
      const url = getCoverUrl(workId, coverVersion) + cacheBust;

      const img = new Image();
      img.onload = () => {
        if (!cancelled) {
          setResolvedSrc(url);
          setState("loaded");
        }
      };
      img.onerror = () => {
        if (cancelled) return;
        if (retryCount.current < MAX_RETRIES) {
          setState("retrying");
          const delay = RETRY_DELAYS[retryCount.current] ?? 30000;
          retryCount.current += 1;
          timerRef.current = setTimeout(attempt, delay);
        } else {
          setState("failed");
        }
      };
      img.src = url;
    }

    attempt();

    return () => {
      cancelled = true;
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [workId, coverVersion]);

  if (state === "loaded" && resolvedSrc) {
    return (
      <div
        className={cn(
          "relative shrink-0 rounded overflow-hidden bg-zinc-800",
          className,
        )}
      >
        <img
          src={resolvedSrc}
          alt=""
          aria-hidden
          className="absolute inset-0 h-full w-full object-cover blur-xl scale-125"
        />
        <img
          src={resolvedSrc}
          alt={title ?? ""}
          className="relative h-full w-full object-contain"
        />
      </div>
    );
  }

  const colorClass = FAUX_COLORS[workId % FAUX_COLORS.length];
  const showSpinner = state === "loading" || state === "retrying";

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
      {showSpinner && (
        <Loader2
          className="animate-spin text-zinc-400"
          style={{ width: "15%", height: "15%" }}
        />
      )}
    </div>
  );
}
