import { useState, useEffect } from "react";
import { Link } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Loader2,
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  CheckCircle2,
  XCircle,
  Undo2,
  ExternalLink,
  Info,
} from "lucide-react";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import * as api from "@/api";
import { formatBytes, formatRelativeDate } from "@/utils/format";
import { cn } from "@/utils/cn";
import { HelpTip } from "@/components/HelpTip";
import type {
  ReadarrRootFolder,
  ImportPreviewResponse,
  ImportPreviewFileItem,
  ImportProgressResponse,
  ImportHistoryItem,
} from "@/types/api";

type Phase =
  | "idle"
  | "connecting"
  | "connected"
  | "previewing"
  | "previewed"
  | "importing"
  | "completed"
  | "failed";

export default function ReadarrImportPage() {
  const queryClient = useQueryClient();

  // Connection form
  const [url, setUrl] = useState("");
  const [apiKey, setApiKey] = useState("");

  // State machine
  const [phase, setPhase] = useState<Phase>("idle");

  // Connected state
  const [readarrFolders, setReadarrFolders] = useState<ReadarrRootFolder[]>([]);
  const [selectedReadarrFolder, setSelectedReadarrFolder] = useState<number | null>(null);
  const [selectedLivrarrFolder, setSelectedLivrarrFolder] = useState<number | null>(null);

  // Import mode
  const [importMode, setImportMode] = useState<"all" | "files_only">("files_only");

  // Path translation
  const [containerPath, setContainerPath] = useState("");
  const [hostPath, setHostPath] = useState("");

  // Preview
  const [preview, setPreview] = useState<ImportPreviewResponse | null>(null);
  const [skippedExpanded, setSkippedExpanded] = useState(false);

  // Progress
  const [progress, setProgress] = useState<ImportProgressResponse | null>(null);

  // Undo modal
  const [undoTarget, setUndoTarget] = useState<ImportHistoryItem | null>(null);

  // Prefill container path from selected Readarr root folder
  useEffect(() => {
    const folder = readarrFolders.find((f) => f.id === selectedReadarrFolder);
    if (folder) setContainerPath(folder.path);
  }, [selectedReadarrFolder, readarrFolders]);

  // Fetch Livrarr root folders
  const { data: livrarrFolders } = useQuery({
    queryKey: ["rootFolders"],
    queryFn: api.listRootFolders,
  });

  // Fetch import history
  const { data: history, refetch: refetchHistory } = useQuery({
    queryKey: ["readarrHistory"],
    queryFn: api.readarrHistory,
  });

  // Poll progress during import
  useEffect(() => {
    if (phase !== "importing") return;
    const interval = setInterval(async () => {
      try {
        const p = await api.readarrProgress();
        setProgress(p);
        if (!p.running && p.phase === "done") {
          const failed = p.errors.length > 0;
          setPhase(failed ? "failed" : "completed");
          if (failed) {
            toast.error(p.errors[0] ?? "Import failed");
          } else {
            toast.success("Import completed successfully");
          }
          refetchHistory();
        }
      } catch {
        // Transient error, keep polling
      }
    }, 2000);
    return () => clearInterval(interval);
  }, [phase, refetchHistory]);

  // Connect mutation
  const connectMut = useMutation({
    mutationFn: () => api.readarrConnect(url, apiKey),
    onMutate: () => setPhase("connecting"),
    onSuccess: (folders) => {
      const list = folders ?? [];
      setReadarrFolders(list);
      const firstReadarr = list[0];
      setSelectedReadarrFolder(firstReadarr ? firstReadarr.id : null);
      const firstLivrarr = livrarrFolders?.[0];
      if (firstLivrarr) {
        setSelectedLivrarrFolder(firstLivrarr.id);
      }
      setPhase("connected");
      toast.success("Connected to Readarr");
    },
    onError: (err: Error) => {
      setPhase("idle");
      toast.error(err.message || "Failed to connect to Readarr");
    },
  });

  // Preview mutation
  const previewMut = useMutation({
    mutationFn: () => {
      if (selectedReadarrFolder === null || selectedLivrarrFolder === null) {
        throw new Error("Select both root folders");
      }
      return api.readarrPreview({
        url,
        apiKey,
        readarrRootFolderId: selectedReadarrFolder,
        livrarrRootFolderId: selectedLivrarrFolder,
        filesOnly: importMode === "files_only",
        containerPath: containerPath || undefined,
        hostPath: hostPath || undefined,
      });
    },
    onMutate: () => setPhase("previewing"),
    onSuccess: (data) => {
      setPreview(data);
      setPhase("previewed");
    },
    onError: (err: Error) => {
      setPhase("connected");
      toast.error(err.message || "Failed to generate preview");
    },
  });

  // Start import mutation
  const startMut = useMutation({
    mutationFn: () => {
      if (selectedReadarrFolder === null || selectedLivrarrFolder === null) {
        throw new Error("Select both root folders");
      }
      return api.readarrStartImport({
        url,
        apiKey,
        readarrRootFolderId: selectedReadarrFolder,
        livrarrRootFolderId: selectedLivrarrFolder,
        filesOnly: importMode === "files_only",
        containerPath: containerPath || undefined,
        hostPath: hostPath || undefined,
      });
    },
    onSuccess: () => {
      setPhase("importing");
      setProgress(null);
    },
    onError: (err: Error) => {
      toast.error(err.message || "Failed to start import");
    },
  });

  // Undo mutation
  const undoMut = useMutation({
    mutationFn: (importId: string) => api.readarrUndo(importId),
    onSuccess: () => {
      toast.success("Import undone");
      refetchHistory();
      queryClient.invalidateQueries({ queryKey: ["works"] });
      queryClient.invalidateQueries({ queryKey: ["authors"] });
    },
    onError: (err: Error) => {
      toast.error(err.message || "Failed to undo import");
    },
  });

  // Build AI help link
  const buildHelpLink = () => {
    const readarrPaths = readarrFolders.map((f) => f.path).join(", ");
    const livrarrFolder = livrarrFolders?.find((f) => f.id === selectedLivrarrFolder);
    const livrarrPath = livrarrFolder?.path ?? "(not selected)";
    const question = `I need help configuring Docker volume mounts for Livrarr to access my Readarr library.\nReadarr root folders: ${readarrPaths || "(not connected yet)"}\nLivrarr root folder: ${livrarrPath}\nPlease show me the docker-compose volume configuration needed.`;
    return `/help?question=${encodeURIComponent(question)}`;
  };

  // Progress percentage
  const progressPct =
    progress && progress.filesTotal > 0
      ? Math.round(
          ((progress.authorsProcessed + progress.worksProcessed + progress.filesProcessed) /
            (progress.authorsTotal + progress.worksTotal + progress.filesTotal)) *
            100,
        )
      : 0;

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Readarr Import</h1>
      </PageToolbar>

      <PageContent>
        <div className="mx-auto max-w-3xl space-y-6">
          {/* Warning banner */}
          <div className="flex items-start gap-3 rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-3">
            <AlertTriangle className="mt-0.5 shrink-0 text-amber-400" size={18} />
            <div className="text-sm text-amber-200">
              <span className="font-medium">Experimental feature.</span> This imports your Readarr
              library into Livrarr. Your Readarr files and directories will never be modified.
            </div>
          </div>

          {/* Mount requirement notice */}
          <div className="flex items-start gap-3 rounded-lg border border-border bg-zinc-800/50 px-4 py-3">
            <Info className="mt-0.5 shrink-0 text-blue-400" size={18} />
            <div className="text-sm text-muted">
              Readarr's library must be accessible from Livrarr's filesystem. If using Docker,
              ensure volumes are mapped correctly.{" "}
              <Link
                to={buildHelpLink()}
                className="inline-flex items-center gap-1 text-brand hover:text-brand-hover"
              >
                Get AI help <ExternalLink size={12} />
              </Link>
            </div>
          </div>

          {/* Connection section */}
          <div className="rounded-lg border border-border bg-zinc-800/50 p-4">
            <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-muted">
              Connection
            </h2>
            <div className="space-y-3">
              <div>
                <label className="mb-1 block text-sm text-muted">Readarr URL</label>
                <input
                  type="text"
                  placeholder="http://localhost:8787"
                  value={url}
                  onChange={(e) => setUrl(e.target.value)}
                  disabled={phase !== "idle"}
                  className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-600 focus:border-brand focus:outline-none disabled:opacity-50"
                />
              </div>
              <div>
                <label className="mb-1 block text-sm text-muted">API Key</label>
                <input
                  type="password"
                  placeholder="Readarr API key"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  disabled={phase !== "idle"}
                  className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 placeholder:text-zinc-600 focus:border-brand focus:outline-none disabled:opacity-50"
                />
              </div>
              {phase === "idle" && (
                <button
                  onClick={() => connectMut.mutate()}
                  disabled={!url.trim() || !apiKey.trim()}
                  className="inline-flex items-center gap-2 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
                >
                  Connect
                </button>
              )}
              {phase === "connecting" && (
                <button disabled className="inline-flex items-center gap-2 rounded bg-brand px-4 py-2 text-sm font-medium text-white opacity-50">
                  <Loader2 size={14} className="animate-spin" />
                  Connecting...
                </button>
              )}
              {phase !== "idle" && phase !== "connecting" && (
                <div className="flex items-center gap-3">
                  <span className="inline-flex items-center gap-1.5 text-sm text-green-400">
                    <CheckCircle2 size={14} />
                    Connected
                  </span>
                  <button
                    onClick={() => {
                      setPhase("idle");
                      setReadarrFolders([]);
                      setSelectedReadarrFolder(null);
                      setPreview(null);
                      setProgress(null);
                    }}
                    className="text-sm text-muted hover:text-zinc-100"
                  >
                    Disconnect
                  </button>
                </div>
              )}
            </div>
          </div>

          {/* Root folder selection */}
          {phase !== "idle" && phase !== "connecting" && (
            <div className="rounded-lg border border-border bg-zinc-800/50 p-4">
              <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-muted">
                Root Folders
              </h2>
              <div className="space-y-3">
                <div>
                  <label className="mb-1 block text-sm text-muted">Readarr Root Folder</label>
                  <select
                    value={selectedReadarrFolder ?? ""}
                    onChange={(e) => setSelectedReadarrFolder(Number(e.target.value))}
                    disabled={phase === "previewing" || phase === "importing"}
                    className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                  >
                    {readarrFolders.map((f) => (
                      <option key={f.id} value={f.id}>
                        {f.path}
                        {f.freeSpace != null && f.totalSpace != null
                          ? ` (${formatBytes(f.freeSpace)} free / ${formatBytes(f.totalSpace)} total)`
                          : ""}
                      </option>
                    ))}
                  </select>
                </div>
                <div>
                  <label className="mb-1 block text-sm text-muted">Livrarr Root Folder</label>
                  <select
                    value={selectedLivrarrFolder ?? ""}
                    onChange={(e) => setSelectedLivrarrFolder(Number(e.target.value))}
                    disabled={phase === "previewing" || phase === "importing"}
                    className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                  >
                    {livrarrFolders?.map((f) => (
                      <option key={f.id} value={f.id}>
                        {f.path} ({f.mediaType})
                        {f.freeSpace != null && f.totalSpace != null
                          ? ` - ${formatBytes(f.freeSpace)} free / ${formatBytes(f.totalSpace)} total`
                          : ""}
                      </option>
                    ))}
                  </select>
                  {(!livrarrFolders || livrarrFolders.length === 0) && (
                    <p className="mt-1 text-xs text-amber-400">
                      No root folders configured in Livrarr.{" "}
                      <Link to="/settings/mediamanagement" className="text-brand hover:text-brand-hover">
                        Add one first.
                      </Link>
                    </p>
                  )}
                </div>

                {/* Path translation */}
                <div className="rounded border border-border bg-zinc-900/60 p-3">
                  <div className="mb-2 flex items-center gap-1.5">
                    <span className="text-sm font-medium text-zinc-300">Path translation</span>
                    <HelpTip text="When Readarr runs in Docker, its file paths reflect the container's internal filesystem (e.g. /books). Enter the equivalent path where Livrarr can access the same files on the host. Leave blank if both apps share the same filesystem." />
                    <Link
                      to={`/help?question=${encodeURIComponent("How do I configure Docker volume path translation for Readarr import in Livrarr? Readarr container path: " + (containerPath || "/books"))}`}
                      className="ml-auto text-xs text-brand hover:text-brand-hover"
                    >
                      AI help <ExternalLink size={10} className="inline" />
                    </Link>
                  </div>
                  <div className="grid grid-cols-2 gap-2">
                    <div>
                      <label className="mb-1 block text-xs text-muted">Container path</label>
                      <input
                        type="text"
                        value={containerPath}
                        onChange={(e) => { setContainerPath(e.target.value); setPreview(null); }}
                        disabled={phase === "previewing" || phase === "importing"}
                        placeholder="/books"
                        className="w-full rounded border border-border bg-zinc-800 px-2 py-1.5 text-xs text-zinc-100 placeholder:text-zinc-600 focus:border-brand focus:outline-none disabled:opacity-50"
                      />
                    </div>
                    <div>
                      <label className="mb-1 block text-xs text-muted">Host path</label>
                      <input
                        type="text"
                        value={hostPath}
                        onChange={(e) => { setHostPath(e.target.value); setPreview(null); }}
                        disabled={phase === "previewing" || phase === "importing"}
                        placeholder="/mnt/data/books"
                        className="w-full rounded border border-border bg-zinc-800 px-2 py-1.5 text-xs text-zinc-100 placeholder:text-zinc-600 focus:border-brand focus:outline-none disabled:opacity-50"
                      />
                    </div>
                  </div>
                </div>
                {(phase === "connected" || phase === "previewed" || phase === "completed" || phase === "failed") && (
                  <div className="space-y-3">
                    <div>
                      <label className="mb-1.5 block text-sm text-muted">Import scope</label>
                      <div className="flex flex-col gap-2">
                        {(["files_only", "all"] as const).map((mode) => (
                          <label key={mode} className="flex cursor-pointer items-start gap-2.5">
                            <input
                              type="radio"
                              name="importMode"
                              value={mode}
                              checked={importMode === mode}
                              onChange={() => { setImportMode(mode); setPreview(null); }}
                              disabled={phase === "previewing" || phase === "importing"}
                              className="mt-0.5 accent-brand"
                            />
                            <span className="text-sm">
                              {mode === "files_only" ? (
                                <>
                                  <span className="font-medium text-zinc-100">Works with files</span>
                                  <span className="ml-1 text-muted">— only import works that have book files in Readarr</span>
                                </>
                              ) : (
                                <>
                                  <span className="font-medium text-zinc-100">All works</span>
                                  <span className="ml-1 text-muted">— import entire Readarr library, including works without files</span>
                                </>
                              )}
                            </span>
                          </label>
                        ))}
                      </div>
                    </div>
                    <button
                      onClick={() => previewMut.mutate()}
                      disabled={selectedReadarrFolder === null || selectedLivrarrFolder === null}
                      className="inline-flex items-center gap-2 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
                    >
                      Preview Import
                    </button>
                  </div>
                )}
                {phase === "previewing" && (
                  <button disabled className="inline-flex items-center gap-2 rounded bg-brand px-4 py-2 text-sm font-medium text-white opacity-50">
                    <Loader2 size={14} className="animate-spin" />
                    Generating preview...
                  </button>
                )}
              </div>
            </div>
          )}

          {/* Preview section */}
          {preview && (phase === "previewed" || phase === "importing" || phase === "completed" || phase === "failed") && (
            <div className="rounded-lg border border-border bg-zinc-800/50 p-4">
              <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-muted">
                Import Preview
              </h2>

              {/* Summary stats */}
              <div className="mb-4 flex flex-wrap gap-4 text-sm">
                <span className="text-zinc-300">
                  <span className="font-semibold text-zinc-100">{preview.authorsToCreate}</span> new authors
                  {preview.authorsExisting > 0 && <span className="text-muted"> ({preview.authorsExisting} existing)</span>}
                </span>
                <span className="text-zinc-300">
                  <span className="font-semibold text-zinc-100">{preview.worksToCreate}</span> new works
                  {preview.worksExisting > 0 && <span className="text-muted"> ({preview.worksExisting} existing)</span>}
                </span>
                <span className="text-zinc-300">
                  <span className="font-semibold text-zinc-100">{preview.filesToImport}</span> files
                </span>
                {preview.filesToSkip > 0 && (
                  <span className="text-amber-400">
                    <span className="font-semibold">{preview.filesToSkip}</span> skipped
                  </span>
                )}
              </div>

              {/* File list */}
              {preview.importFiles.length > 0 && (
                <div className="mb-4 max-h-96 overflow-y-auto rounded border border-border">
                  <table className="w-full text-xs">
                    <thead className="sticky top-0 bg-zinc-800">
                      <tr className="border-b border-border text-left text-muted">
                        <th className="px-3 py-2">Title</th>
                        <th className="px-3 py-2">Author</th>
                        <th className="px-3 py-2">Format</th>
                        <th className="px-3 py-2">Status</th>
                        <th className="px-3 py-2">Action</th>
                      </tr>
                    </thead>
                    <tbody>
                      {preview.importFiles.map((item, i) => (
                        <PreviewFileRow key={i} item={item} />
                      ))}
                    </tbody>
                  </table>
                </div>
              )}

              {/* Skipped items */}
              {preview.skippedItems.length > 0 && (
                <div className="mb-4">
                  <button
                    onClick={() => setSkippedExpanded(!skippedExpanded)}
                    className="flex items-center gap-1.5 text-sm text-muted hover:text-zinc-100"
                  >
                    {skippedExpanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                    {preview.skippedItems.length} skipped item{preview.skippedItems.length !== 1 ? "s" : ""}
                  </button>
                  {skippedExpanded && (
                    <div className="mt-2 max-h-48 overflow-y-auto rounded border border-border bg-zinc-900 p-3">
                      <div className="space-y-1.5">
                        {preview.skippedItems.map((item, i) => (
                          <div key={i} className="flex items-start gap-2 text-xs">
                            <XCircle size={12} className="mt-0.5 shrink-0 text-red-400" />
                            <span className="text-zinc-300">{item.title}</span>
                            {item.author && <span className="text-zinc-500">{item.author}</span>}
                            <span className="text-muted">— {item.reason}</span>
                          </div>
                        ))}
                      </div>
                    </div>
                  )}
                </div>
              )}

              {/* Confirm button */}
              {phase === "previewed" && (
                <button
                  onClick={() => startMut.mutate()}
                  disabled={startMut.isPending || preview.filesToImport === 0}
                  className="inline-flex items-center gap-2 rounded bg-green-600 px-4 py-2 text-sm font-medium text-white hover:bg-green-700 disabled:opacity-50"
                >
                  {startMut.isPending ? (
                    <>
                      <Loader2 size={14} className="animate-spin" />
                      Starting...
                    </>
                  ) : (
                    `Confirm & Import ${preview.filesToImport} file${preview.filesToImport !== 1 ? "s" : ""}`
                  )}
                </button>
              )}
            </div>
          )}

          {/* Progress section */}
          {(phase === "importing" || phase === "completed" || phase === "failed") && progress && (
            <div className="rounded-lg border border-border bg-zinc-800/50 p-4">
              <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-muted">
                Import Progress
              </h2>

              {/* Progress bar */}
              <div className="mb-4 h-3 overflow-hidden rounded-full bg-zinc-700">
                <div
                  className={cn(
                    "h-full rounded-full transition-all duration-300",
                    phase === "failed" ? "bg-red-500" : phase === "completed" ? "bg-green-500" : "bg-brand",
                  )}
                  style={{ width: `${progressPct}%` }}
                />
              </div>

              {/* Status line */}
              <div className="mb-3 flex items-center gap-2 text-sm">
                {phase === "importing" && <Loader2 size={14} className="animate-spin text-brand" />}
                {phase === "completed" && <CheckCircle2 size={14} className="text-green-400" />}
                {phase === "failed" && <XCircle size={14} className="text-red-400" />}
                <span className="text-zinc-200">
                  {phase === "importing" && "Importing..."}
                  {phase === "completed" && "Import completed"}
                  {phase === "failed" && (progress.errors[0] ?? "Import failed")}
                </span>
                <span className="ml-auto text-muted">{progressPct}%</span>
              </div>

              {/* Counters */}
              <div className="grid grid-cols-2 gap-2 text-xs sm:grid-cols-4">
                <ProgressCounter label="Authors" done={progress.authorsProcessed} total={progress.authorsTotal} />
                <ProgressCounter label="Works" done={progress.worksProcessed} total={progress.worksTotal} />
                <ProgressCounter label="Files" done={progress.filesProcessed} total={progress.filesTotal} />
                <ProgressCounter label="Skipped" done={progress.filesSkipped} total={null} />
              </div>
            </div>
          )}

          {/* Import history */}
          {history && history.length > 0 && (
            <div className="rounded-lg border border-border bg-zinc-800/50 p-4">
              <h2 className="mb-4 text-sm font-semibold uppercase tracking-wider text-muted">
                Import History
              </h2>
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b border-border text-left text-xs text-muted">
                      <th className="pb-2 pr-3">Date</th>
                      <th className="pb-2 pr-3">Source</th>
                      <th className="pb-2 pr-3">Status</th>
                      <th className="pb-2 pr-3 text-right">Authors</th>
                      <th className="pb-2 pr-3 text-right">Works</th>
                      <th className="pb-2 pr-3 text-right">Files</th>
                      <th className="pb-2" />
                    </tr>
                  </thead>
                  <tbody>
                    {history.map((item) => (
                      <tr key={item.id} className="border-b border-border/50 last:border-0">
                        <td className="py-2 pr-3 text-zinc-300">
                          {formatRelativeDate(item.startedAt)}
                        </td>
                        <td className="py-2 pr-3 text-zinc-400 max-w-[200px] truncate">
                          {item.sourceUrl ?? item.source}
                        </td>
                        <td className="py-2 pr-3">
                          <StatusBadge status={item.status} />
                        </td>
                        <td className="py-2 pr-3 text-right text-zinc-300">
                          {item.authorsCreated}
                        </td>
                        <td className="py-2 pr-3 text-right text-zinc-300">
                          {item.worksCreated}
                        </td>
                        <td className="py-2 pr-3 text-right text-zinc-300">
                          {item.filesImported}
                          {item.filesSkipped > 0 && (
                            <span className="text-muted"> ({item.filesSkipped} skipped)</span>
                          )}
                        </td>
                        <td className="py-2 text-right">
                          {item.status !== "running" && item.status !== "undone" && (
                            <button
                              onClick={() => setUndoTarget(item)}
                              disabled={undoMut.isPending}
                              className="inline-flex items-center gap-1 rounded px-2 py-1 text-xs text-muted hover:bg-zinc-700 hover:text-zinc-100 disabled:opacity-50"
                            >
                              <Undo2 size={12} />
                              Undo
                            </button>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>
      </PageContent>

      {/* Undo confirmation modal */}
      <ConfirmModal
        open={!!undoTarget}
        onOpenChange={(open) => !open && setUndoTarget(null)}
        title="Undo Import"
        description={`This will remove ${undoTarget?.authorsCreated ?? 0} authors, ${undoTarget?.worksCreated ?? 0} works, and ${undoTarget?.filesImported ?? 0} files created by this import. This cannot be reversed.`}
        confirmLabel="Undo Import"
        variant="danger"
        onConfirm={async () => {
          if (undoTarget) {
            await undoMut.mutateAsync(undoTarget.id);
            setUndoTarget(null);
          }
        }}
      />
    </>
  );
}

function PreviewFileRow({ item }: { item: ImportPreviewFileItem }) {
  return (
    <tr className="border-b border-border/40 last:border-0 hover:bg-zinc-700/30">
      <td className="max-w-[200px] truncate px-3 py-1.5 text-zinc-200" title={item.title}>
        {item.title}
      </td>
      <td className="max-w-[160px] truncate px-3 py-1.5 text-zinc-400" title={item.author}>
        {item.author}
      </td>
      <td className="px-3 py-1.5">
        <span className={cn(
          "inline-flex items-center rounded px-1.5 py-0.5 text-xs font-medium",
          item.mediaType === "audiobook"
            ? "bg-purple-500/15 text-purple-400"
            : "bg-blue-500/15 text-blue-400",
        )}>
          {item.mediaType}
        </span>
      </td>
      <td className="px-3 py-1.5">
        <span className={cn(
          "text-xs",
          item.workStatus === "new" ? "text-green-400" : "text-blue-400",
        )}>
          {item.workStatus === "new" ? "New" : "In Library"}
        </span>
      </td>
      <td className="px-3 py-1.5">
        <span className="text-xs text-green-400">
          {item.workStatus === "new" ? "Create & Import" : "Add file"}
        </span>
      </td>
    </tr>
  );
}

function ProgressCounter({
  label,
  done,
  total,
}: {
  label: string;
  done: number;
  total: number | null;
}) {
  return (
    <div className="rounded border border-border bg-zinc-900 px-2 py-1.5 text-center">
      <div className="font-medium text-zinc-100">
        {done}
        {total !== null && <span className="text-muted">/{total}</span>}
      </div>
      <div className="text-muted">{label}</div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const styles: Record<string, string> = {
    completed: "bg-green-500/15 text-green-400 border-green-500/30",
    failed: "bg-red-500/15 text-red-400 border-red-500/30",
    running: "bg-blue-500/15 text-blue-400 border-blue-500/30",
    undone: "bg-zinc-500/15 text-zinc-400 border-zinc-500/30",
  };
  return (
    <span
      className={cn(
        "inline-flex items-center rounded-full border px-2 py-0.5 text-xs font-medium",
        styles[status] ?? styles.failed,
      )}
    >
      {status}
    </span>
  );
}
