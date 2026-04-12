import type { ReactNode } from "react";

export function PageToolbar({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-col gap-3 border-b border-border bg-zinc-900 px-4 py-3 sm:flex-row sm:items-center sm:justify-between sm:gap-4">
      {children}
    </div>
  );
}
