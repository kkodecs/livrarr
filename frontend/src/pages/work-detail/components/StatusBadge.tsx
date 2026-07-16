import { cn } from "@/utils/cn";
import { HelpTip } from "@/components/HelpTip";

export type BadgeTone = "green" | "blue" | "amber" | "red" | "orange" | "zinc";

export const BADGE_TONE: Record<BadgeTone, { wrap: string; dot: string }> = {
  green: { wrap: "text-green-400 bg-green-500/10 border-green-500/30", dot: "bg-green-500" },
  blue: { wrap: "text-blue-400 bg-blue-500/10 border-blue-500/30", dot: "bg-blue-500" },
  amber: { wrap: "text-amber-400 bg-amber-500/10 border-amber-500/30", dot: "bg-amber-500" },
  red: { wrap: "text-red-400 bg-red-500/10 border-red-500/30", dot: "bg-red-500" },
  orange: { wrap: "text-orange-400 bg-orange-500/10 border-orange-500/30", dot: "bg-orange-500" },
  zinc: { wrap: "text-zinc-400 bg-zinc-500/10 border-zinc-500/25", dot: "bg-zinc-500" },
};

export function StatusBadge({ tone, label, tip }: { tone: BadgeTone; label: string; tip: string }) {
  const t = BADGE_TONE[tone];
  return (
    <span className={cn("inline-flex items-center gap-2 rounded-full border px-2.5 py-1 text-xs font-medium", t.wrap)}>
      <span className={cn("h-1.5 w-1.5 rounded-full", t.dot)} />
      {label}
      <HelpTip text={tip} />
    </span>
  );
}
