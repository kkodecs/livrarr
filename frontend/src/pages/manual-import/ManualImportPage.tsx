import { useState, useRef, useEffect, useCallback } from "react";
import { Link } from "react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { FolderSearch, Upload, Search, AlertTriangle, Check, X } from "lucide-react";
import { scanManualImport, scanManualImportProgress, executeManualImport, searchManualImport } from "@/api";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { EmptyState } from "@/components/Page/EmptyState";
import { LoadingSpinner } from "@/components/Page/LoadingSpinner";
import { PathPicker } from "@/components/PathPicker/PathPicker";
import type { ScannedFile, OlMatch, ManualImportItem, ManualImportResult } from "@/types/api";

type FileState = ScannedFile & {
  selected: boolean;
  deleteExisting: boolean;
  importResult?: ManualImportResult;
  correctedMatch?: OlMatch;
};

export default function ManualImportPage() {
  const queryClient = useQueryClient();
  const [path, setPath] = useState("");
  const [pathError, setPathError] = useState("");
  const [showPicker, setShowPicker] = useState(false);
  const [files, setFiles] = useState<FileState[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [hasCorrections, setHasCorrections] = useState(false);
  const [hasScanned, setHasScanned] = useState(false);
  const [scanId, setScanId] = useState<string | null>(null);
  const [olTotal, setOlTotal] = useState(0);
  const [olCompleted, setOlCompleted] = useState(0);
  const pollRef = useRef<ReturnType<typeof setInterval>>(undefined);
  const [importTotal, setImportTotal] = useState(0);
  const [importCompleted, setImportCompleted] = useState(0);
  const [importing, setImporting] = useState(false);

  // Inline search state
  const [searchingIdx, setSearchingIdx] = useState<number | null>(null);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<OlMatch[]>([]);
  const [searchLoading, setSearchLoading] = useState(false);
  const [searchError, setSearchError] = useState(false);
  const searchTimeout = useRef<ReturnType<typeof setTimeout>>(undefined);
  const searchAbort = useRef<AbortController | null>(null);

  // Cleanup search timeout and abort controller on unmount
  useEffect(() => {
    return () => {
      if (searchTimeout.current) clearTimeout(searchTimeout.current);
      searchAbort.current?.abort();
    };
  }, []);

  // Helper: file is importable if it has parsed title+author OR an OL match.
  const isImportable = useCallback(
    (f: ScannedFile) =>
      !!(f.correctedMatch || f.match || (f.parsed?.title && f.parsed?.author)),
    [],
  );

  const scanMutation = useMutation({
    mutationFn: (p: string) => scanManualImport(p),
    onSuccess: (data) => {
      // Default selected if we have title+author (from parsing or OL).
      setFiles(
        data.files.map((f) => ({
          ...f,
          selected: !!(f.parsed?.title && f.parsed?.author) && !f.hasExistingMediaType,
          deleteExisting: false,
        })),
      );
      setWarnings(data.warnings);
      setPathError("");
      setHasScanned(true);
      setHasCorrections(false);
      setOlTotal(data.olTotal);
      setOlCompleted(data.olCompleted);

      // Start polling for OL progress if there are lookups to do.
      if (data.scanId && data.olTotal > 0) {
        setScanId(data.scanId);
        if (pollRef.current) clearInterval(pollRef.current);
        pollRef.current = setInterval(async () => {
          try {
            const progress = await scanManualImportProgress(data.scanId);
            setOlCompleted(progress.olCompleted);
            // Update files with new OL matches (preserve user selections).
            setFiles((prev) =>
              prev.map((f, i) => {
                const updated = progress.files[i];
                if (!updated) return f;
                // Deselect if OL discovered same media type already imported.
                const newlyDuplicate = !f.hasExistingMediaType && updated.hasExistingMediaType;
                return {
                  ...f,
                  match: updated.match,
                  existingWorkId: updated.existingWorkId,
                  hasExistingMediaType: updated.hasExistingMediaType,
                  selected: newlyDuplicate ? false : f.selected,
                };
              }),
            );
            if (progress.olCompleted >= progress.olTotal) {
              clearInterval(pollRef.current);
              pollRef.current = undefined;
              setScanId(null);
            }
          } catch {
            // Scan expired or server restarted — stop polling.
            clearInterval(pollRef.current);
            pollRef.current = undefined;
            setScanId(null);
          }
        }, 2000);
      }
    },
    onError: (e: Error) => {
      setPathError(e.message || "The file system path specified was not found.");
      setFiles([]);
      setWarnings([]);
      setHasScanned(false);
      setScanId(null);
      setOlTotal(0);
      setOlCompleted(0);
    },
  });

  // Cleanup polling on unmount.
  useEffect(() => {
    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, []);

  const runSequentialImport = async (groups: ManualImportItem[][]) => {
    setImporting(true);
    setImportTotal(groups.length);
    setImportCompleted(0);
    let totalImported = 0;
    let totalFailed = 0;

    for (let i = 0; i < groups.length; i++) {
      const group = groups[i]!;
      const groupPaths = new Set(group.map((g) => g.path));
      try {
        const data = await executeManualImport(group);
        const imported = data.results.filter((r) => r.status === "imported").length;
        const failed = data.results.filter((r) => r.status === "failed").length;
        totalImported += imported;
        totalFailed += failed;
        setFiles((prev) =>
          prev.map((f) => {
            const paths = f.groupedPaths ?? [f.path];
            if (paths.some((p) => groupPaths.has(p))) {
              const allResults = data.results.filter((r) => paths.includes(r.path));
              const allOk = allResults.length > 0 && allResults.every((r) => r.status === "imported");
              const anyFail = allResults.some((r) => r.status === "failed");
              return {
                ...f,
                importResult: {
                  path: f.path,
                  status: anyFail ? "failed" : allOk ? "imported" : "skipped",
                  workId: allResults[0]?.workId ?? null,
                  error: allResults.find((r) => r.status === "failed")?.error ?? null,
                } as ManualImportResult,
              };
            }
            return f;
          }),
        );
      } catch {
        totalFailed += group.length;
      }
      setImportCompleted(i + 1);
    }

    setImporting(false);
    queryClient.invalidateQueries({ queryKey: ["works"] });
    if (totalFailed > 0) {
      toast.warning(`${totalImported} imported, ${totalFailed} failed`);
    } else {
      toast.success(`${totalImported} file${totalImported !== 1 ? "s" : ""} imported`);
    }
  };

  const confirmAndScan = (scanPath: string) => {
    if (hasCorrections && files.length > 0) {
      if (!confirm("Re-scanning will clear your current matches. Continue?")) return;
    }
    scanMutation.mutate(scanPath);
  };

  const handleScan = () => {
    if (!path.trim()) return;
    confirmAndScan(path.trim());
  };

  const handlePickerSelect = (selectedPath: string) => {
    setPath(selectedPath);
    setShowPicker(false);
    confirmAndScan(selectedPath);
  };

  const isImported = (f: FileState) => f.importResult?.status === "imported";

  const handleSelectAll = (checked: boolean) => {
    setFiles((prev) =>
      prev.map((f) => ({
        ...f,
        selected: checked && hasMatch(f) && !isImported(f),
      })),
    );
  };

  const handleToggle = (idx: number) => {
    setFiles((prev) =>
      prev.map((f, i) =>
        i === idx && hasMatch(f) && !isImported(f) ? { ...f, selected: !f.selected } : f,
      ),
    );
  };

  const handleDeleteExisting = (idx: number, checked: boolean) => {
    setFiles((prev) =>
      prev.map((f, i) => (i === idx ? { ...f, deleteExisting: checked } : f)),
    );
  };

  const handleImport = () => {
    const selected = files.filter((f) => f.selected && hasMatch(f));
    const groups: ManualImportItem[][] = [];
    for (const f of selected) {
      const m = f.correctedMatch || f.match;
      const title = m?.title ?? f.parsed?.title ?? f.filename;
      const author = m?.author ?? f.parsed?.author ?? "";
      const olKey = m?.olKey ?? "";
      const language = f.parsed?.language || undefined;
      const paths = f.groupedPaths ?? [f.path];
      const group: ManualImportItem[] = paths.map((p) => ({
        path: p,
        olKey,
        title,
        author,
        deleteExisting: f.deleteExisting,
        language,
      }));
      groups.push(group);
    }
    runSequentialImport(groups);
  };

  const handleInlineSearch = (idx: number) => {
    setSearchingIdx(idx);
    const f = files[idx]!;
    const query = f.parsed ? `${f.parsed.title} ${f.parsed.author}` : f.filename;
    setSearchResults([]);
    setSearchError(false);
    handleSearchInput(query);
  };

  const handleSearchInput = (query: string) => {
    setSearchQuery(query);
    setSearchError(false);
    if (searchTimeout.current) clearTimeout(searchTimeout.current);
    searchAbort.current?.abort();
    if (query.trim().length < 2) return;
    searchTimeout.current = setTimeout(async () => {
      const controller = new AbortController();
      searchAbort.current = controller;
      setSearchLoading(true);
      try {
        const resp = await searchManualImport(query);
        if (!controller.signal.aborted) {
          setSearchResults(resp.results);
          setSearchError(false);
        }
      } catch {
        if (!controller.signal.aborted) {
          setSearchResults([]);
          setSearchError(true);
        }
      } finally {
        if (!controller.signal.aborted) {
          setSearchLoading(false);
        }
      }
    }, 400);
  };

  const handleCloseSearch = () => {
    setSearchingIdx(null);
    if (searchTimeout.current) clearTimeout(searchTimeout.current);
    searchAbort.current?.abort();
  };

  const handleSelectMatch = (idx: number, match: OlMatch) => {
    setFiles((prev) =>
      prev.map((f, i) =>
        i === idx
          ? {
              ...f,
              correctedMatch: match,
              existingWorkId: match.existingWorkId,
            }
          : f,
      ),
    );
    handleCloseSearch();
    setHasCorrections(true);
  };

  const hasMatch = (f: FileState) =>
    !!(f.correctedMatch || f.match || (f.parsed?.title && f.parsed?.author));

  const effectiveMatch = (f: FileState) => f.correctedMatch || f.match;

  const selectedCount = files.filter((f) => f.selected).length;
  const selectableCount = files.filter((f) => hasMatch(f) && !isImported(f)).length;
  const canImport = selectedCount > 0 && !importing;

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Manual Import</h1>
      </PageToolbar>

      <PageContent>
        {/* Path input */}
        <div className="mb-6 space-y-3">
          <div className="flex flex-col sm:flex-row items-stretch sm:items-center gap-2">
            <input
              type="text"
              value={path}
              onChange={(e) => { setPath(e.target.value); setPathError(""); }}
              onKeyDown={(e) => e.key === "Enter" && handleScan()}
              placeholder="/path/to/files"
              aria-label="Path to scan"
              className={`flex-1 rounded border px-3 py-2 text-sm bg-zinc-800 text-zinc-200 font-mono ${
                pathError ? "border-red-500" : "border-border"
              }`}
            />
            <div className="flex items-center gap-2">
              <button
                onClick={() => setShowPicker(!showPicker)}
                className="btn-secondary inline-flex items-center gap-1.5 text-sm"
              >
                <FolderSearch size={14} />
                Browse
              </button>
              <button
                onClick={handleScan}
                disabled={!path.trim() || scanMutation.isPending}
                className="btn-primary inline-flex items-center gap-1.5 text-sm"
              >
                {scanMutation.isPending ? <LoadingSpinner size={14} /> : <Search size={14} />}
                Scan
              </button>
            </div>
          </div>

          {pathError && (
            <p className="text-sm text-red-400">{pathError}</p>
          )}

          {showPicker && (
            <PathPicker
              initialPath={path || "/"}
              onSelect={handlePickerSelect}
              onClose={() => setShowPicker(false)}
            />
          )}
        </div>

        {/* Warnings */}
        {warnings.map((w, i) => (
          <div key={i} className="mb-3 flex items-center gap-2 rounded border border-yellow-900/50 bg-yellow-900/10 px-3 py-2 text-sm text-yellow-300">
            <AlertTriangle size={14} />
            {w}
          </div>
        ))}

        {/* Scanning spinner */}
        {scanMutation.isPending && (
          <div className="flex flex-col items-center justify-center py-16 gap-3">
            <LoadingSpinner size={32} />
            <p className="text-sm text-muted">Scanning files and parsing metadata...</p>
          </div>
        )}

        {/* OL progress bar */}
        {!scanMutation.isPending && olTotal > 0 && olCompleted < olTotal && (
          <div className="mb-3 space-y-1">
            <div className="flex items-center justify-between text-xs text-muted">
              <span>Matching against OpenLibrary...</span>
              <span>{olCompleted}/{olTotal}</span>
            </div>
            <div className="h-1.5 rounded-full bg-zinc-800 overflow-hidden">
              <div
                className="h-full rounded-full bg-brand transition-all duration-300"
                style={{ width: `${(olCompleted / olTotal) * 100}%` }}
              />
            </div>
          </div>
        )}

        {/* Results table */}
        {!scanMutation.isPending && files.length > 0 && (
          <>
            <div className="mb-3 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-2">
              <span className="text-sm text-muted">
                {files.length} file{files.length !== 1 ? "s" : ""} found
                {selectedCount > 0 && ` · ${selectedCount} selected`}
              </span>
              <button
                onClick={handleImport}
                disabled={!canImport}
                className="btn-primary inline-flex items-center gap-1.5 text-sm"
              >
                {importing ? (
                  <LoadingSpinner size={14} />
                ) : (
                  <Upload size={14} />
                )}
                Import Selected
              </button>
            </div>

            {importing && importTotal > 0 && (
              <div className="mb-3 space-y-1">
                <div className="flex items-center justify-between text-xs text-muted">
                  <span>Importing and enriching...</span>
                  <span>{importCompleted}/{importTotal}</span>
                </div>
                <div className="h-1.5 rounded-full bg-zinc-800 overflow-hidden">
                  <div
                    className="h-full rounded-full bg-brand transition-all duration-300"
                    style={{ width: `${(importCompleted / importTotal) * 100}%` }}
                  />
                </div>
              </div>
            )}

            <div className="overflow-x-auto rounded border border-border">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
                    <th className="px-3 py-2 w-8">
                      <input
                        type="checkbox"
                        aria-label="Select all files"
                        checked={selectedCount === selectableCount && selectableCount > 0}
                        onChange={(e) => handleSelectAll(e.target.checked)}
                      />
                    </th>
                    <th className="px-3 py-2">File</th>
                    <th className="hidden md:table-cell px-3 py-2">Type</th>
                    <th className="px-3 py-2">Match</th>
                    <th className="hidden sm:table-cell px-3 py-2">Status</th>
                  </tr>
                </thead>
                <tbody>
                  {files.map((f, idx) => {
                    const match = effectiveMatch(f);
                    const workId = f.existingWorkId || match?.existingWorkId;
                    const hasWork = !!workId;
                    const hasSameMedia = f.hasExistingMediaType;
                    const result = f.importResult;
                    const imported = isImported(f);

                    return (
                      <tr
                        key={f.path}
                        className={`border-b border-border/50 ${
                          result?.status === "failed" ? "bg-red-900/10" : ""
                        } ${result?.status === "imported" ? "bg-green-900/10" : ""}`}
                      >
                        {/* Checkbox */}
                        <td className="px-3 py-2">
                          <input
                            type="checkbox"
                            aria-label={`Select ${f.filename}`}
                            checked={f.selected}
                            disabled={!hasMatch(f) || imported}
                            onChange={() => handleToggle(idx)}
                          />
                        </td>

                        {/* Filename */}
                        <td className="px-3 py-2">
                          <div className="font-mono text-xs text-zinc-300 truncate max-w-xs" title={f.filename}>
                            {f.filename}
                          </div>
                          {f.parsed && (
                            <div className="text-xs text-muted mt-0.5">
                              {f.parsed.author} — {f.parsed.title}
                              {f.parsed.series && (
                                <span className="text-zinc-500">
                                  {" "}({f.parsed.series}
                                  {f.parsed.seriesPosition && ` #${f.parsed.seriesPosition}`})
                                </span>
                              )}
                              {f.parsed.language && (
                                <span className="ml-1 text-zinc-500 uppercase">
                                  ({f.parsed.language.slice(0, 2).toUpperCase()})
                                </span>
                              )}
                            </div>
                          )}
                        </td>

                        {/* Media type */}
                        <td className="hidden md:table-cell px-3 py-2">
                          <span className="rounded bg-zinc-700 px-1.5 py-0.5 text-xs text-zinc-300">
                            {f.mediaType}
                          </span>
                          {!f.routable && (
                            <span className="ml-1 text-yellow-400" title="No root folder configured for this media type">
                              <AlertTriangle size={12} className="inline" />
                            </span>
                          )}
                        </td>

                        {/* Match */}
                        <td className="px-3 py-2">
                          {match ? (
                            <div>
                              <button
                                onClick={() => handleInlineSearch(idx)}
                                className="text-left text-xs text-blue-400 hover:underline"
                                disabled={imported}
                              >
                                {match.title} — {match.author}
                              </button>
                              {hasSameMedia && workId && (
                                <Link
                                  to={`/work/${workId}`}
                                  className="ml-1.5 rounded bg-yellow-900/50 px-1.5 py-0.5 text-xs text-yellow-300 hover:underline"
                                >
                                  duplicate
                                </Link>
                              )}
                              {f.selected && hasSameMedia && (
                                <label className="mt-1 flex items-center gap-1.5 text-xs text-muted">
                                  <input
                                    type="checkbox"
                                    checked={f.deleteExisting}
                                    onChange={(e) =>
                                      handleDeleteExisting(idx, e.target.checked)
                                    }
                                  />
                                  Delete existing release(s)
                                </label>
                              )}
                            </div>
                          ) : f.parsed?.title && f.parsed?.author ? (
                            <div>
                              <button
                                onClick={() => handleInlineSearch(idx)}
                                className="text-left text-xs text-zinc-400 hover:underline"
                                disabled={imported}
                                title="From filename — click to search OL"
                              >
                                {f.parsed.title} — {f.parsed.author}
                              </button>
                              <span className="ml-1 text-[10px] text-zinc-600">(parsed)</span>
                              {hasSameMedia && workId && (
                                <Link
                                  to={`/work/${workId}`}
                                  className="ml-1.5 rounded bg-yellow-900/50 px-1.5 py-0.5 text-xs text-yellow-300 hover:underline"
                                >
                                  duplicate
                                </Link>
                              )}
                            </div>
                          ) : (
                            <button
                              onClick={() => handleInlineSearch(idx)}
                              className="text-xs text-muted hover:text-zinc-200"
                              disabled={imported}
                            >
                              Search...
                            </button>
                          )}

                          {/* Inline search dropdown */}
                          {searchingIdx === idx && (
                            <div className="mt-2 rounded border border-border bg-zinc-900 p-2 shadow-lg">
                              <input
                                type="text"
                                value={searchQuery}
                                onChange={(e) => handleSearchInput(e.target.value)}
                                className="w-full rounded border border-border bg-zinc-800 px-2 py-1 text-xs text-zinc-200"
                                placeholder="Search Open Library..."
                                aria-label="Search Open Library"
                                autoFocus
                              />
                              <div className="mt-1 max-h-40 overflow-y-auto">
                                {searchLoading && (
                                  <p className="py-2 text-center text-xs text-muted">
                                    Searching...
                                  </p>
                                )}
                                {searchError && (
                                  <p className="py-2 text-center text-xs text-red-400">
                                    Search failed — try again
                                  </p>
                                )}
                                {!searchLoading && !searchError && searchResults.length === 0 && searchQuery.length >= 2 && (
                                  <p className="py-2 text-center text-xs text-muted">
                                    No results
                                  </p>
                                )}
                                {searchResults.map((r, ri) => (
                                  <button
                                    key={ri}
                                    onClick={() => handleSelectMatch(idx, r)}
                                    className="flex w-full items-center gap-2 rounded px-2 py-1 text-left text-xs text-zinc-300 hover:bg-zinc-800"
                                  >
                                    <span className="truncate">
                                      {r.title} — {r.author}
                                    </span>
                                    {r.existingWorkId && (
                                      <span className="shrink-0 rounded bg-yellow-900/50 px-1 text-yellow-300">
                                        in library
                                      </span>
                                    )}
                                  </button>
                                ))}
                              </div>
                              <button
                                onClick={handleCloseSearch}
                                className="mt-1 w-full text-center text-xs text-muted hover:text-zinc-200"
                              >
                                Close
                              </button>
                            </div>
                          )}
                        </td>

                        {/* Status */}
                        <td className="hidden sm:table-cell px-3 py-2">
                          {result ? (
                            <div>
                              {result.status === "imported" && (
                                <span className="inline-flex items-center gap-1 text-xs text-green-400">
                                  <Check size={12} />
                                  <Link
                                    to={`/work/${result.workId}`}
                                    className="hover:underline"
                                  >
                                    Imported
                                  </Link>
                                </span>
                              )}
                              {result.status === "skipped" && (
                                <span className="text-xs text-muted">Skipped</span>
                              )}
                              {result.status === "failed" && (
                                <span className="inline-flex items-center gap-1 text-xs text-red-400">
                                  <X size={12} />
                                  {result.error}
                                </span>
                              )}
                            </div>
                          ) : !f.routable ? (
                            <Link
                              to="/settings/media-management"
                              target="_blank"
                              className="text-xs text-blue-400 hover:underline"
                            >
                              Configure root folder
                            </Link>
                          ) : null}
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          </>
        )}

        {/* Empty state */}
        {!scanMutation.isPending && files.length === 0 && !pathError && (
          <EmptyState
            icon={<FolderSearch size={40} />}
            title={hasScanned ? "No recognized media files found" : "Enter a path or browse to scan for media files"}
          />
        )}
      </PageContent>
    </>
  );
}
