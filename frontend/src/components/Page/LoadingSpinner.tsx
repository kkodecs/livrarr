import { Loader2 } from "lucide-react";
import { cn } from "@/utils/cn";

export function LoadingSpinner({
  className,
  size = 24,
}: {
  className?: string;
  size?: number;
}) {
  return (
    <Loader2 className={cn("animate-spin text-muted", className)} size={size} />
  );
}

export function PageLoading() {
  return (
    <div className="flex h-64 items-center justify-center">
      <LoadingSpinner size={32} />
    </div>
  );
}

export function FullPageLoading() {
  return (
    <div className="flex h-screen items-center justify-center bg-zinc-900">
      <LoadingSpinner size={40} />
    </div>
  );
}
