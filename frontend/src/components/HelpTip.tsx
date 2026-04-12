import { Info } from "lucide-react";
import { useRef, useState } from "react";
import { createPortal } from "react-dom";

export function HelpTip({ text }: { text: string }) {
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const iconRef = useRef<HTMLSpanElement>(null);

  const show = () => {
    const rect = iconRef.current?.getBoundingClientRect();
    if (rect) setPos({ top: rect.bottom + 6, left: rect.left });
  };
  const hide = () => setPos(null);

  return (
    <span ref={iconRef} className="inline-flex" onMouseEnter={show} onMouseLeave={hide}>
      <Info size={14} className="text-muted" />
      {pos &&
        createPortal(
          <span
            style={{ top: pos.top, left: pos.left }}
            className="pointer-events-none fixed z-[9999] w-64 rounded bg-zinc-700 px-3 py-2 text-xs text-zinc-200 shadow-lg"
          >
            {text}
          </span>,
          document.body,
        )}
    </span>
  );
}
