import type { ReactNode } from "react";

export function PageToolbar({ children }: { children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-border bg-zinc-900 px-4 py-3">
      {children}
    </div>
  );
}
