import { useState, useRef, useCallback, useEffect } from "react";
import { ReactReader, ReactReaderStyle } from "react-reader";
type Rendition = any;
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  getDownloadUrl,
  getPlaybackProgress,
  getCrossFormatAnchors,
  getCrossFormatPrompt,
  declineCrossFormat,
  syncCrossFormatToHere,
} from "@/api";
import { ApiError, apiFetch } from "@/api/client";
import { resolveTsForCfi } from "@/utils/kashAnchors";
import { ResumePromptBanner } from "@/components/ResumePromptBanner";
import { savePosition } from "./savePosition";
import {
  ArrowLeft,
  List,
  Settings,
  Maximize2,
  Minimize2,
  Bookmark,
  Pencil,
  X,
  AlertTriangle,
  RefreshCw,
} from "lucide-react";
import { useNavigate } from "react-router";
import * as Popover from "@radix-ui/react-popover";
import { cn } from "@/utils/cn";
import type { BookmarkResponse, CreateBookmarkRequest, ResumePromptDTO } from "@/types/api";

interface TocItem {
  label: string;
  href: string;
  subitems?: TocItem[];
}

const FONT_FAMILIES: Record<string, string> = {
  serif: "'Georgia', 'Times New Roman', serif",
  sans: "'Inter', 'Helvetica Neue', sans-serif",
  mono: "'JetBrains Mono', 'Courier New', monospace",
};

interface Props {
  libraryItemId: number;
}

/** The session note shown after the user chose to start without the saved place. */
export const FROM_START_NOTE =
  "Your saved place could not be loaded. Progress is not being saved this time.";

/** The detail line of the saved-place error screen. */
export const SAVED_PLACE_DETAIL = "Your saved place could not be loaded.";

/** The outcome of reading a saved place. */
export type SavedPlaceRead =
  | { kind: "place"; position: string }
  | { kind: "none" }
  | { kind: "failed" };

/**
 * Reads the saved place of a library item. A 404, or an empty position,
 * means nothing has been saved yet. Any other error, and any reply without a
 * position string, is a failed read.
 */
export function readSavedPlace(libraryItemId: number): Promise<SavedPlaceRead> {
  return getPlaybackProgress(libraryItemId).then(
    (reply): SavedPlaceRead => {
      const position: unknown =
        reply && typeof reply === "object"
          ? (reply as { position?: unknown }).position
          : undefined;
      if (typeof position !== "string") return { kind: "failed" };
      return position === "" ? { kind: "none" } : { kind: "place", position };
    },
    (err): SavedPlaceRead =>
      err instanceof ApiError && err.status === 404
        ? { kind: "none" }
        : { kind: "failed" },
  );
}

/**
 * Where the saved place of the item on screen stands: still loading, failed,
 * known (applied to the location), or skipped by "Read from the beginning".
 */
type SavedPlaceStatus = "loading" | "failed" | "known" | "fromStart";

export function EpubReader({ libraryItemId }: Props) {
  const navigate = useNavigate();
  const containerRef = useRef<HTMLDivElement>(null);
  const renditionRef = useRef<Rendition | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [location, setLocation] = useState<string | number>(0);

  // Each opening of an item in this reader has its own id, including a
  // return to an item opened before. Results of an earlier opening never
  // count for the current one.
  const [opening, setOpening] = useState({ itemId: libraryItemId, id: 0 });
  if (opening.itemId !== libraryItemId)
    setOpening({ itemId: libraryItemId, id: opening.id + 1 });
  const openingId = opening.itemId === libraryItemId ? opening.id : opening.id + 1;

  const [loadAttempt, setLoadAttempt] = useState(0);
  // The download's outcome for one opening and attempt; `data` is null when it failed.
  const [download, setDownload] = useState<{
    opening: number;
    attempt: number;
    data: ArrayBuffer | null;
  } | null>(null);
  const [readAttempt, setReadAttempt] = useState(0);
  const [savedPlace, setSavedPlace] = useState<{
    opening: number;
    attempt: number;
    status: SavedPlaceStatus;
  } | null>(null);
  // The opening whose saved place is known, so its position may be saved.
  // Unset while the place is loading or failed and in a from-the-beginning
  // session; `saveProgress` sends nothing unless it is the current opening.
  const savePermitRef = useRef<number | null>(null);
  // Identifies the saved-place read in force; a reply for an older one is ignored.
  const readTokenRef = useRef<object>({});

  const currentDownload =
    download?.opening === openingId && download.attempt === loadAttempt
      ? download
      : null;
  const epubData = currentDownload?.data ?? null;
  const loadFailed = currentDownload !== null && currentDownload.data === null;
  const placeStatus: SavedPlaceStatus =
    savedPlace?.opening === openingId && savedPlace.attempt === readAttempt
      ? savedPlace.status
      : "loading";
  const placeReady = placeStatus === "known" || placeStatus === "fromStart";

  // Settings (persisted to localStorage)
  const [darkTheme, setDarkTheme] = useState(() =>
    localStorage.getItem("epub_theme") !== "light",
  );
  const [fontSize, setFontSize] = useState(() =>
    Number(localStorage.getItem("epub_font_size") ?? "110"),
  );
  const [fontFamily, setFontFamily] = useState<string>(() =>
    localStorage.getItem("epub_font_family") ?? "serif",
  );

  // TOC
  const [tocItems, setTocItems] = useState<TocItem[]>([]);
  const [tocOpen, setTocOpen] = useState(false);

  // Fullscreen
  const [isFullscreen, setIsFullscreen] = useState(false);

  // Bookmarks
  const queryClient = useQueryClient();
  const { data: bookmarks = [] } = useQuery<BookmarkResponse[]>({
    queryKey: ["bookmarks", libraryItemId],
    queryFn: () => apiFetch(`/workfile/${libraryItemId}/bookmarks`),
  });
  const [bookmarkPanelOpen, setBookmarkPanelOpen] = useState(false);

  const createBookmarkMut = useMutation({
    mutationFn: (req: CreateBookmarkRequest) =>
      apiFetch(`/workfile/${libraryItemId}/bookmarks`, {
        method: "POST",
        body: JSON.stringify(req),
      }),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["bookmarks", libraryItemId] }),
    onError: () => toast.error("Could not add the bookmark"),
  });

  const deleteBookmarkMut = useMutation({
    mutationFn: (id: number) =>
      apiFetch(`/bookmarks/${id}`, { method: "DELETE" }),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["bookmarks", libraryItemId] }),
    onError: () => toast.error("Could not delete the bookmark"),
  });

  const renameBookmarkMut = useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) =>
      apiFetch(`/bookmarks/${id}`, {
        method: "PATCH",
        body: JSON.stringify({ name }),
      }),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: ["bookmarks", libraryItemId] }),
    onError: () => toast.error("Could not rename the bookmark"),
  });
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");

  const [currentPct, setCurrentPct] = useState(0);

  // Cross-format resume
  const jumpUntilRef = useRef<number>(Date.now() + 3000);
  const currentCfiRef = useRef<string>("");
  const promptFiredRef = useRef(false);
  const [firstCfiKnown, setFirstCfiKnown] = useState(false);
  const [resumePrompt, setResumePrompt] = useState<ResumePromptDTO | null>(null);

  const { data: anchors } = useQuery({
    queryKey: ["cross-format-anchors", libraryItemId],
    queryFn: () => getCrossFormatAnchors(libraryItemId),
    retry: false,
  });
  // Ref mirror so the relocated closure always sees the latest anchors without re-registering.
  const anchorsRef = useRef(anchors);
  useEffect(() => {
    anchorsRef.current = anchors;
  }, [anchors]);

  // Persist settings
  useEffect(() => {
    localStorage.setItem("epub_theme", darkTheme ? "dark" : "light");
  }, [darkTheme]);
  useEffect(() => {
    localStorage.setItem("epub_font_size", String(fontSize));
  }, [fontSize]);
  useEffect(() => {
    localStorage.setItem("epub_font_family", fontFamily);
  }, [fontFamily]);

  // Fetch EPUB as ArrayBuffer with auth headers.
  const url = getDownloadUrl(libraryItemId);
  const token = localStorage.getItem("livrarr_token") ?? "";
  useEffect(() => {
    const controller = new AbortController();
    const opening = openingId;
    const attempt = loadAttempt;
    fetch(url, {
      headers: { Authorization: `Bearer ${token}` },
      signal: controller.signal,
    })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.arrayBuffer();
      })
      .then((data) => {
        if (!controller.signal.aborted) setDownload({ opening, attempt, data });
      })
      .catch(() => {
        if (!controller.signal.aborted)
          setDownload({ opening, attempt, data: null });
      });
    return () => controller.abort();
  }, [openingId, url, token, loadAttempt]);

  // Read the saved place once per opening and once per Retry. The book
  // opens only once it is known; a failed read shows the saved-place error.
  useEffect(() => {
    const readToken = {};
    readTokenRef.current = readToken;
    savePermitRef.current = null;
    promptFiredRef.current = false;
    currentCfiRef.current = "";
    setFirstCfiKnown(false);
    setResumePrompt(null);
    setCurrentPct(0);
    const opening = openingId;
    const attempt = readAttempt;
    readSavedPlace(libraryItemId).then((read) => {
      if (readTokenRef.current !== readToken) return;
      if (read.kind === "failed") {
        setSavedPlace({ opening, attempt, status: "failed" });
        return;
      }
      setLocation(read.kind === "place" ? read.position : 0);
      setSavedPlace({ opening, attempt, status: "known" });
      savePermitRef.current = opening;
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- libraryItemId changes only together with openingId
  }, [openingId, readAttempt]);

  // Opens the book as for "no saved place" and saves no position for the rest of this session.
  const readFromBeginning = () => {
    savePermitRef.current = null;
    setLocation(0);
    setSavedPlace({
      opening: openingId,
      attempt: readAttempt,
      status: "fromStart",
    });
  };

  // Save progress with trailing debounce. This is the reader's only save
  // door: it sends nothing unless the current opening's saved place is
  // known, checked when the save is asked for and again when it is sent, so
  // never while the place is loading or failed or in a from-the-beginning
  // session.
  const saveProgress = useCallback(
    (
      cfi: string,
      pct: number,
      kind?: "progress" | "seek",
      crossFormatTs?: number,
    ) => {
      if (savePermitRef.current !== openingId) return;
      const opening = openingId;
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        if (savePermitRef.current !== opening) return;
        savePosition(libraryItemId, cfi, pct, kind, crossFormatTs);
      }, 2000);
    },
    [libraryItemId, openingId],
  );

  useEffect(() => {
    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    };
  }, []);

  // Cross-format prompt: fire once per opened item after loaded + anchors available + first CFI known.
  useEffect(() => {
    if (!placeReady || !anchors || !firstCfiKnown || promptFiredRef.current) return;
    const cfi = currentCfiRef.current;
    if (!cfi) return;
    promptFiredRef.current = true;
    const currentTs = resolveTsForCfi(anchors, cfi);
    const readToken = readTokenRef.current;
    getCrossFormatPrompt(libraryItemId, currentTs)
      .then((dto) => {
        if (dto && readTokenRef.current === readToken) setResumePrompt(dto);
      })
      .catch(() => {});
  }, [placeReady, anchors, firstCfiKnown, libraryItemId]);

  const onLocationChanged = useCallback(
    (loc: string) => {
      setLocation(loc);
    },
    [],
  );

  const applyTheme = useCallback(
    (rendition: Rendition) => {
      rendition.themes.override("color", darkTheme ? "#e4e4e7" : "#1c1917");
      rendition.themes.override(
        "background-color",
        darkTheme ? "#18181b" : "#fafaf9",
      );
      rendition.themes.override("font-size", `${fontSize}%`);
      rendition.themes.override(
        "font-family",
        FONT_FAMILIES[fontFamily] ?? FONT_FAMILIES.serif,
      );
    },
    [darkTheme, fontSize, fontFamily],
  );

  useEffect(() => {
    if (renditionRef.current) applyTheme(renditionRef.current);
  }, [applyTheme]);

  // Fullscreen
  const toggleFullscreen = useCallback(() => {
    if (!document.fullscreenElement) {
      containerRef.current?.requestFullscreen();
    } else {
      document.exitFullscreen();
    }
  }, []);

  useEffect(() => {
    const handler = () => setIsFullscreen(!!document.fullscreenElement);
    document.addEventListener("fullscreenchange", handler);
    return () => document.removeEventListener("fullscreenchange", handler);
  }, []);

  // Navigate forward, handling the cover page where rendition.next() is a no-op.
  // When react-reader is at numeric location 0 (cover), epub.js hasn't resolved
  // a CFI yet so currentLocation() is undefined and next() silently does nothing.
  // Force-display the first spine section, which gives epub.js a real CFI, then
  // call next() to advance past it.
  const goNext = useCallback(() => {
    const rendition = renditionRef.current;
    if (!rendition) return;

    if (location === 0) {
      const spine = rendition.book?.spine;
      if (spine) {
        // Display the first LINEAR spine section so epub.js resolves a CFI.
        // A linear="no" head item (cover) accepts display() but next() from
        // it is a no-op — jumping past it IS the advance, so only chain
        // next() when the first linear item is also the spine head.
        const items =
          (spine as unknown as { items?: Array<{ href?: string; linear?: boolean | string }> })
            .items ?? [];
        const firstLinear = items.find(
          (it) => it.linear !== false && it.linear !== "no",
        );
        const first = spine.get(0);
        if (firstLinear?.href && first && firstLinear.href !== first.href) {
          rendition.display(firstLinear.href);
          return;
        }
        if (first) {
          rendition.display(first.href).then(() => rendition.next());
          return;
        }
      }
    }
    rendition.next();
  }, [location]);

  // Keyboard shortcuts
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (
        e.target instanceof HTMLInputElement ||
        e.target instanceof HTMLTextAreaElement
      )
        return;

      switch (e.key) {
        case "ArrowLeft":
          renditionRef.current?.prev();
          break;
        case "ArrowRight":
          goNext();
          break;
        case "f":
          if (!e.ctrlKey && !e.metaKey) toggleFullscreen();
          break;
        case "t":
          if (!e.ctrlKey && !e.metaKey) setTocOpen((v) => !v);
          break;
        case "d":
          if (!e.ctrlKey && !e.metaKey) setDarkTheme((v) => !v);
          break;
        case "Escape":
          if (tocOpen) setTocOpen(false);
          break;
        case "+":
        case "=":
          setFontSize((s) => Math.min(s + 10, 200));
          break;
        case "-":
          setFontSize((s) => Math.max(s - 10, 80));
          break;
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [toggleFullscreen, tocOpen, goNext]);

  if (loadFailed) {
    return (
      <BookLoadError
        onRetry={() => {
          setLoadAttempt((n) => n + 1);
          if (placeStatus === "failed") setReadAttempt((n) => n + 1);
        }}
      />
    );
  }

  if (placeStatus === "failed" && epubData) {
    return (
      <BookLoadError
        detail={SAVED_PLACE_DETAIL}
        onRetry={() => setReadAttempt((n) => n + 1)}
        secondaryAction={{
          label: "Read from the beginning",
          onClick: readFromBeginning,
        }}
      />
    );
  }

  if (!placeReady || !epubData) {
    return (
      <div className="flex h-screen items-center justify-center bg-zinc-900 text-zinc-400">
        Loading...
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="flex h-screen flex-col"
      style={{
        background: darkTheme ? "#18181b" : "#fafaf9",
        color: darkTheme ? "#e4e4e7" : "#1c1917",
      }}
    >
      {/* Toolbar */}
      <div
        className={cn(
          "flex items-center gap-3 border-b px-4 py-2",
          darkTheme
            ? "border-zinc-700 bg-zinc-900 text-zinc-100"
            : "border-zinc-300 bg-zinc-100 text-zinc-900",
        )}
      >
        <button
          onClick={() => navigate(-1)}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title="Back"
        >
          <ArrowLeft size={20} />
        </button>
        <button
          onClick={() => setTocOpen(!tocOpen)}
          className={cn(
            "rounded p-1 hover:text-zinc-100",
            tocOpen ? "text-zinc-100" : "text-zinc-400",
          )}
          title="Table of contents (T)"
        >
          <List size={20} />
        </button>
        {currentPct > 0 && (
          <span className={cn(
            "text-xs tabular-nums",
            darkTheme ? "text-zinc-500" : "text-zinc-400",
          )}>
            {Math.round(currentPct * 100)}%
          </span>
        )}
        <div className="flex-1 min-w-0">
          {placeStatus === "fromStart" && (
            <p className="text-xs text-amber-400">{FROM_START_NOTE}</p>
          )}
        </div>

        {/* Settings popover */}
        <Popover.Root>
          <Popover.Trigger asChild>
            <button
              className="rounded p-1 text-zinc-400 hover:text-zinc-100"
              title="Settings"
            >
              <Settings size={16} />
            </button>
          </Popover.Trigger>
          <Popover.Content
            className="w-56 rounded-lg border border-zinc-700 bg-zinc-900 p-4 shadow-xl z-50"
            sideOffset={8}
            align="end"
          >
            {/* Theme */}
            <label className="block text-xs text-zinc-400 mb-1">Theme</label>
            <div className="flex gap-1 mb-3">
              {(["dark", "light"] as const).map((t) => (
                <button
                  key={t}
                  onClick={() => setDarkTheme(t === "dark")}
                  className={cn(
                    "flex-1 rounded px-2 py-1.5 text-xs capitalize",
                    (t === "dark") === darkTheme
                      ? "bg-zinc-700 text-zinc-100"
                      : "bg-zinc-800 text-zinc-400 hover:text-zinc-200",
                  )}
                >
                  {t}
                </button>
              ))}
            </div>

            {/* Font size slider */}
            <label className="block text-xs text-zinc-400 mb-1">
              Font Size ({fontSize}%)
            </label>
            <input
              type="range"
              min={80}
              max={200}
              step={5}
              value={fontSize}
              onChange={(e) => setFontSize(Number(e.target.value))}
              className="w-full accent-brand mb-3"
            />

            {/* Font family */}
            <label className="block text-xs text-zinc-400 mb-1">Font</label>
            <div className="flex gap-1">
              {(["serif", "sans", "mono"] as const).map((f) => (
                <button
                  key={f}
                  onClick={() => setFontFamily(f)}
                  className={cn(
                    "flex-1 rounded px-2 py-1.5 text-xs capitalize",
                    fontFamily === f
                      ? "bg-zinc-700 text-zinc-100"
                      : "bg-zinc-800 text-zinc-400 hover:text-zinc-200",
                  )}
                  style={{ fontFamily: FONT_FAMILIES[f] }}
                >
                  {f}
                </button>
              ))}
            </div>

            {/* Sync to here — visible only when cross-format anchors are loaded */}
            {anchors && (
              <>
                <div className="my-3 border-t border-zinc-700" />
                <button
                  onClick={() => {
                    const cfi = currentCfiRef.current;
                    if (!cfi) return;
                    syncCrossFormatToHere(
                      libraryItemId,
                      resolveTsForCfi(anchors, cfi),
                    ).catch(() => {});
                    toast("Position synced");
                  }}
                  className="w-full rounded px-3 py-1.5 text-left text-xs text-zinc-300 hover:bg-zinc-800"
                >
                  Sync to here
                </button>
              </>
            )}

            <Popover.Arrow className="fill-zinc-700" />
          </Popover.Content>
        </Popover.Root>

        {/* Bookmark button */}
        <button
          onClick={() => {
            const cfi = typeof location === "string" ? location : "";
            const pct = currentPct;
            const name = `${Math.round(pct * 100)}%`;
            createBookmarkMut.mutate({
              position: cfi,
              sortKey: pct,
              name,
              chapterTitle: null,
            });
          }}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title="Add bookmark"
        >
          <Bookmark size={16} />
        </button>
        <button
          onClick={() => setBookmarkPanelOpen(!bookmarkPanelOpen)}
          className={cn(
            "rounded px-2 py-1 text-xs",
            bookmarkPanelOpen
              ? "text-brand"
              : "text-zinc-400 hover:text-zinc-100",
          )}
          title="Bookmarks"
        >
          {bookmarks.length} bookmarks
        </button>

        <button
          onClick={toggleFullscreen}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title={isFullscreen ? "Exit fullscreen (F)" : "Fullscreen (F)"}
        >
          {isFullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
        </button>
      </div>

      {/* Main area */}
      <div className="relative flex-1">
        {/* TOC sidebar */}
        {tocOpen && (
          <div className="absolute inset-0 z-40 flex">
            <div className="w-72 bg-zinc-900 border-r border-zinc-700 overflow-y-auto p-4">
              <div className="flex items-center justify-between mb-4">
                <h2 className="text-sm font-semibold text-zinc-200">
                  Contents
                </h2>
                <button
                  onClick={() => setTocOpen(false)}
                  className="text-zinc-400 hover:text-zinc-100"
                >
                  <X size={16} />
                </button>
              </div>
              {tocItems.length === 0 && (
                <p className="text-xs text-zinc-500">
                  No table of contents available.
                </p>
              )}
              {tocItems.map((item, i) => (
                <TocEntry
                  key={i}
                  item={item}
                  onNavigate={(href) => {
                    jumpUntilRef.current = Date.now() + 1500;
                    setLocation(href);
                    setTocOpen(false);
                  }}
                />
              ))}
            </div>
            <div
              className="flex-1 bg-black/50"
              onClick={() => setTocOpen(false)}
            />
          </div>
        )}

        {/* Bookmark panel */}
        {bookmarkPanelOpen && (
          <div className="absolute inset-0 z-40 flex justify-end">
            <div
              className="flex-1 bg-black/50"
              onClick={() => setBookmarkPanelOpen(false)}
            />
            <div className="w-72 bg-zinc-900 border-l border-zinc-700 overflow-y-auto p-4">
              <div className="flex items-center justify-between mb-4">
                <h2 className="text-sm font-semibold text-zinc-200">
                  Bookmarks
                </h2>
                <button
                  onClick={() => setBookmarkPanelOpen(false)}
                  className="text-zinc-400 hover:text-zinc-100"
                >
                  <X size={16} />
                </button>
              </div>
              {bookmarks.length === 0 ? (
                <p className="text-xs text-zinc-500">No bookmarks yet</p>
              ) : (
                bookmarks.map((bm) => (
                  <div
                    key={bm.id}
                    className="flex items-center gap-2 px-2 py-2 rounded hover:bg-zinc-800 group cursor-pointer"
                    onClick={() => {
                      if (renamingId !== bm.id && bm.position) {
                        jumpUntilRef.current = Date.now() + 1500;
                        setLocation(bm.position);
                      }
                    }}
                  >
                    <div className="flex-1 min-w-0">
                      {renamingId === bm.id ? (
                        <form
                          onSubmit={(e) => {
                            e.preventDefault();
                            renameBookmarkMut.mutate({
                              id: bm.id,
                              name: renameValue,
                            });
                            setRenamingId(null);
                          }}
                          onClick={(e) => e.stopPropagation()}
                        >
                          <input
                            autoFocus
                            className="w-full bg-zinc-800 border border-zinc-600 rounded px-2 py-0.5 text-sm text-zinc-100"
                            value={renameValue}
                            onChange={(e) => setRenameValue(e.target.value)}
                            onBlur={() => setRenamingId(null)}
                            onKeyDown={(e) => {
                              if (e.key === "Escape") setRenamingId(null);
                            }}
                          />
                        </form>
                      ) : (
                        <>
                          <p className="text-sm text-zinc-200 truncate">
                            {bm.name}
                          </p>
                          {bm.chapterTitle && (
                            <p className="text-xs text-zinc-500 truncate">
                              {bm.chapterTitle}
                            </p>
                          )}
                        </>
                      )}
                    </div>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        setRenamingId(bm.id);
                        setRenameValue(bm.name);
                      }}
                      className="opacity-0 group-hover:opacity-100 text-zinc-500 hover:text-zinc-200"
                      title="Rename"
                    >
                      <Pencil size={12} />
                    </button>
                    <button
                      onClick={(e) => {
                        e.stopPropagation();
                        deleteBookmarkMut.mutate(bm.id);
                      }}
                      className="opacity-0 group-hover:opacity-100 text-zinc-500 hover:text-red-400"
                    >
                      <X size={14} />
                    </button>
                  </div>
                ))
              )}
            </div>
          </div>
        )}

        {/* Reader */}
        <ReactReader
          url={epubData}
          location={location}
          locationChanged={onLocationChanged}
          tocChanged={(toc: TocItem[]) => {
            setTocItems(
              toc.map((item) => ({
                label: item.label?.trim() ?? "",
                href: item.href,
                subitems: (item.subitems as TocItem[]) ?? [],
              })),
            );
          }}
          getRendition={(rendition: Rendition) => {
            renditionRef.current = rendition;
            applyTheme(rendition);
            rendition.on("relocated", (loc: { start: { cfi: string; percentage: number } }) => {
              const pct = loc.start.percentage ?? 0;
              setCurrentPct(pct);
              if (pct > 0) {
                const cfi = loc.start.cfi;
                if (!currentCfiRef.current) {
                  // First relocation — signal the prompt effect.
                  setFirstCfiKnown(true);
                }
                currentCfiRef.current = cfi;
                const kind = Date.now() < jumpUntilRef.current ? "seek" : "progress";
                const currentAnchors = anchorsRef.current;
                const crossFormatTs = currentAnchors
                  ? resolveTsForCfi(currentAnchors, cfi)
                  : undefined;
                saveProgress(cfi, pct, kind, crossFormatTs);
              }
            });
            rendition.book.ready.then(() => {
              // A spine that opens with non-linear front matter (<itemref
              // linear="no"> cover) strands epub.js at location 0: next()
              // from a non-linear section is a no-op, and the built-in
              // arrows call next() directly. With no saved position, open
              // at the first LINEAR spine item instead. A saved-progress
              // CFI landing before or after this wins either way — both
              // paths go through setLocation and this only replaces 0.
              const items =
                (
                  rendition.book.spine as unknown as {
                    items?: Array<{ href?: string; linear?: boolean | string }>;
                  }
                ).items ?? [];
              const firstLinear = items.find(
                (it) => it.linear !== false && it.linear !== "no",
              );
              const linearHref = firstLinear?.href;
              if (linearHref) {
                setLocation((current) => (current === 0 ? linearHref : current));
              }
              const key = `livrarr-locations-${libraryItemId}`;
              const stored = localStorage.getItem(key);
              if (stored) {
                rendition.book.locations.load(stored);
                return;
              }
              return rendition.book.locations.generate(1600).then(() => {
                localStorage.setItem(key, rendition.book.locations.save());
              });
            });
          }}
          readerStyles={darkTheme ? {
            ...ReactReaderStyle,
            container: { ...ReactReaderStyle.container, background: "#18181b" },
            readerArea: { ...ReactReaderStyle.readerArea, background: "#18181b" },
            reader: { ...ReactReaderStyle.reader, background: "#18181b" },
            arrow: { ...ReactReaderStyle.arrow, color: "#a1a1aa" },
            arrowHover: { ...ReactReaderStyle.arrowHover, color: "#e4e4e7" },
            tocArea: { ...ReactReaderStyle.tocArea, background: "#18181b" },
            tocButton: { ...ReactReaderStyle.tocButton, color: "#a1a1aa" },
            tocButtonExpanded: { ...ReactReaderStyle.tocButtonExpanded, background: "#27272a" },
          } : undefined}
        />
      </div>

      {/* Reading progress bar */}
      {currentPct > 0 && (
        <div className={cn(
          "h-1 w-full",
          darkTheme ? "bg-zinc-800" : "bg-zinc-200",
        )}>
          <div
            className="h-full bg-brand transition-all duration-300"
            style={{ width: `${currentPct * 100}%` }}
          />
        </div>
      )}

      {/* Cross-format resume banner */}
      {resumePrompt && (
        <ResumePromptBanner
          label={resumePrompt.label}
          onJump={() => {
            jumpUntilRef.current = Date.now() + 1500;
            setLocation(resumePrompt.position);
            setResumePrompt(null);
          }}
          onStay={() => {
            declineCrossFormat(libraryItemId).catch(() => {});
            setResumePrompt(null);
          }}
        />
      )}
    </div>
  );
}

function TocEntry({
  item,
  onNavigate,
  depth = 0,
}: {
  item: TocItem;
  onNavigate: (href: string) => void;
  depth?: number;
}) {
  return (
    <>
      <button
        onClick={() => onNavigate(item.href)}
        className="w-full text-left text-sm text-zinc-300 hover:text-zinc-100 py-1.5 hover:bg-zinc-800 rounded px-2"
        style={{ paddingLeft: `${8 + depth * 16}px` }}
      >
        {item.label}
      </button>
      {item.subitems?.map((sub, i) => (
        <TocEntry
          key={i}
          item={sub}
          onNavigate={onNavigate}
          depth={depth + 1}
        />
      ))}
    </>
  );
}

export default EpubReader;

/**
 * Full-screen notice for a book that could not be loaded, with Retry. An
 * optional detail line says what failed, and an optional second action
 * offers a way forward that does not depend on the failing request.
 */
export function BookLoadError({
  onRetry,
  detail,
  secondaryAction,
}: {
  onRetry: () => void;
  detail?: string;
  secondaryAction?: { label: string; onClick: () => void };
}) {
  const retry = (
    <button
      onClick={onRetry}
      className={cn(
        !secondaryAction && "mt-4",
        "inline-flex items-center gap-2 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover",
      )}
    >
      <RefreshCw size={14} />
      Retry
    </button>
  );
  return (
    <div className="flex h-screen flex-col items-center justify-center bg-zinc-900 text-center">
      <AlertTriangle className="mb-4 text-red-400" size={32} />
      <h3 className="text-lg font-medium text-zinc-200">
        Could not load this book.
      </h3>
      {detail && <p className="mt-2 text-sm text-zinc-400">{detail}</p>}
      {secondaryAction ? (
        <div className="mt-4 flex flex-wrap items-center justify-center gap-3">
          {retry}
          <button
            onClick={secondaryAction.onClick}
            className="rounded border border-zinc-600 px-4 py-2 text-sm font-medium text-zinc-200 hover:bg-zinc-800"
          >
            {secondaryAction.label}
          </button>
        </div>
      ) : (
        retry
      )}
    </div>
  );
}
