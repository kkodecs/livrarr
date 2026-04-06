import { useState } from "react";
import { Link } from "react-router";
import { useQuery, useMutation } from "@tanstack/react-query";
import { toast } from "sonner";
import { FolderSearch, Search, Settings } from "lucide-react";
import { listRootFolders, scanRootFolder, scanUnmappedPath } from "@/api";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { EmptyState } from "@/components/Page/EmptyState";
import { PageLoading, LoadingSpinner } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { PathPicker } from "@/components/PathPicker/PathPicker";
import type { ScanResult } from "@/types/api";

type ScanMode = "rootfolder" | "path";

export default function UnmappedPage() {
  const [mode, setMode] = useState<ScanMode>("path");
  const [selectedId, setSelectedId] = useState<number | "">("");
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState("");
  const [showPicker, setShowPicker] = useState(false);
  const [result, setResult] = useState<ScanResult | null>(null);

  const {
    data: rootFolders,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["root-folders"],
    queryFn: listRootFolders,
  });

  const scanFolderMutation = useMutation({
    mutationFn: (id: number) => scanRootFolder(id),
    onSuccess: (data) => {
      setResult(data);
      toast.success("Scan complete");
    },
    onError: (e: Error) => toast.error(e.message ?? "Scan failed"),
  });

  const scanPathMutation = useMutation({
    mutationFn: (p: string) => scanUnmappedPath(p),
    onSuccess: (data) => {
      setResult(data);
      setPathError("");
      toast.success("Scan complete");
    },
    onError: (e: Error) => {
      setPathError(e.message || "The file system path specified was not found.");
      setResult(null);
    },
  });

  const isPending = scanFolderMutation.isPending || scanPathMutation.isPending;

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  const folders = rootFolders ?? [];

  const handleScanFolder = () => {
    if (selectedId === "") return;
    setResult(null);
    scanFolderMutation.mutate(selectedId);
  };

  const handleScanPath = () => {
    if (!path.trim()) return;
    setResult(null);
    scanPathMutation.mutate(path.trim());
  };

  const handlePickerSelect = (selectedPath: string) => {
    setPath(selectedPath);
    setShowPicker(false);
    setResult(null);
    scanPathMutation.mutate(selectedPath);
  };

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Unmapped Files</h1>
      </PageToolbar>

      <PageContent>
        {/* Mode tabs */}
        <div className="mb-4 flex gap-1 rounded bg-zinc-800/50 p-1 w-fit">
          <button
            onClick={() => setMode("path")}
            className={`rounded px-3 py-1.5 text-xs font-medium ${
              mode === "path" ? "bg-zinc-700 text-zinc-100" : "text-muted hover:text-zinc-300"
            }`}
          >
            Browse Path
          </button>
          <button
            onClick={() => setMode("rootfolder")}
            className={`rounded px-3 py-1.5 text-xs font-medium ${
              mode === "rootfolder" ? "bg-zinc-700 text-zinc-100" : "text-muted hover:text-zinc-300"
            }`}
          >
            Root Folder
          </button>
        </div>

        {/* Path mode */}
        {mode === "path" && (
          <div className="mb-6 space-y-3">
            <div className="flex items-center gap-2">
              <input
                type="text"
                value={path}
                onChange={(e) => { setPath(e.target.value); setPathError(""); }}
                onKeyDown={(e) => e.key === "Enter" && handleScanPath()}
                placeholder="/path/to/scan"
                aria-label="Path to scan"
                className={`flex-1 rounded border px-3 py-2 text-sm bg-zinc-800 text-zinc-200 font-mono ${
                  pathError ? "border-red-500" : "border-border"
                }`}
              />
              <button
                onClick={() => setShowPicker(!showPicker)}
                className="btn-secondary inline-flex items-center gap-1.5 text-sm"
              >
                <FolderSearch size={14} />
                Browse
              </button>
              <button
                onClick={handleScanPath}
                disabled={!path.trim() || isPending}
                className="btn-primary inline-flex items-center gap-1.5 text-sm"
              >
                {isPending ? <LoadingSpinner size={14} /> : <Search size={14} />}
                Scan
              </button>
            </div>
            {pathError && <p className="text-sm text-red-400">{pathError}</p>}
            {showPicker && (
              <PathPicker
                initialPath={path || "/"}
                onSelect={handlePickerSelect}
                onClose={() => setShowPicker(false)}
              />
            )}
          </div>
        )}

        {/* Root folder mode */}
        {mode === "rootfolder" && (
          <div className="mb-6">
            {folders.length === 0 ? (
              <EmptyState
                icon={<Settings size={40} />}
                title="Configure a root folder in Settings first"
                action={
                  <Link
                    to="/settings/media-management"
                    className="btn-primary text-sm"
                  >
                    Go to Settings
                  </Link>
                }
              />
            ) : (
              <div className="flex items-center gap-3">
                <select
                  value={selectedId}
                  onChange={(e) =>
                    setSelectedId(e.target.value ? Number(e.target.value) : "")
                  }
                  aria-label="Select root folder"
                  className="rounded border border-border bg-zinc-800 px-3 py-1.5 text-sm text-zinc-200"
                >
                  <option value="">Select root folder...</option>
                  {folders.map((f) => (
                    <option key={f.id} value={f.id}>
                      {f.path} ({f.mediaType})
                    </option>
                  ))}
                </select>
                <button
                  onClick={handleScanFolder}
                  disabled={selectedId === "" || isPending}
                  className="btn-primary inline-flex items-center gap-1.5 text-sm"
                >
                  <FolderSearch size={14} />
                  Scan
                </button>
              </div>
            )}
          </div>
        )}

        {/* Loading */}
        {isPending && (
          <div className="flex items-center justify-center py-16">
            <LoadingSpinner size={32} />
          </div>
        )}

        {/* Results */}
        {!isPending && result && (
          <div className="space-y-6">
            <section>
              <h2 className="mb-2 text-sm font-semibold text-zinc-100">
                Matched ({result.matched})
              </h2>
              {result.matched === 0 ? (
                <p className="text-sm text-muted">No matched files found.</p>
              ) : (
                <p className="text-sm text-zinc-300">
                  {result.matched} file{result.matched !== 1 && "s"} matched to
                  existing works.
                </p>
              )}
            </section>

            <section>
              <h2 className="mb-2 text-sm font-semibold text-zinc-100">
                Unmatched ({result.unmatched.length})
              </h2>
              {result.unmatched.length === 0 ? (
                <p className="text-sm text-muted">No unmatched files.</p>
              ) : (
                <ul className="space-y-1">
                  {result.unmatched.map((f, i) => (
                    <li
                      key={i}
                      className="flex items-center gap-3 rounded bg-zinc-800/50 px-3 py-2 text-sm"
                    >
                      <span className="rounded bg-zinc-700 px-1.5 py-0.5 text-xs text-zinc-300">
                        {f.mediaType}
                      </span>
                      <span className="truncate text-zinc-300" title={f.path}>
                        {f.path}
                      </span>
                    </li>
                  ))}
                </ul>
              )}
            </section>

            {result.errors.length > 0 && (
              <section>
                <h2 className="mb-2 text-sm font-semibold text-red-400">
                  Errors ({result.errors.length})
                </h2>
                <ul className="space-y-1">
                  {result.errors.map((e, i) => (
                    <li
                      key={i}
                      className="rounded border border-red-900/50 bg-red-900/10 px-3 py-2 text-sm"
                    >
                      <p className="font-mono text-xs text-zinc-400">
                        {e.path}
                      </p>
                      <p className="mt-0.5 text-red-300">{e.message}</p>
                    </li>
                  ))}
                </ul>
              </section>
            )}
          </div>
        )}

        {!isPending && !result && mode === "path" && (
          <EmptyState title="Enter a path or browse to scan for unmapped files" />
        )}
      </PageContent>
    </>
  );
}
