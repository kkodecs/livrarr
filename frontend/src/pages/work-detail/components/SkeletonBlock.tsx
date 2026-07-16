import { cn } from "@/utils/cn";

export function SkeletonBlock({ className }: { className?: string }) {
  return (
    <span
      aria-hidden
      className={cn("inline-block animate-pulse rounded bg-zinc-700/60", className)}
    />
  );
}
