interface Props {
  progressPct: number | null;
}

export default function ProgressBar({ progressPct }: Props) {
  const hasProgress = progressPct != null && progressPct > 0;
  const width = hasProgress
    ? progressPct >= 0.98
      ? 100
      : Math.min(progressPct * 100, 100)
    : 0;

  return (
    <div className="absolute bottom-0 left-0 right-0 h-[3px] bg-black/20">
      {hasProgress && (
        <div
          className="h-full bg-blue-500 transition-all duration-300"
          style={{ width: `${width}%` }}
        />
      )}
    </div>
  );
}
