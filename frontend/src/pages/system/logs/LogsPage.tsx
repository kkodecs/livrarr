import { useState, useRef, useCallback, useEffect } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import { RefreshCw, Search, ChevronUp, ChevronDown, X } from "lucide-react";
import { toast } from "sonner";
import { getLogTail, setLogLevel as apiSetLogLevel, getSystemStatus } from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { HelpTip } from "@/components/HelpTip";

export default function LogsPage() {
  const { data, isLoading, error, refetch, isFetching } = useQuery({
    queryKey: ["logs"],
    queryFn: () => getLogTail(200),
    refetchInterval: 5000,
  });

  const { data: statusData } = useQuery({
    queryKey: ["system-status"],
    queryFn: getSystemStatus,
  });

  const [logLevel, setLogLevel] = useState(statusData?.logLevel ?? "info");
  // Sync with server when status loads.
  useEffect(() => {
    if (statusData?.logLevel) setLogLevel(statusData.logLevel);
  }, [statusData?.logLevel]);
  const levelMut = useMutation({
    mutationFn: apiSetLogLevel,
    onSuccess: (data: { level: string }) => {
      setLogLevel(data.level);
      toast.success(`Log level set to ${data.level}`);
    },
    onError: (err: Error) =>
      toast.error(err.message || "Failed to set log level"),
  });

  const [displayLevel, setDisplayLevel] = useState("all");

  const [search, setSearch] = useState("");
  const [matchIndex, setMatchIndex] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);
  const matchRefs = useRef<(HTMLSpanElement | null)[]>([]);

  const LOG_LEVELS = ["trace", "debug", "info", "warn", "error"];
  const allLines = data ?? [];
  const lines =
    displayLevel === "all"
      ? allLines
      : allLines.filter((line) => {
          const levelMatch = line.match(
            /^\S+\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s/,
          );
          if (!levelMatch?.[1]) return true;
          const lineLevel = levelMatch[1].toLowerCase();
          return (
            LOG_LEVELS.indexOf(lineLevel) >= LOG_LEVELS.indexOf(displayLevel)
          );
        });

  // Find all matching line indices
  const lowerSearch = search.toLowerCase();
  const matchingLines = search
    ? lines
        .map((line, i) => (line.toLowerCase().includes(lowerSearch) ? i : -1))
        .filter((i) => i >= 0)
    : [];

  // Scroll to current match
  const scrollToMatch = useCallback((idx: number) => {
    const el = matchRefs.current[idx];
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, []);

  // Reset match index when search changes
  useEffect(() => {
    setMatchIndex(0);
    if (matchingLines.length > 0) {
      setTimeout(() => scrollToMatch(0), 0);
    }
  }, [search]); // eslint-disable-line react-hooks/exhaustive-deps

  const goNext = () => {
    if (matchingLines.length === 0) return;
    const next = (matchIndex + 1) % matchingLines.length;
    setMatchIndex(next);
    scrollToMatch(next);
  };

  const goPrev = () => {
    if (matchingLines.length === 0) return;
    const prev =
      (matchIndex - 1 + matchingLines.length) % matchingLines.length;
    setMatchIndex(prev);
    scrollToMatch(prev);
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") {
      if (e.shiftKey) goPrev();
      else goNext();
      e.preventDefault();
    }
    if (e.key === "Escape") {
      setSearch("");
    }
  };

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  // Reset refs array
  matchRefs.current = [];
  let matchCounter = 0;

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Logs</h1>
        <div className="flex-1" />
        <div className="flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-1.5 text-xs text-muted">
            Show:
            <HelpTip text="Filter which log lines are shown on this page" />
            <select
              value={displayLevel}
              onChange={(e) => setDisplayLevel(e.target.value)}
              className="rounded border border-border bg-zinc-800 px-2 py-1.5 text-xs text-zinc-100 focus:border-brand focus:outline-none"
            >
              <option value="all">All</option>
              <option value="error">Error+</option>
              <option value="warn">Warn+</option>
              <option value="info">Info+</option>
              <option value="debug">Debug+</option>
              <option value="trace">Trace+</option>
            </select>
          </label>
          <label className="flex items-center gap-1.5 text-xs text-muted">
            Capture:
            <HelpTip text="Server-side log level — what gets captured. Takes effect immediately, resets on restart." />
            <select
              value={logLevel}
              onChange={(e) => levelMut.mutate(e.target.value)}
              className="rounded border border-border bg-zinc-800 px-2 py-1.5 text-xs text-zinc-100 focus:border-brand focus:outline-none"
            >
              <option value="error">Error</option>
              <option value="warn">Warn</option>
              <option value="info">Info</option>
              <option value="debug">Debug</option>
              <option value="trace">Trace</option>
            </select>
          </label>
          <button
            onClick={() => refetch()}
            disabled={isFetching}
            className="inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-xs text-muted hover:text-zinc-100 hover:bg-zinc-800 disabled:opacity-50"
          >
            <RefreshCw
              size={12}
              className={isFetching ? "animate-spin" : ""}
            />
            Refresh
          </button>
        </div>
      </PageToolbar>

      <PageContent>
        <div
          ref={containerRef}
          className="rounded border border-border bg-zinc-950 p-4 overflow-x-auto max-h-[calc(100vh-11rem)] overflow-y-auto"
        >
          {lines.length > 0 ? (
            <pre className="text-xs font-mono leading-5 whitespace-pre">
              {lines.map((line, i) => {
                const isMatch =
                  search && line.toLowerCase().includes(lowerSearch);
                const isCurrentMatch =
                  isMatch && matchingLines[matchIndex] === i;
                const refIdx = isMatch ? matchCounter++ : -1;

                if (!isMatch) {
                  return (
                    <div key={i} className="text-zinc-400">
                      {line}
                    </div>
                  );
                }

                return (
                  <div
                    key={i}
                    ref={(el) => {
                      if (refIdx >= 0) matchRefs.current[refIdx] = el;
                    }}
                    className={
                      isCurrentMatch
                        ? "bg-amber-500/20 text-amber-200"
                        : "bg-amber-500/10 text-amber-300/80"
                    }
                  >
                    {highlightMatch(line, lowerSearch)}
                  </div>
                );
              })}
            </pre>
          ) : (
            <p className="text-sm text-muted">No log lines yet.</p>
          )}
        </div>

        {/* Search bar */}
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <div className="relative flex items-center">
            <Search
              size={14}
              className="absolute left-2.5 text-muted pointer-events-none"
            />
            <input
              type="text"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Search logs..."
              className="w-full sm:w-64 rounded border border-border bg-zinc-800 py-1.5 pl-8 pr-8 text-xs text-zinc-100 placeholder:text-muted focus:border-brand focus:outline-none"
            />
            {search && (
              <button
                onClick={() => setSearch("")}
                className="absolute right-2 text-muted hover:text-zinc-300"
              >
                <X size={12} />
              </button>
            )}
          </div>
          {search && matchingLines.length > 0 && (
            <div className="flex items-center gap-1">
              <span className="text-xs text-muted">
                {matchIndex + 1}/{matchingLines.length}
              </span>
              <button
                onClick={goPrev}
                className="rounded p-1 text-muted hover:text-zinc-100 hover:bg-zinc-700"
              >
                <ChevronUp size={14} />
              </button>
              <button
                onClick={goNext}
                className="rounded p-1 text-muted hover:text-zinc-100 hover:bg-zinc-700"
              >
                <ChevronDown size={14} />
              </button>
            </div>
          )}
          {search && matchingLines.length === 0 && (
            <span className="text-xs text-red-400">No matches</span>
          )}
          <div className="flex-1" />
          <span className="hidden sm:inline text-xs text-zinc-600">
            Showing last 200 lines{statusData?.logFile ? ` — log file: ${statusData.logFile}` : ""}
          </span>
        </div>
      </PageContent>
    </>
  );
}

function highlightMatch(line: string, search: string): React.ReactNode {
  if (!search) return line;
  const lower = line.toLowerCase();
  const idx = lower.indexOf(search);
  if (idx < 0) return line;

  return (
    <>
      {line.slice(0, idx)}
      <mark className="bg-amber-500/40 text-amber-100 rounded-sm px-0.5">
        {line.slice(idx, idx + search.length)}
      </mark>
      {line.slice(idx + search.length)}
    </>
  );
}
