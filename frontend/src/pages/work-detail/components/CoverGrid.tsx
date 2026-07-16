import { Upload } from "lucide-react";
import { BookCover } from "@/components/BookCover";
import type { CoverCandidate } from "@/api";
import { CoverCard } from "./CoverCard";

export function CoverGrid({
  label,
  candidates,
  selecting,
  onSelect,
  onUpload,
  current,
}: {
  label: string;
  candidates: CoverCandidate[];
  selecting: boolean;
  onSelect: (c: CoverCandidate) => void;
  onUpload: (file: Blob) => void;
  current?: { workId: number; coverVersion?: number; mediaType?: "audiobook" };
}) {
  return (
    <div>
      <span className="text-[10px] font-medium text-zinc-400 uppercase tracking-wider">{label}</span>
      <div className="grid grid-cols-3 gap-2 mt-1">
        {current && (
          <div className="relative rounded border border-emerald-700/60">
            <BookCover
              workId={current.workId}
              coverVersion={current.coverVersion}
              mediaType={current.mediaType}
              className="h-full w-full rounded"
            />
            <span className="absolute bottom-1 left-1 rounded bg-emerald-900/80 px-1 py-0.5 text-[9px] text-emerald-300">
              current
            </span>
          </div>
        )}
        {candidates.map((c) => (
          <CoverCard
            key={c.candidateId}
            candidate={c}
            selecting={selecting}
            onSelect={() => onSelect(c)}
          />
        ))}
        <label className="flex flex-col items-center justify-center gap-1 rounded border border-dashed border-zinc-600 hover:border-brand bg-zinc-800/50 cursor-pointer aspect-[2/3] transition-colors">
          <Upload size={16} className="text-zinc-500" />
          <span className="text-[9px] text-zinc-500">Upload</span>
          <input
            type="file"
            accept="image/jpeg,image/png,image/webp"
            onChange={(e) => {
              const file = e.target.files?.[0];
              if (file) onUpload(file);
            }}
            className="hidden"
          />
        </label>
      </div>
    </div>
  );
}
