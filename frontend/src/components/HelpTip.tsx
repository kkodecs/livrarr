import { Info } from "lucide-react";

export function HelpTip({ text }: { text: string }) {
  return (
    <span className="relative group inline-flex">
      <Info size={14} className="text-muted" />
      <span className="pointer-events-none absolute left-6 top-1/2 -translate-y-1/2 z-50 hidden w-64 rounded bg-zinc-700 px-3 py-2 text-xs text-zinc-200 shadow-lg group-hover:block">
        {text}
      </span>
    </span>
  );
}
