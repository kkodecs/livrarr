interface Props {
  progressPct: number | null;
  mediaType: string;
  durationSeconds: number | null;
  finishedAt: string | null;
}

export default function ProgressBadge({ progressPct, mediaType, durationSeconds, finishedAt }: Props) {
  if (finishedAt) return <span className="text-xs text-green-500 font-medium">Complete</span>;
  if (!progressPct || progressPct <= 0) return null;

  if (mediaType === "ebook" || !durationSeconds || !Number.isFinite(durationSeconds)) {
    return <span className="text-xs text-zinc-400">{Math.round(progressPct * 100)}%</span>;
  }

  const remainingSecs = durationSeconds * (1 - progressPct);
  if (remainingSecs < 60) return <span className="text-xs text-zinc-400">&lt;1m</span>;

  const hours = Math.floor(remainingSecs / 3600);
  const minutes = Math.round((remainingSecs % 3600) / 60);

  const text = hours >= 1 ? `${hours}h ${minutes}m left` : `${minutes}m left`;
  return <span className="text-xs text-zinc-400">{text}</span>;
}
