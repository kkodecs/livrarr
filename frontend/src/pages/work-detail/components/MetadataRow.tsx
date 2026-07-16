import { SkeletonBlock } from "./SkeletonBlock";

export function MetadataRow({
  label,
  value,
  skeleton,
}: {
  label: string;
  value: React.ReactNode;
  skeleton?: boolean;
}) {
  if (!value) {
    if (!skeleton) return null;
    return (
      <div className="flex gap-4 py-2 border-b border-border/30">
        <dt className="w-36 shrink-0 text-xs text-muted uppercase tracking-wide">{label}</dt>
        <dd className="text-sm">
          <SkeletonBlock className="h-3.5 w-32" />
        </dd>
      </div>
    );
  }
  return (
    <div className="flex gap-4 py-2 border-b border-border/30">
      <dt className="w-36 shrink-0 text-xs text-muted uppercase tracking-wide">{label}</dt>
      <dd className="text-sm text-zinc-200">{value}</dd>
    </div>
  );
}
