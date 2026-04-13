import { useState, useEffect, useCallback, useRef } from "react";
import { Document, Page, pdfjs } from "react-pdf";
import {
  getDownloadUrl,
  getPlaybackProgress,
  updatePlaybackProgress,
} from "@/api";
import {
  ArrowLeft,
  ChevronLeft,
  ChevronRight,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { useNavigate } from "react-router";

pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  /* @vite-ignore */ "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

const ZOOM_LEVELS = [0.75, 1.0, 1.5] as const;

interface Props {
  libraryItemId: number;
}

export function PdfReader({ libraryItemId }: Props) {
  const navigate = useNavigate();
  const [numPages, setNumPages] = useState(0);
  const [pageNumber, setPageNumber] = useState(1);
  const [zoomIdx, setZoomIdx] = useState(1);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Load saved progress.
  useEffect(() => {
    getPlaybackProgress(libraryItemId)
      .then((p) => {
        if (p?.position) {
          const pg = parseInt(p.position, 10);
          if (!isNaN(pg) && pg > 0) setPageNumber(pg);
        }
      })
      .catch(() => {});
  }, [libraryItemId]);

  const saveProgress = useCallback(
    (pg: number, total: number) => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        const pct = total > 0 ? pg / total : 0;
        updatePlaybackProgress(libraryItemId, String(pg), pct).catch(() => {});
      }, 2000);
    },
    [libraryItemId],
  );

  useEffect(() => {
    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    };
  }, []);

  const goToPage = useCallback(
    (pg: number) => {
      const clamped = Math.max(1, Math.min(pg, numPages));
      setPageNumber(clamped);
      saveProgress(clamped, numPages);
    },
    [numPages, saveProgress],
  );

  const url = getDownloadUrl(libraryItemId);
  const token = localStorage.getItem("livrarr_token") ?? "";

  // Fetch PDF as blob with auth headers.
  const [pdfData, setPdfData] = useState<ArrayBuffer | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    fetch(url, {
      headers: { Authorization: `Bearer ${token}` },
      signal: controller.signal,
    })
      .then((res) => res.arrayBuffer())
      .then(setPdfData)
      .catch(() => {});
    return () => controller.abort();
  }, [url, token]);

  return (
    <div className="flex h-screen flex-col bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 border-b border-zinc-700 bg-zinc-900 px-4 py-2">
        <button
          onClick={() => navigate(-1)}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title="Back"
        >
          <ArrowLeft size={20} />
        </button>

        <div className="flex items-center gap-2">
          <button
            onClick={() => goToPage(pageNumber - 1)}
            disabled={pageNumber <= 1}
            className="rounded p-1 text-zinc-400 hover:text-zinc-100 disabled:opacity-30"
          >
            <ChevronLeft size={18} />
          </button>
          <input
            type="number"
            min={1}
            max={numPages}
            value={pageNumber}
            onChange={(e) => goToPage(parseInt(e.target.value, 10) || 1)}
            className="w-14 rounded border border-zinc-600 bg-zinc-800 px-2 py-1 text-center text-sm text-zinc-200"
          />
          <span className="text-sm text-zinc-400">/ {numPages}</span>
          <button
            onClick={() => goToPage(pageNumber + 1)}
            disabled={pageNumber >= numPages}
            className="rounded p-1 text-zinc-400 hover:text-zinc-100 disabled:opacity-30"
          >
            <ChevronRight size={18} />
          </button>
        </div>

        <div className="flex-1" />

        <button
          onClick={() => setZoomIdx((i) => Math.max(0, i - 1))}
          disabled={zoomIdx <= 0}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100 disabled:opacity-30"
          title="Zoom out"
        >
          <ZoomOut size={16} />
        </button>
        <span className="text-xs text-zinc-400">
          {Math.round((ZOOM_LEVELS[zoomIdx] ?? 1) * 100)}%
        </span>
        <button
          onClick={() =>
            setZoomIdx((i) => Math.min(ZOOM_LEVELS.length - 1, i + 1))
          }
          disabled={zoomIdx >= ZOOM_LEVELS.length - 1}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100 disabled:opacity-30"
          title="Zoom in"
        >
          <ZoomIn size={16} />
        </button>
      </div>

      {/* PDF content */}
      <div className="flex-1 overflow-auto flex justify-center py-4">
        <Document
          file={pdfData ? { data: pdfData } : null}
          onLoadSuccess={({ numPages: n }) => setNumPages(n)}
          loading={
            <div className="text-zinc-400 py-8">Loading PDF...</div>
          }
          error={
            <div className="text-red-400 py-8">Failed to load PDF.</div>
          }
        >
          <Page
            pageNumber={pageNumber}
            scale={ZOOM_LEVELS[zoomIdx] ?? 1}
            renderTextLayer={true}
            renderAnnotationLayer={true}
          />
        </Document>
      </div>
    </div>
  );
}

export default PdfReader;
