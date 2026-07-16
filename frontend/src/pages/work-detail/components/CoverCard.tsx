import { useState } from "react";
import { cn } from "@/utils/cn";
import type { CoverCandidate } from "@/api";

export function CoverCard({
  candidate,
  selecting,
  onSelect,
}: {
  candidate: CoverCandidate;
  selecting: boolean;
  onSelect: () => void;
}) {
  const [failed, setFailed] = useState(false);
  const [dims, setDims] = useState<{ w: number; h: number } | null>(null);
  const good = dims ? dims.w >= 400 && dims.h >= 600 : null;

  if (failed) return null;

  return (
    <button
      type="button"
      onClick={onSelect}
      disabled={selecting}
      className="group relative rounded border border-zinc-700 hover:border-brand overflow-hidden bg-zinc-800 transition-colors"
    >
      <div className="aspect-[2/3] relative">
        <img
          src={candidate.proxyUrl}
          alt={candidate.source}
          className="absolute inset-0 h-full w-full object-contain"
          onError={() => setFailed(true)}
          onLoad={(e) => {
            const img = e.target as HTMLImageElement;
            setDims({ w: img.naturalWidth, h: img.naturalHeight });
          }}
        />
      </div>
      <div className="px-1.5 py-1 text-center">
        <span className="text-[11px] text-zinc-400 block truncate">{candidate.source}</span>
        {dims && (
          <span className={cn("text-[10px]", good ? "text-green-500" : "text-amber-500")}>
            {dims.w}&times;{dims.h}
          </span>
        )}
      </div>
    </button>
  );
}
