interface Props {
  progressPct: number | null;
}

export default function ProgressBar({ progressPct }: Props) {
  if (!progressPct || progressPct <= 0) return null;

  const width = progressPct >= 0.98 ? 100 : Math.min(progressPct * 100, 100);

  return (
    <div className="absolute bottom-0 left-0 right-0 h-[3px] bg-black/20">
      <div
        className="h-full bg-blue-500 transition-all duration-300"
        style={{ width: `${width}%` }}
      />
    </div>
  );
}
