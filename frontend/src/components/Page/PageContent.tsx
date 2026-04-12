import type { ReactNode } from "react";
import { cn } from "@/utils/cn";

export function PageContent({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("mx-auto w-full max-w-7xl px-3 py-4 sm:px-4 sm:py-6", className)}>
      {children}
    </div>
  );
}
