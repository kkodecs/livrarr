import { useState, useRef, useCallback, useEffect } from "react";
import { ReactReader } from "react-reader";
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Rendition = any;
import {
  getDownloadUrl,
  getPlaybackProgress,
  updatePlaybackProgress,
} from "@/api";
import {
  ArrowLeft,
  List,
  Settings,
  Maximize2,
  Minimize2,
  X,
} from "lucide-react";
import { useNavigate } from "react-router";
import * as Popover from "@radix-ui/react-popover";
import { cn } from "@/utils/cn";

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

export function EpubReader({ libraryItemId }: Props) {
  const navigate = useNavigate();
  const containerRef = useRef<HTMLDivElement>(null);
  const renditionRef = useRef<Rendition | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [location, setLocation] = useState<string | number>(0);
  const [initialLoaded, setInitialLoaded] = useState(false);
  const [epubData, setEpubData] = useState<ArrayBuffer | null>(null);

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
    fetch(url, {
      headers: { Authorization: `Bearer ${token}` },
      signal: controller.signal,
    })
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        return res.arrayBuffer();
      })
      .then(setEpubData)
      .catch(() => {});
    return () => controller.abort();
  }, [url, token]);

  // Load saved progress on mount.
  useEffect(() => {
    getPlaybackProgress(libraryItemId)
      .then((p) => {
        if (p?.position) setLocation(p.position);
      })
      .catch(() => {})
      .finally(() => setInitialLoaded(true));
  }, [libraryItemId]);

  // Save progress with trailing debounce.
  const saveProgress = useCallback(
    (cfi: string, pct: number) => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        updatePlaybackProgress(libraryItemId, cfi, pct).catch(() => {});
      }, 2000);
    },
    [libraryItemId],
  );

  useEffect(() => {
    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    };
  }, []);

  const onLocationChanged = useCallback(
    (loc: string) => {
      setLocation(loc);
      if (renditionRef.current) {
        const displayed = renditionRef.current.location;
        if (displayed?.start?.percentage != null) {
          saveProgress(loc, displayed.start.percentage);
        }
      }
    },
    [saveProgress],
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
          renditionRef.current?.next();
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
  }, [toggleFullscreen, tocOpen]);

  if (!initialLoaded || !epubData) {
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
      style={{ background: darkTheme ? "#18181b" : "#fafaf9" }}
    >
      {/* Toolbar */}
      <div className="flex items-center gap-3 border-b border-zinc-700 bg-zinc-900 px-4 py-2">
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
        <div className="flex-1" />

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

            <Popover.Arrow className="fill-zinc-700" />
          </Popover.Content>
        </Popover.Root>

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
          }}
        />
      </div>
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
