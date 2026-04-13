import { useState, useRef, useCallback, useEffect } from "react";
import { ReactReader } from "react-reader";
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Rendition = any;
import { getDownloadUrl, getPlaybackProgress, updatePlaybackProgress } from "@/api";
import { ArrowLeft, Sun, Moon, Type } from "lucide-react";
import { useNavigate } from "react-router";

const FONT_SIZES = ["90%", "110%", "130%"] as const;
const FONT_LABELS = ["Small", "Medium", "Large"] as const;

interface Props {
  libraryItemId: number;
}

export function EpubReader({ libraryItemId }: Props) {
  const navigate = useNavigate();
  const [location, setLocation] = useState<string | number>(0);
  const [darkTheme, setDarkTheme] = useState(true);
  const [fontIdx, setFontIdx] = useState(1);
  const renditionRef = useRef<Rendition | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [initialLoaded, setInitialLoaded] = useState(false);

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

  // Save on unmount.
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
      rendition.themes.override("font-size", FONT_SIZES[fontIdx]);
    },
    [darkTheme, fontIdx],
  );

  useEffect(() => {
    if (renditionRef.current) applyTheme(renditionRef.current);
  }, [applyTheme]);

  const url = getDownloadUrl(libraryItemId);

  if (!initialLoaded) {
    return (
      <div className="flex h-screen items-center justify-center bg-zinc-900 text-zinc-400">
        Loading...
      </div>
    );
  }

  return (
    <div
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
        <div className="flex-1" />
        <button
          onClick={() => setFontIdx((i) => (i + 1) % FONT_SIZES.length)}
          className="flex items-center gap-1 rounded px-2 py-1 text-xs text-zinc-400 hover:text-zinc-100"
          title="Font size"
        >
          <Type size={14} />
          <span>{FONT_LABELS[fontIdx]}</span>
        </button>
        <button
          onClick={() => setDarkTheme((d) => !d)}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title={darkTheme ? "Light mode" : "Dark mode"}
        >
          {darkTheme ? <Sun size={16} /> : <Moon size={16} />}
        </button>
      </div>

      {/* Reader */}
      <div className="flex-1">
        <ReactReader
          url={url}
          location={location}
          locationChanged={onLocationChanged}
          epubInitOptions={{
            requestHeaders: {
              Authorization: `Bearer ${localStorage.getItem("livrarr_token") ?? ""}`,
            },
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

export default EpubReader;
