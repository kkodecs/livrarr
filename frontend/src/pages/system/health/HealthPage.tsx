import { useQuery } from "@tanstack/react-query";
import { CheckCircle2 } from "lucide-react";
import { getHealth } from "@/api";
import { PageContent } from "@/components/Page/PageContent";
import { EmptyState } from "@/components/Page/EmptyState";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { cn } from "@/utils/cn";
import type { HealthCheckType } from "@/types/api";

const typeColors: Record<HealthCheckType, string> = {
  ok: "text-green-400",
  warning: "text-amber-400",
  error: "text-red-400",
};

const typeBgColors: Record<HealthCheckType, string> = {
  ok: "bg-green-400/10",
  warning: "bg-amber-400/10",
  error: "bg-red-400/10",
};

export default function HealthPage() {
  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["health"],
    queryFn: getHealth,
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  const checks = data ?? [];

  if (checks.length === 0) {
    return (
      <PageContent>
        <EmptyState
          icon={<CheckCircle2 size={40} className="text-green-400" />}
          title="All systems healthy"
        />
      </PageContent>
    );
  }

  return (
    <PageContent>
      <h1 className="mb-6 text-lg font-semibold text-zinc-100">Health</h1>
      <ul className="space-y-2">
        {checks.map((check, i) => (
          <li
            key={i}
            className={cn(
              "flex items-start gap-4 rounded-lg border border-border px-4 py-3",
              typeBgColors[check.checkType],
            )}
          >
            <span
              className={cn(
                "mt-0.5 text-xs font-semibold uppercase",
                typeColors[check.checkType],
              )}
            >
              {check.checkType}
            </span>
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium text-zinc-200">
                {check.source}
              </p>
              <p className="mt-0.5 text-sm text-muted">{check.message}</p>
            </div>
          </li>
        ))}
      </ul>
    </PageContent>
  );
}
