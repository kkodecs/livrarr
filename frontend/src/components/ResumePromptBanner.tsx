import { X } from "lucide-react";
import { cn } from "@/utils/cn";

interface Props {
  label: string;
  onJump(): void;
  onStay(): void;
}

/**
 * Non-modal, dismissible banner offering to resume at a cross-format position.
 * Fixed at bottom center; never blocks or pauses the underlying player/reader.
 */
export function ResumePromptBanner({ label, onJump, onStay }: Props) {
  return (
    <div
      className={cn(
        "fixed bottom-6 left-1/2 z-50 -translate-x-1/2",
        "flex items-center gap-3 rounded-lg border border-zinc-700 bg-zinc-900",
        "px-4 py-3 shadow-xl",
      )}
    >
      <span className="text-sm text-zinc-200">
        Resume at <span className="font-medium text-zinc-100">{label}</span>?
      </span>
      <button
        onClick={onJump}
        className="rounded bg-brand px-3 py-1 text-sm font-medium text-white hover:bg-brand/90"
      >
        Jump
      </button>
      <button
        onClick={onStay}
        className="rounded px-3 py-1 text-sm text-zinc-400 hover:text-zinc-100"
      >
        Stay
      </button>
      <button
        onClick={onStay}
        className="ml-1 text-zinc-500 hover:text-zinc-300"
        title="Dismiss"
      >
        <X size={14} />
      </button>
    </div>
  );
}
