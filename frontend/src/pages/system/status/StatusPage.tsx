import { useQuery } from "@tanstack/react-query";
import { CheckCircle2 } from "lucide-react";
import { getSystemStatus, getHealth } from "@/api";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { formatAbsoluteDate } from "@/utils/format";
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

export default function StatusPage() {
  const {
    data: status,
    isLoading: statusLoading,
    error: statusError,
    refetch: refetchStatus,
  } = useQuery({
    queryKey: ["system-status"],
    queryFn: getSystemStatus,
  });

  const {
    data: healthData,
    isLoading: healthLoading,
    error: healthError,
    refetch: refetchHealth,
  } = useQuery({
    queryKey: ["health"],
    queryFn: getHealth,
  });

  if (statusLoading || healthLoading) return <PageLoading />;
  if (statusError)
    return <ErrorState error={statusError} onRetry={() => refetchStatus()} />;
  if (healthError)
    return <ErrorState error={healthError} onRetry={() => refetchHealth()} />;
  if (!status) return null;

  const rows: [string, string][] = [
    ["Version", status.version],
    ["OS", status.osInfo],
    ["Data Directory", status.dataDirectory],
    ["Log File", status.logFile],
    ["Startup Time", formatAbsoluteDate(status.startupTime)],
  ];

  const checks = healthData ?? [];

  return (
    <PageContent>
      {/* Status */}
      <h1 className="mb-6 text-lg font-semibold text-zinc-100">Status</h1>
      <dl className="max-w-lg space-y-3">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-baseline gap-4">
            <dt className="w-36 shrink-0 text-sm text-muted">{label}</dt>
            <dd className="text-sm text-zinc-200">{value}</dd>
          </div>
        ))}
      </dl>

      {/* Health */}
      <div className="mt-10 mb-4" />
      {checks.length === 0 ? (
        <div className="flex items-center gap-2 text-sm text-green-400">
          <CheckCircle2 size={16} />
          All systems healthy
        </div>
      ) : (
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
      )}
    </PageContent>
  );
}
