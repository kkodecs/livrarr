import { useState, useCallback, useRef } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  Upload,
  Loader2,
  CheckCircle2,
  XCircle,
  AlertCircle,
  Undo2,
  BookOpen,
  FileText,
} from "lucide-react";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import * as api from "@/api";
import { formatRelativeDate } from "@/utils/format";
import { cn } from "@/utils/cn";
import type {
  ListImportPreviewResponse,
  ListImportConfirmRowResult,
} from "@/types/api";

type Phase = "idle" | "uploading" | "previewed" | "confirming" | "done";

const BATCH_SIZE = 10;

export default function ListImportPage() {
  const queryClient = useQueryClient();
  const fileInputRef = useRef<HTMLInputElement>(null);

  const [phase, setPhase] = useState<Phase>("idle");
  const [preview, setPreview] = useState<ListImportPreviewResponse | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [progress, setProgress] = useState({ done: 0, total: 0 });
  const [results, setResults] = useState<ListImportConfirmRowResult[]>([]);
  const [currentImportId, setCurrentImportId] = useState<string | null>(null);
  const [undoTarget, setUndoTarget] = useState<string | null>(null);

  // Fetch import history for undo.
  const { data: history, refetch: refetchHistory } = useQuery({
    queryKey: ["list-import-history"],
    queryFn: api.listImportHistory,
  });

  // File drop handler.
  const handleFile = useCallback(async (file: File) => {
    if (!file.name.endsWith(".csv")) {
      toast.error("Please upload a CSV file");
      return;
    }
    setPhase("uploading");
    try {
      const resp = await api.listImportPreview(file);
      setPreview(resp);
      // Auto-select "new" rows, deselect others.
      const sel = new Set<number>();
      for (const row of resp.rows) {
        if (row.previewStatus === "new") sel.add(row.rowIndex);
      }
      setSelected(sel);
      setPhase("previewed");
    } catch (e: unknown) {
      toast.error(e instanceof Error ? e.message : "Upload failed");
      setPhase("idle");
    }
  }, []);

  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const file = e.dataTransfer.files[0];
      if (file) handleFile(file);
    },
    [handleFile]
  );

  const onFileSelect = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      const file = e.target.files?.[0];
      if (file) handleFile(file);
    },
    [handleFile]
  );

  // Toggle row selection.
  const toggleRow = (idx: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      return next;
    });
  };

  const toggleAll = () => {
    if (!preview) return;
    const newRows = preview.rows.filter((r) => r.previewStatus === "new");
    if (selected.size === newRows.length) {
      setSelected(new Set());
    } else {
      setSelected(new Set(newRows.map((r) => r.rowIndex)));
    }
  };

  // Batch confirm.
  const handleConfirm = async () => {
    if (!preview || selected.size === 0) return;
    setPhase("confirming");
    setResults([]);

    const indices = Array.from(selected);
    setProgress({ done: 0, total: indices.length });

    let importId: string | undefined;
    const allResults: ListImportConfirmRowResult[] = [];

    for (let i = 0; i < indices.length; i += BATCH_SIZE) {
      const batch = indices.slice(i, i + BATCH_SIZE);
      try {
        const resp = await api.listImportConfirm({
          previewId: preview.previewId,
          rowIndices: batch,
          importId,
        });
        importId = resp.importId;
        allResults.push(...resp.results);
        setProgress({ done: Math.min(i + BATCH_SIZE, indices.length), total: indices.length });
        setResults([...allResults]);
      } catch (e: unknown) {
        toast.error(e instanceof Error ? e.message : "Import batch failed");
        break;
      }
    }

    // Mark complete.
    if (importId) {
      try {
        await api.listImportComplete(importId);
      } catch {
        // Non-critical — import still worked.
      }
      setCurrentImportId(importId);
    }

    setPhase("done");
    queryClient.invalidateQueries({ queryKey: ["works"] });
    refetchHistory();

    const added = allResults.filter((r) => r.status === "added").length;
    const exists = allResults.filter((r) => r.status === "already_exists").length;
    const failed = allResults.filter(
      (r) => r.status === "add_failed" || r.status === "lookup_error"
    ).length;
    toast.success(`Import complete: ${added} added, ${exists} existing, ${failed} failed`);
  };

  // Undo.
  const handleUndo = async (importId: string) => {
    try {
      const resp = await api.listImportUndo(importId);
      toast.success(`Removed ${resp.worksRemoved} imported works`);
      queryClient.invalidateQueries({ queryKey: ["works"] });
      refetchHistory();
      setUndoTarget(null);
      if (currentImportId === importId) {
        reset();
      }
    } catch (e: unknown) {
      toast.error(e instanceof Error ? e.message : "Undo failed");
    }
  };

  const reset = () => {
    setPhase("idle");
    setPreview(null);
    setSelected(new Set());
    setResults([]);
    setCurrentImportId(null);
    setProgress({ done: 0, total: 0 });
  };

  // Status badge for preview rows.
  const statusBadge = (status: string) => {
    switch (status) {
      case "new":
        return <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-green-500/20 text-green-400"><CheckCircle2 size={12} /> New</span>;
      case "already_exists":
        return <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-muted text-muted-foreground"><AlertCircle size={12} /> Exists</span>;
      case "parse_error":
        return <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-red-500/20 text-red-400"><XCircle size={12} /> Error</span>;
      default:
        return null;
    }
  };

  const confirmStatusIcon = (status: string) => {
    switch (status) {
      case "added":
        return <CheckCircle2 size={14} className="text-green-400" />;
      case "already_exists":
        return <AlertCircle size={14} className="text-muted-foreground" />;
      default:
        return <XCircle size={14} className="text-red-400" />;
    }
  };

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-foreground">List Import</h1>
      </PageToolbar>
      <PageContent>
        {/* Import history (undo) */}
        {history && history.filter((h) => h.status !== "undone").length > 0 && (
          <div className="mb-6 p-4 rounded-lg border border-border bg-card">
            <h3 className="text-sm font-medium text-foreground mb-3">Recent Imports</h3>
            <div className="space-y-2">
              {history
                .filter((h) => h.status !== "undone")
                .slice(0, 5)
                .map((imp) => (
                  <div key={imp.id} className="flex items-center justify-between text-sm">
                    <div className="flex items-center gap-2">
                      <span className="capitalize text-muted-foreground">{imp.source}</span>
                      <span className="text-foreground">{imp.worksCreated} works</span>
                      <span className="text-muted-foreground">{formatRelativeDate(imp.startedAt)}</span>
                      <span className={cn(
                        "text-xs px-1.5 py-0.5 rounded",
                        imp.status === "completed" ? "bg-green-500/20 text-green-400" :
                        imp.status === "running" ? "bg-yellow-500/20 text-yellow-400" :
                        "bg-muted text-muted-foreground"
                      )}>{imp.status}</span>
                    </div>
                    <button
                      onClick={() => setUndoTarget(imp.id)}
                      className="flex items-center gap-1 text-xs text-red-400 hover:text-red-300"
                    >
                      <Undo2 size={12} /> Undo
                    </button>
                  </div>
                ))}
            </div>
          </div>
        )}

        {/* Phase: idle — source selection + file drop */}
        {phase === "idle" && (
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            {[
              {
                name: "Goodreads",
                icon: BookOpen,
                instructions: "Export your library from goodreads.com/review/import — click \"Export Library\" at the top.",
              },
              {
                name: "Hardcover",
                icon: FileText,
                instructions: "Export your library from hardcover.app/account/exports — download the CSV file.",
              },
            ].map((src) => (
              <div
                key={src.name}
                className="p-6 rounded-lg border-2 border-dashed border-border hover:border-primary/50 transition-colors cursor-pointer bg-card"
                onDragOver={(e) => e.preventDefault()}
                onDrop={onDrop}
                onClick={() => fileInputRef.current?.click()}
              >
                <div className="flex flex-col items-center gap-3 text-center">
                  <src.icon size={32} className="text-muted-foreground" />
                  <h3 className="text-lg font-medium text-foreground">{src.name}</h3>
                  <p className="text-sm text-muted-foreground">{src.instructions}</p>
                  <div className="flex items-center gap-2 mt-2 text-sm text-primary">
                    <Upload size={16} />
                    Drop CSV here or click to browse
                  </div>
                </div>
              </div>
            ))}
            <input
              ref={fileInputRef}
              type="file"
              accept=".csv"
              className="hidden"
              onChange={onFileSelect}
            />
          </div>
        )}

        {/* Phase: uploading */}
        {phase === "uploading" && (
          <div className="flex items-center justify-center gap-3 p-12 text-muted-foreground">
            <Loader2 size={20} className="animate-spin" />
            Parsing CSV...
          </div>
        )}

        {/* Phase: previewed — show table */}
        {phase === "previewed" && preview && (
          <div>
            <div className="flex items-center justify-between mb-4">
              <div>
                <h3 className="text-sm font-medium text-foreground">
                  {preview.totalRows} books from{" "}
                  <span className="capitalize">{preview.source}</span>
                </h3>
                <p className="text-xs text-muted-foreground mt-1">
                  Match status is based on local ISBN lookup — some books may resolve differently during import.
                  Reading status and ratings are shown for reference but are not imported yet.
                </p>
              </div>
              <div className="flex gap-2">
                <button onClick={reset} className="btn btn-secondary text-sm">
                  Cancel
                </button>
                <button
                  onClick={handleConfirm}
                  disabled={selected.size === 0}
                  className="btn btn-primary text-sm"
                >
                  Import {selected.size} Selected
                </button>
              </div>
            </div>

            <div className="border border-border rounded-lg overflow-hidden">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b border-border bg-muted/50">
                    <th className="p-3 w-8">
                      <input type="checkbox" checked={preview.rows.filter((r) => r.previewStatus === "new").length === selected.size && selected.size > 0} onChange={toggleAll} />
                    </th>
                    <th className="p-3 text-left text-muted-foreground font-medium">Title</th>
                    <th className="p-3 text-left text-muted-foreground font-medium">Author</th>
                    <th className="p-3 text-left text-muted-foreground font-medium">ISBN</th>
                    <th className="p-3 text-left text-muted-foreground font-medium">Status</th>
                    <th className="p-3 text-left text-muted-foreground font-medium">Rating</th>
                    <th className="p-3 text-left text-muted-foreground font-medium">Match</th>
                  </tr>
                </thead>
                <tbody>
                  {preview.rows.map((row) => (
                    <tr
                      key={row.rowIndex}
                      className={cn(
                        "border-b border-border last:border-0 hover:bg-muted/30",
                        row.previewStatus !== "new" && "opacity-50"
                      )}
                    >
                      <td className="p-3">
                        <input
                          type="checkbox"
                          checked={selected.has(row.rowIndex)}
                          onChange={() => toggleRow(row.rowIndex)}
                          disabled={row.previewStatus === "parse_error"}
                        />
                      </td>
                      <td className="p-3 text-foreground">{row.title || <span className="italic text-muted-foreground">—</span>}</td>
                      <td className="p-3 text-muted-foreground">{row.author}</td>
                      <td className="p-3 text-muted-foreground font-mono text-xs">{row.isbn13 || row.isbn10 || "—"}</td>
                      <td className="p-3 text-muted-foreground text-xs">{row.sourceStatus || "—"}</td>
                      <td className="p-3 text-muted-foreground">{row.sourceRating || "—"}</td>
                      <td className="p-3">{statusBadge(row.previewStatus)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}

        {/* Phase: confirming — progress */}
        {phase === "confirming" && (
          <div className="p-8">
            <div className="flex items-center gap-3 mb-4">
              <Loader2 size={20} className="animate-spin text-primary" />
              <span className="text-foreground">
                Importing... {progress.done} / {progress.total}
              </span>
            </div>
            <div className="w-full bg-muted rounded-full h-2">
              <div
                className="bg-primary h-2 rounded-full transition-all duration-300"
                style={{ width: `${progress.total ? (progress.done / progress.total) * 100 : 0}%` }}
              />
            </div>
            {results.length > 0 && (
              <div className="mt-4 max-h-64 overflow-y-auto space-y-1">
                {results.map((r) => (
                  <div key={r.rowIndex} className="flex items-center gap-2 text-sm">
                    {confirmStatusIcon(r.status)}
                    <span className="text-muted-foreground">Row {r.rowIndex + 1}:</span>
                    <span className="text-foreground">{r.status}</span>
                    {r.message && <span className="text-muted-foreground text-xs">({r.message})</span>}
                  </div>
                ))}
              </div>
            )}
          </div>
        )}

        {/* Phase: done — summary */}
        {phase === "done" && (
          <div className="p-8">
            <div className="flex items-center gap-3 mb-4">
              <CheckCircle2 size={24} className="text-green-400" />
              <h3 className="text-lg font-medium text-foreground">Import Complete</h3>
            </div>
            <div className="grid grid-cols-3 gap-4 mb-6">
              <div className="p-3 rounded bg-green-500/10 text-center">
                <div className="text-2xl font-bold text-green-400">
                  {results.filter((r) => r.status === "added").length}
                </div>
                <div className="text-xs text-muted-foreground">Added</div>
              </div>
              <div className="p-3 rounded bg-muted text-center">
                <div className="text-2xl font-bold text-muted-foreground">
                  {results.filter((r) => r.status === "already_exists").length}
                </div>
                <div className="text-xs text-muted-foreground">Already Existed</div>
              </div>
              <div className="p-3 rounded bg-red-500/10 text-center">
                <div className="text-2xl font-bold text-red-400">
                  {results.filter((r) => r.status === "add_failed" || r.status === "lookup_error").length}
                </div>
                <div className="text-xs text-muted-foreground">Failed</div>
              </div>
            </div>
            <div className="flex gap-2">
              <button onClick={reset} className="btn btn-primary text-sm">
                Import Another
              </button>
              {currentImportId && (
                <button
                  onClick={() => setUndoTarget(currentImportId)}
                  className="btn btn-secondary text-sm flex items-center gap-1 text-red-400"
                >
                  <Undo2 size={14} /> Undo This Import
                </button>
              )}
            </div>
          </div>
        )}

        {/* Undo confirmation modal */}
        <ConfirmModal
          open={!!undoTarget}
          onOpenChange={(open) => !open && setUndoTarget(null)}
          title="Undo Import"
          description="This will remove all works added by this import. This cannot be undone."
          confirmLabel="Undo Import"
          variant="danger"
          onConfirm={() => { if (undoTarget) handleUndo(undoTarget); }}
        />
      </PageContent>
    </>
  );
}
