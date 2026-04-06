import { useQuery } from "@tanstack/react-query";
import { getSystemStatus } from "@/api";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { formatAbsoluteDate } from "@/utils/format";

export default function StatusPage() {
  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["system-status"],
    queryFn: getSystemStatus,
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;
  if (!data) return null;

  const rows: [string, string][] = [
    ["Version", data.version],
    ["OS", data.osInfo],
    ["Data Directory", data.dataDirectory],
    ["Startup Time", formatAbsoluteDate(data.startupTime)],
  ];

  return (
    <PageContent>
      <h1 className="mb-6 text-lg font-semibold text-zinc-100">About</h1>
      <dl className="max-w-lg space-y-3">
        {rows.map(([label, value]) => (
          <div key={label} className="flex items-baseline gap-4">
            <dt className="w-36 shrink-0 text-sm text-muted">{label}</dt>
            <dd className="text-sm text-zinc-200">{value}</dd>
          </div>
        ))}
      </dl>
    </PageContent>
  );
}
