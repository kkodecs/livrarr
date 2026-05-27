import { useQuery } from "@tanstack/react-query";
import {
  CheckCircle2,
  Brain,
  Search,
  Download,
  Rss,
  Database,
  Loader2,
} from "lucide-react";
import { getSystemStatus, getHealth, getHealthSummary } from "@/api";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { formatAbsoluteDate, formatRelativeDate } from "@/utils/format";
import { cn } from "@/utils/cn";
import type { HealthCheckType, ProviderStatusInfo, InfraItemStatus, HealthSummaryResponse } from "@/types/api";

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

function StatusDot({ ok }: { ok: boolean }) {
  return (
    <span
      className={cn(
        "inline-block h-2 w-2 rounded-full flex-shrink-0",
        ok ? "bg-green-400" : "bg-red-400",
      )}
    />
  );
}

function SectionHeader({
  icon,
  label,
}: {
  icon: React.ReactNode;
  label: string;
}) {
  return (
    <div className="flex items-center gap-2 mb-3">
      <span className="text-muted">{icon}</span>
      <h2 className="text-sm font-semibold text-zinc-100 uppercase tracking-wider">
        {label}
      </h2>
    </div>
  );
}

function ProviderRow({ p }: { p: ProviderStatusInfo }) {
  return (
    <div className="flex items-center gap-3 py-1.5">
      <StatusDot ok={p.status === "ok"} />
      <span className="text-sm text-zinc-200 flex-1">{p.name}</span>
      {p.lastError && (
        <span className="text-xs text-red-400 truncate max-w-[200px]">
          {p.lastError}
        </span>
      )}
    </div>
  );
}

function InfraRow({ item }: { item: InfraItemStatus }) {
  return (
    <div className="flex items-center gap-3 py-1.5">
      <StatusDot ok={item.enabled} />
      <span className="text-sm text-zinc-200 flex-1">{item.name}</span>
      <span className="text-xs text-zinc-500">{item.implementation}</span>
      {!item.enabled && (
        <span className="text-xs text-zinc-500">disabled</span>
      )}
    </div>
  );
}

function HealthSummarySection({ summary }: { summary: HealthSummaryResponse }) {
  return (
    <div className="space-y-6 max-w-lg">
      {/* LLM */}
      <div>
        <SectionHeader icon={<Brain size={16} />} label="LLM" />
        <div className="flex items-center gap-3 py-1.5">
          <StatusDot ok={summary.llm.configured} />
          <span className="text-sm text-zinc-200 flex-1">
            {summary.llm.configured
              ? `${summary.llm.provider ?? "custom"} — ${summary.llm.model ?? "unknown model"}`
              : summary.llm.enabled
                ? "Enabled but not fully configured"
                : "Not configured"}
          </span>
        </div>
      </div>

      {/* Indexers */}
      <div>
        <SectionHeader icon={<Search size={16} />} label="Indexers" />
        {summary.indexers.length === 0 ? (
          <p className="text-sm text-zinc-500 py-1.5">No indexers configured</p>
        ) : (
          summary.indexers.map((ix) => <InfraRow key={ix.id} item={ix} />)
        )}
      </div>

      {/* Download Clients */}
      <div>
        <SectionHeader
          icon={<Download size={16} />}
          label="Download Clients"
        />
        {summary.downloadClients.length === 0 ? (
          <p className="text-sm text-zinc-500 py-1.5">
            No download clients configured
          </p>
        ) : (
          summary.downloadClients.map((dc) => (
            <InfraRow key={dc.id} item={dc} />
          ))
        )}
      </div>

      {/* RSS Sync */}
      <div>
        <SectionHeader icon={<Rss size={16} />} label="RSS Sync" />
        <div className="flex items-center gap-3 py-1.5">
          {summary.rssSync.running ? (
            <>
              <Loader2 size={12} className="animate-spin text-brand" />
              <span className="text-sm text-brand">Running</span>
            </>
          ) : (
            <>
              <StatusDot ok />
              <span className="text-sm text-zinc-200">Idle</span>
            </>
          )}
          {summary.rssSync.lastRunAt && (
            <span className="text-xs text-zinc-500">
              Last run {formatRelativeDate(summary.rssSync.lastRunAt)}
            </span>
          )}
          {!summary.rssSync.lastRunAt && !summary.rssSync.running && (
            <span className="text-xs text-zinc-500">Never run</span>
          )}
        </div>
      </div>

      {/* Metadata Providers */}
      <div>
        <SectionHeader
          icon={<Database size={16} />}
          label="Metadata Providers"
        />
        {summary.metadataProviders.map((p) => (
          <ProviderRow key={p.name} p={p} />
        ))}
      </div>
    </div>
  );
}

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

  const {
    data: summary,
    isLoading: summaryLoading,
  } = useQuery({
    queryKey: ["health-summary"],
    queryFn: getHealthSummary,
    refetchInterval: 60_000,
  });

  if (statusLoading || healthLoading || summaryLoading) return <PageLoading />;
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
      {/* System Info */}
      <h1 className="mb-6 text-lg font-semibold text-zinc-100">Status</h1>
      <dl className="max-w-lg space-y-3">
        {rows.map(([label, value]) => (
          <div
            key={label}
            className="flex flex-col sm:flex-row sm:items-baseline gap-1 sm:gap-4"
          >
            <dt className="w-36 shrink-0 text-sm text-muted">{label}</dt>
            <dd className="text-sm text-zinc-200 break-all">{value}</dd>
          </div>
        ))}
      </dl>

      {/* Health Checks */}
      <div className="mt-10 mb-6">
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
                  "flex flex-col sm:flex-row items-start gap-2 sm:gap-4 rounded-lg border border-border px-3 sm:px-4 py-2 sm:py-3",
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
      </div>

      {/* Health Summary */}
      {summary && (
        <div className="mt-8 border-t border-border pt-8">
          <h2 className="mb-6 text-lg font-semibold text-zinc-100">
            Infrastructure
          </h2>
          <HealthSummarySection summary={summary} />
        </div>
      )}
    </PageContent>
  );
}
