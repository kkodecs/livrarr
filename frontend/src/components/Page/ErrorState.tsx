import { AlertTriangle, RefreshCw } from "lucide-react";
import { ApiError } from "@/api/client";

export function ErrorState({
  error,
  onRetry,
}: {
  error: Error;
  onRetry?: () => void;
}) {
  const message =
    error instanceof ApiError ? error.message : "Something went wrong";

  return (
    <div className="flex flex-col items-center justify-center py-16 text-center">
      <AlertTriangle className="mb-4 text-red-400" size={32} />
      <h3 className="text-lg font-medium text-zinc-200">{message}</h3>
      {onRetry && (
        <button
          onClick={onRetry}
          className="mt-4 inline-flex items-center gap-2 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover"
        >
          <RefreshCw size={14} />
          Retry
        </button>
      )}
    </div>
  );
}
