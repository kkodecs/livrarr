import { useState, useEffect, useRef, useCallback, useMemo } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import {
  getStreamUrl,
  getPlaybackProgress,
  updatePlaybackProgress,
  getCrossFormatPrompt,
  declineCrossFormat,
  syncCrossFormatToHere,
} from "@/api";
import { apiFetch } from "@/api/client";
import { ResumePromptBanner } from "@/components/ResumePromptBanner";
import {
  ArrowLeft,
  Play,
  Pause,
  SkipBack,
  SkipForward,
  Volume2,
  VolumeX,
  Timer,
  Settings,
  Maximize2,
  Minimize2,
  List,
  ChevronLeft,
  ChevronRight,
  Bookmark,
  Check,
  Pencil,
  X,
} from "lucide-react";
import { useNavigate } from "react-router";
import * as Popover from "@radix-ui/react-popover";
import { cn } from "@/utils/cn";
import type { ChapterResponse, BookmarkResponse, CreateBookmarkRequest, ResumePromptDTO } from "@/types/api";

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3] as const;
const SLEEP_OPTIONS = [5, 10, 15, 30, 45, 60];
const SKIP_OPTIONS = [5, 10, 15, 30, 45, 60];

interface Props {
  libraryItemId: number;
  workTitle: string;
  authorName: string;
  workId: number;
}

function formatTime(seconds: number): string {
  if (!isFinite(seconds) || seconds < 0) return "0:00";
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = Math.floor(seconds % 60);
  if (h > 0)
    return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function AudioPlayer({
  libraryItemId,
  workTitle,
  authorName,
  workId,
}: Props) {
  const navigate = useNavigate();
  const audioRef = useRef<HTMLAudioElement>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [speedIdx, setSpeedIdx] = useState(2);
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);
  const [isFullscreen, setIsFullscreen] = useState(false);

  // Configurable skip amounts (persisted)
  const [skipBack, setSkipBack] = useState(() =>
    Number(localStorage.getItem("livrarr_skip_back") ?? "15"),
  );
  const [skipFwd, setSkipFwd] = useState(() =>
    Number(localStorage.getItem("livrarr_skip_fwd") ?? "30"),
  );

  // Chapters
  const { data: chapters = [] } = useQuery<ChapterResponse[]>({
    queryKey: ["chapters", libraryItemId],
    queryFn: () => apiFetch(`/workfile/${libraryItemId}/chapters`),
  });
  const [chapterPanelOpen, setChapterPanelOpen] = useState(false);
  const [sleepAtChapterEnd, setSleepAtChapterEnd] = useState(false);

  const currentChapter = useMemo(() => {
    if (chapters.length === 0) return null;
    return chapters.find(
      (c) => currentTime >= c.startTimeSecs && currentTime < c.endTimeSecs,
    ) ?? null;
  }, [chapters, currentTime]);

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
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["bookmarks", libraryItemId] }),
  });

  const deleteBookmarkMut = useMutation({
    mutationFn: (id: number) =>
      apiFetch(`/bookmarks/${id}`, { method: "DELETE" }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["bookmarks", libraryItemId] }),
  });

  const renameBookmarkMut = useMutation({
    mutationFn: ({ id, name }: { id: number; name: string }) =>
      apiFetch(`/bookmarks/${id}`, {
        method: "PATCH",
        body: JSON.stringify({ name }),
      }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["bookmarks", libraryItemId] }),
  });
  const [renamingId, setRenamingId] = useState<number | null>(null);
  const [renameValue, setRenameValue] = useState("");

  // Sleep timer
  const [sleepMinutes, setSleepMinutes] = useState<number | null>(null);
  const [sleepRemaining, setSleepRemaining] = useState<number | null>(null);
  const sleepTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const sleepDeadlineRef = useRef<number | null>(null);

  // Cross-format resume
  const [resumePrompt, setResumePrompt] = useState<ResumePromptDTO | null>(null);
  const promptFiredRef = useRef(false);

  // Sleep bookmark dedup guard (in-flight: query may not reflect a recent create)
  const lastSleepBookmarkRef = useRef<{ pos: number } | null>(null);

  useEffect(() => {
    localStorage.setItem("livrarr_skip_back", String(skipBack));
  }, [skipBack]);
  useEffect(() => {
    localStorage.setItem("livrarr_skip_fwd", String(skipFwd));
  }, [skipFwd]);

  const streamUrl = getStreamUrl(libraryItemId);
  const coverUrl = `/api/v1/mediacover/${workId}/audiocover.jpg`;

  // Load saved progress, then check for a cross-format resume prompt. The
  // prompt fires even when no progress row exists yet (404): a never-played
  // audiobook whose linked ebook is ahead is exactly the resume moment.
  useEffect(() => {
    promptFiredRef.current = false;
    const fetchPrompt = (currentTs: number) => {
      if (promptFiredRef.current) return;
      promptFiredRef.current = true;
      getCrossFormatPrompt(libraryItemId, currentTs)
        .then((dto) => {
          if (dto) setResumePrompt(dto);
        })
        .catch(() => {});
    };
    getPlaybackProgress(libraryItemId)
      .then((p) => {
        let restoredTs = 0;
        if (p?.position) {
          const t = parseFloat(p.position);
          if (!isNaN(t) && t > 0) {
            restoredTs = t;
            setCurrentTime(t);
            if (audioRef.current) audioRef.current.currentTime = t;
          }
        }
        fetchPrompt(restoredTs);
      })
      .catch(() => fetchPrompt(0));
  }, [libraryItemId]);

  const saveProgress = useCallback(
    (
      time: number,
      dur: number,
      kind: "progress" | "seek" = "progress",
    ) => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        const pct = isFinite(dur) && dur > 0 ? time / dur : 0;
        updatePlaybackProgress(libraryItemId, String(time), pct, kind, time).catch(
          () => {},
        );
      }, 2000);
    },
    [libraryItemId],
  );

  // Periodic save while playing (genuine progress; advances furthest_ts).
  useEffect(() => {
    if (!playing) return;
    const interval = setInterval(() => {
      if (audioRef.current) {
        const t = audioRef.current.currentTime;
        const d = audioRef.current.duration;
        if (isFinite(t) && isFinite(d)) {
          const pct = d > 0 ? t / d : 0;
          updatePlaybackProgress(libraryItemId, String(t), pct, "progress", t).catch(
            () => {},
          );
        }
      }
    }, 10000);
    return () => clearInterval(interval);
  }, [playing, libraryItemId]);

  useEffect(() => {
    return () => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    };
  }, []);

  const togglePlay = useCallback(() => {
    if (!audioRef.current) return;
    if (playing) {
      audioRef.current.pause();
      saveProgress(audioRef.current.currentTime, audioRef.current.duration);
    } else {
      audioRef.current.play().catch(() => {});
    }
    setPlaying(!playing);
  }, [playing, saveProgress]);

  const skip = useCallback(
    (seconds: number) => {
      if (!audioRef.current) return;
      const cap = isFinite(duration) && duration > 0 ? duration : Number.MAX_SAFE_INTEGER;
      audioRef.current.currentTime = Math.max(
        0,
        Math.min(audioRef.current.currentTime + seconds, cap),
      );
    },
    [duration],
  );

  const onTimeUpdate = () => {
    if (!audioRef.current) return;
    const t = audioRef.current.currentTime;
    setCurrentTime(t);

    if (sleepAtChapterEnd && currentChapter && t >= currentChapter.endTimeSecs) {
      audioRef.current.pause();
      setPlaying(false);
      setSleepAtChapterEnd(false);
      saveProgress(t, audioRef.current.duration);
    }
  };

  const onLoadedMetadata = () => {
    if (audioRef.current) {
      const d = audioRef.current.duration;
      setDuration(isFinite(d) && d > 0 ? d : 0);
      getPlaybackProgress(libraryItemId)
        .then((p) => {
          if (p?.position && audioRef.current) {
            const t = parseFloat(p.position);
            if (!isNaN(t) && t > 0) audioRef.current.currentTime = t;
          }
        })
        .catch(() => {});
    }
  };

  const onSeek = (e: React.ChangeEvent<HTMLInputElement>) => {
    const t = parseFloat(e.target.value);
    if (audioRef.current) {
      audioRef.current.currentTime = t;
      setCurrentTime(t);
      saveProgress(t, duration, "seek");
    }
  };

  const cycleSpeed = useCallback(() => {
    const next = (speedIdx + 1) % SPEEDS.length;
    setSpeedIdx(next);
    if (audioRef.current) audioRef.current.playbackRate = SPEEDS[next] ?? 1;
  }, [speedIdx]);

  const toggleMute = useCallback(() => {
    setMuted(!muted);
    if (audioRef.current) audioRef.current.muted = !muted;
  }, [muted]);

  const onVolumeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const v = parseFloat(e.target.value);
    setVolume(v);
    if (audioRef.current) audioRef.current.volume = v;
  };

  // Sleep bookmark helper (REQ-010..REQ-013)
  const createSleepBookmark = useCallback(() => {
    const t = audioRef.current?.currentTime ?? currentTime;
    // Dedupe: skip if an existing bookmark is within 60s of current position.
    const tooClose = bookmarks.some(
      (bm) =>
        bm.name.startsWith("Sleep Timer / ") &&
        Math.abs(parseFloat(bm.position) - t) <= 60,
    );
    if (tooClose) return;
    // In-flight guard: the query may not reflect a create fired seconds ago.
    if (
      lastSleepBookmarkRef.current !== null &&
      Math.abs(lastSleepBookmarkRef.current.pos - t) <= 60
    ) {
      return;
    }
    const now = new Date();
    const name = `Sleep Timer / ${now.toLocaleDateString()} @ ${now.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
    createBookmarkMut.mutate(
      {
        position: String(t),
        sortKey: t,
        name,
        chapterTitle: currentChapter?.title ?? null,
      },
      {
        onError: (err) => console.warn("Sleep bookmark failed", err),
      },
    );
    lastSleepBookmarkRef.current = { pos: t };
    toast("Bookmark saved");
  }, [audioRef, currentTime, bookmarks, currentChapter, createBookmarkMut]);

  // Sleep timer
  const startSleepTimer = (minutes: number) => {
    createSleepBookmark();
    if (sleepTimerRef.current) clearInterval(sleepTimerRef.current);

    const deadline = Date.now() + minutes * 60 * 1000;
    sleepDeadlineRef.current = deadline;
    setSleepMinutes(minutes);
    setSleepRemaining(minutes * 60);

    sleepTimerRef.current = setInterval(() => {
      const remaining = Math.max(
        0,
        Math.round(((sleepDeadlineRef.current ?? 0) - Date.now()) / 1000),
      );
      setSleepRemaining(remaining);
      if (remaining <= 0) {
        audioRef.current?.pause();
        setPlaying(false);
        if (audioRef.current) {
          saveProgress(
            audioRef.current.currentTime,
            audioRef.current.duration,
          );
        }
        cancelSleepTimer();
      }
    }, 1000);
  };

  const cancelSleepTimer = () => {
    if (sleepTimerRef.current) clearInterval(sleepTimerRef.current);
    sleepTimerRef.current = null;
    sleepDeadlineRef.current = null;
    setSleepMinutes(null);
    setSleepRemaining(null);
  };

  useEffect(() => {
    return () => {
      if (sleepTimerRef.current) clearInterval(sleepTimerRef.current);
    };
  }, []);

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
        case " ":
          e.preventDefault();
          togglePlay();
          break;
        case "ArrowLeft":
          skip(-skipBack);
          break;
        case "ArrowRight":
          skip(skipFwd);
          break;
        case "ArrowUp":
          e.preventDefault();
          setVolume((v) => {
            const nv = Math.min(1, v + 0.05);
            if (audioRef.current) audioRef.current.volume = nv;
            return nv;
          });
          break;
        case "ArrowDown":
          e.preventDefault();
          setVolume((v) => {
            const nv = Math.max(0, v - 0.05);
            if (audioRef.current) audioRef.current.volume = nv;
            return nv;
          });
          break;
        case "m":
          toggleMute();
          break;
        case "s":
          if (!e.ctrlKey && !e.metaKey) cycleSpeed();
          break;
        case "f":
          if (!e.ctrlKey && !e.metaKey) toggleFullscreen();
          break;
      }
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [togglePlay, skip, skipBack, skipFwd, toggleMute, cycleSpeed, toggleFullscreen]);

  const speed = SPEEDS[speedIdx] ?? 1;
  const rawRemaining = isFinite(duration) ? duration - currentTime : 0;
  const adjustedRemaining = rawRemaining / speed;

  return (
    <div ref={containerRef} className="flex h-screen flex-col bg-zinc-900">
      {/* Top bar */}
      <div className="flex items-center border-b border-zinc-700 bg-zinc-900 px-4 py-2">
        <button
          onClick={() => navigate(-1)}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title="Back"
        >
          <ArrowLeft size={20} />
        </button>
        <div className="flex-1" />
        <button
          onClick={toggleFullscreen}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title={isFullscreen ? "Exit fullscreen (F)" : "Fullscreen (F)"}
        >
          {isFullscreen ? <Minimize2 size={16} /> : <Maximize2 size={16} />}
        </button>
      </div>

      {/* Main content */}
      <div className="flex flex-1 flex-col items-center justify-center gap-8 px-8">
        {/* Cover */}
        <div className="flex max-h-80 max-w-64 items-center justify-center overflow-hidden rounded-lg bg-zinc-800 shadow-2xl">
          <img
            src={coverUrl}
            alt={workTitle}
            className="max-h-80 max-w-64 object-contain"
            onError={(e) => {
              (e.target as HTMLImageElement).style.display = "none";
            }}
          />
        </div>

        {/* Title + author */}
        <div className="text-center">
          <h1 className="text-xl font-semibold text-zinc-100">{workTitle}</h1>
          <p className="text-sm text-zinc-400">{authorName}</p>
        </div>

        {/* Chapter bar */}
        {chapters.length > 0 && currentChapter && (
          <div className="w-full max-w-md flex items-center gap-2">
            <span className="text-sm text-zinc-400 truncate flex-1">
              {currentChapter.title}
            </span>
            <button
              onClick={() => setChapterPanelOpen(!chapterPanelOpen)}
              className="text-zinc-400 hover:text-zinc-100"
              title="Chapter list"
            >
              <List size={16} />
            </button>
          </div>
        )}

        {/* Seek bar */}
        <div className="w-full max-w-md">
          <div className="relative">
            <input
              type="range"
              min={0}
              max={isFinite(duration) && duration > 0 ? duration : 1}
              step={0.1}
              value={currentTime}
              onChange={onSeek}
              className="w-full accent-brand"
            />
            {/* Chapter tick marks */}
            {chapters.length > 0 && isFinite(duration) && duration > 0 && (
              <div className="absolute top-0 left-0 right-0 h-full pointer-events-none">
                {chapters.slice(1).map((ch) => (
                  <div
                    key={ch.id}
                    className="absolute top-0 w-0.5 h-full bg-zinc-400"
                    style={{ left: `${(ch.startTimeSecs / duration) * 100}%` }}
                  />
                ))}
              </div>
            )}
          </div>
          {/* Chapter progress bar */}
          {currentChapter && (() => {
            const chElapsed = currentTime - currentChapter.startTimeSecs;
            const chDuration = currentChapter.endTimeSecs - currentChapter.startTimeSecs;
            const chPct = chDuration > 0 ? Math.min(100, (chElapsed / chDuration) * 100) : 0;
            return (
              <div className="mt-1">
                <div className="h-1.5 bg-zinc-800 rounded-full overflow-hidden">
                  <div
                    className="h-full bg-brand/60 rounded-full"
                    style={{ width: `${chPct}%` }}
                  />
                </div>
                <div className="flex justify-between text-xs text-zinc-500 mt-0.5">
                  <span>Ch {currentChapter.chapterIndex + 1}/{chapters.length}</span>
                  <span>{formatTime(chElapsed)} / {formatTime(chDuration)}</span>
                </div>
              </div>
            );
          })()}
          <div className="flex justify-between text-sm text-zinc-500 mt-1">
            <span>{formatTime(currentTime)}</span>
            <span>
              -{formatTime(adjustedRemaining)}
              {speed !== 1 && (
                <span className="text-zinc-600 ml-1">at {speed}x</span>
              )}
            </span>
          </div>
        </div>

        {/* Controls */}
        <div className="flex items-center gap-4">
          {chapters.length > 0 && (
            <button
              onClick={() => {
                if (!audioRef.current || !currentChapter) return;
                const timeInCh = currentTime - currentChapter.startTimeSecs;
                if (timeInCh > 3) {
                  audioRef.current.currentTime = currentChapter.startTimeSecs;
                } else {
                  const prevIdx = currentChapter.chapterIndex - 1;
                  audioRef.current.currentTime =
                    prevIdx >= 0 ? chapters[prevIdx]!.startTimeSecs : 0;
                }
              }}
              className="text-zinc-400 hover:text-zinc-100"
              title="Previous chapter"
            >
              <ChevronLeft size={20} />
            </button>
          )}
          <button
            onClick={() => skip(-skipBack)}
            className="relative text-zinc-400 hover:text-zinc-100"
            title={`Back ${skipBack}s`}
          >
            <SkipBack size={24} />
            <span className="absolute -bottom-3 left-1/2 -translate-x-1/2 text-xs text-zinc-500">
              {skipBack}
            </span>
          </button>
          <button
            onClick={togglePlay}
            className="flex h-14 w-14 items-center justify-center rounded-full bg-zinc-100 text-zinc-900 hover:bg-zinc-200"
          >
            {playing ? (
              <Pause size={28} />
            ) : (
              <Play size={28} className="ml-1" />
            )}
          </button>
          <button
            onClick={() => skip(skipFwd)}
            className="relative text-zinc-400 hover:text-zinc-100"
            title={`Forward ${skipFwd}s`}
          >
            <SkipForward size={24} />
            <span className="absolute -bottom-3 left-1/2 -translate-x-1/2 text-xs text-zinc-500">
              {skipFwd}
            </span>
          </button>
          {chapters.length > 0 && (
            <button
              onClick={() => {
                if (!audioRef.current || !currentChapter) return;
                const nextIdx = currentChapter.chapterIndex + 1;
                if (nextIdx < chapters.length) {
                  audioRef.current.currentTime = chapters[nextIdx]!.startTimeSecs;
                }
              }}
              disabled={!currentChapter || currentChapter.chapterIndex >= chapters.length - 1}
              className="text-zinc-400 hover:text-zinc-100 disabled:opacity-30"
              title="Next chapter"
            >
              <ChevronRight size={20} />
            </button>
          )}
        </div>

        {/* Secondary controls */}
        <div className="flex items-center gap-4">
          {/* Speed */}
          <button
            onClick={cycleSpeed}
            className="rounded px-2 py-1 text-sm font-medium text-zinc-400 hover:text-zinc-100"
            title="Playback speed (S)"
          >
            {speed}x
          </button>

          {/* Sleep timer */}
          <Popover.Root>
            <Popover.Trigger asChild>
              <button
                className={cn(
                  "flex items-center gap-1 rounded px-2 py-1 text-sm",
                  sleepMinutes
                    ? "text-brand"
                    : "text-zinc-400 hover:text-zinc-100",
                )}
                title="Sleep timer"
              >
                <Timer size={16} />
                {sleepRemaining != null && (
                  <span>{formatTime(sleepRemaining)}</span>
                )}
              </button>
            </Popover.Trigger>
            <Popover.Content
              className="rounded-lg border border-zinc-700 bg-zinc-900 p-2 shadow-xl z-50"
              sideOffset={8}
            >
              {chapters.length > 0 && (
                <button
                  onClick={() => {
                    cancelSleepTimer();
                    createSleepBookmark();
                    setSleepAtChapterEnd(true);
                  }}
                  className={cn(
                    "block w-full text-left text-sm rounded px-3 py-1.5 mb-1",
                    sleepAtChapterEnd
                      ? "text-brand bg-zinc-800"
                      : "text-zinc-300 hover:bg-zinc-800",
                  )}
                >
                  {sleepAtChapterEnd ? "Sleeping at chapter end" : "End of chapter"}
                </button>
              )}
              {SLEEP_OPTIONS.map((m) => (
                <button
                  key={m}
                  onClick={() => {
                    setSleepAtChapterEnd(false);
                    startSleepTimer(m);
                  }}
                  className="block w-full text-left text-sm text-zinc-300 hover:bg-zinc-800 rounded px-3 py-1.5"
                >
                  {m} minutes
                </button>
              ))}
              {sleepMinutes && (
                <button
                  onClick={cancelSleepTimer}
                  className="block w-full text-left text-sm text-red-400 hover:bg-zinc-800 rounded px-3 py-1.5 mt-1 border-t border-zinc-700 pt-1.5"
                >
                  Cancel timer
                </button>
              )}
              <Popover.Arrow className="fill-zinc-700" />
            </Popover.Content>
          </Popover.Root>

          {/* Volume */}
          <div className="flex items-center gap-2">
            <button
              onClick={toggleMute}
              className="text-zinc-400 hover:text-zinc-100"
              title="Mute (M)"
            >
              {muted ? <VolumeX size={16} /> : <Volume2 size={16} />}
            </button>
            <input
              type="range"
              min={0}
              max={1}
              step={0.05}
              value={muted ? 0 : volume}
              onChange={onVolumeChange}
              className="w-20 accent-brand"
            />
          </div>

          {/* Player settings */}
          <Popover.Root>
            <Popover.Trigger asChild>
              <button
                className="rounded p-1 text-zinc-400 hover:text-zinc-100"
                title="Player settings"
              >
                <Settings size={14} />
              </button>
            </Popover.Trigger>
            <Popover.Content
              className="rounded-lg border border-zinc-700 bg-zinc-900 p-4 shadow-xl w-56 z-50"
              sideOffset={8}
              align="end"
            >
              <label className="block text-xs text-zinc-400 mb-1">
                Skip Back (seconds)
              </label>
              <select
                value={skipBack}
                onChange={(e) => setSkipBack(Number(e.target.value))}
                className="w-full rounded bg-zinc-800 border border-zinc-700 text-sm text-zinc-200 px-2 py-1 mb-3"
              >
                {SKIP_OPTIONS.map((s) => (
                  <option key={s} value={s}>
                    {s}s
                  </option>
                ))}
              </select>
              <label className="block text-xs text-zinc-400 mb-1">
                Skip Forward (seconds)
              </label>
              <select
                value={skipFwd}
                onChange={(e) => setSkipFwd(Number(e.target.value))}
                className="w-full rounded bg-zinc-800 border border-zinc-700 text-sm text-zinc-200 px-2 py-1"
              >
                {SKIP_OPTIONS.map((s) => (
                  <option key={s} value={s}>
                    {s}s
                  </option>
                ))}
              </select>
              <div className="mt-3 border-t border-zinc-700 pt-3">
                <button
                  onClick={() => {
                    syncCrossFormatToHere(libraryItemId, currentTime).catch(
                      () => {},
                    );
                    toast("Position synced");
                  }}
                  className="w-full rounded px-2 py-1.5 text-left text-xs text-zinc-300 hover:bg-zinc-800"
                >
                  Sync to here
                </button>
              </div>
              <Popover.Arrow className="fill-zinc-700" />
            </Popover.Content>
          </Popover.Root>

          {/* Bookmark button */}
          <button
            onClick={() => {
              const pos = String(currentTime);
              const name = currentChapter
                ? `${currentChapter.title} — ${formatTime(currentTime)}`
                : formatTime(currentTime);
              createBookmarkMut.mutate({
                position: pos,
                sortKey: currentTime,
                name,
                chapterTitle: currentChapter?.title ?? null,
              });
            }}
            className="text-zinc-400 hover:text-zinc-100"
            title="Add bookmark"
          >
            <Bookmark size={16} />
          </button>
          <button
            onClick={() => setBookmarkPanelOpen(!bookmarkPanelOpen)}
            className={cn(
              "text-sm px-2 py-1 rounded",
              bookmarkPanelOpen ? "text-brand" : "text-zinc-400 hover:text-zinc-100",
            )}
            title="Bookmarks"
          >
            {bookmarks.length > 0 ? `${bookmarks.length}` : "0"} bookmarks
          </button>
        </div>
      </div>

      {/* Chapter panel */}
      {chapterPanelOpen && chapters.length > 0 && (
        <div className="fixed right-0 top-0 bottom-0 w-80 bg-zinc-900 border-l border-zinc-700 z-50 overflow-y-auto">
          <div className="flex items-center justify-between p-3 border-b border-zinc-700">
            <span className="text-sm font-medium text-zinc-100">Chapters</span>
            <button onClick={() => setChapterPanelOpen(false)} className="text-zinc-400 hover:text-zinc-100">
              <X size={16} />
            </button>
          </div>
          {chapters.map((ch) => {
            const isCurrent = currentChapter?.id === ch.id;
            const isPast = ch.endTimeSecs <= currentTime;
            return (
              <button
                key={ch.id}
                onClick={() => {
                  if (audioRef.current) {
                    audioRef.current.currentTime = ch.startTimeSecs;
                    audioRef.current.play().catch(() => {});
                    setPlaying(true);
                  }
                }}
                className={cn(
                  "block w-full text-left px-3 py-2 text-sm border-b border-zinc-800 hover:bg-zinc-800",
                  isCurrent && "bg-zinc-800",
                )}
              >
                <div className="flex items-center gap-2">
                  <span className="w-5 text-center">
                    {isPast ? <Check size={12} className="text-green-500" /> : isCurrent ? <Play size={12} className="text-brand" /> : null}
                  </span>
                  <span className={cn("flex-1 truncate", isCurrent ? "text-zinc-100" : "text-zinc-400")}>
                    {ch.chapterIndex + 1}. {ch.title}
                  </span>
                  <span className="text-xs text-zinc-500">{formatTime(ch.startTimeSecs)}</span>
                </div>
              </button>
            );
          })}
        </div>
      )}

      {/* Bookmark panel */}
      {bookmarkPanelOpen && (
        <div className="fixed right-0 top-0 bottom-0 w-80 bg-zinc-900 border-l border-zinc-700 z-50 overflow-y-auto">
          <div className="flex items-center justify-between p-3 border-b border-zinc-700">
            <span className="text-sm font-medium text-zinc-100">Bookmarks</span>
            <button onClick={() => setBookmarkPanelOpen(false)} className="text-zinc-400 hover:text-zinc-100">
              <X size={16} />
            </button>
          </div>
          {bookmarks.length === 0 ? (
            <p className="p-4 text-sm text-zinc-500">No bookmarks yet</p>
          ) : (
            bookmarks.map((bm) => (
              <div
                key={bm.id}
                className="flex items-center gap-2 px-3 py-2 border-b border-zinc-800 hover:bg-zinc-800 group cursor-pointer"
                onClick={() => {
                  if (renamingId !== bm.id && audioRef.current)
                    audioRef.current.currentTime = parseFloat(bm.position);
                }}
              >
                <div className="flex-1 min-w-0">
                  {renamingId === bm.id ? (
                    <form
                      onSubmit={(e) => {
                        e.preventDefault();
                        renameBookmarkMut.mutate({ id: bm.id, name: renameValue });
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
                        onKeyDown={(e) => { if (e.key === "Escape") setRenamingId(null); }}
                      />
                    </form>
                  ) : (
                    <>
                      <p className="text-sm text-zinc-200 truncate">{bm.name}</p>
                      {bm.chapterTitle && (
                        <p className="text-xs text-zinc-500 truncate">{bm.chapterTitle}</p>
                      )}
                    </>
                  )}
                </div>
                <span className="text-xs text-zinc-500">{formatTime(parseFloat(bm.position))}</span>
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
      )}

      {/* Cross-format resume banner */}
      {resumePrompt && (
        <ResumePromptBanner
          label={resumePrompt.label}
          onJump={() => {
            const t = parseFloat(resumePrompt.position);
            if (audioRef.current) {
              audioRef.current.currentTime = t;
            }
            setCurrentTime(t);
            updatePlaybackProgress(
              libraryItemId,
              resumePrompt.position,
              duration > 0 ? t / duration : 0,
              "seek",
              t,
            ).catch(() => {});
            setResumePrompt(null);
          }}
          onStay={() => {
            declineCrossFormat(libraryItemId).catch(() => {});
            setResumePrompt(null);
          }}
        />
      )}

      {/* Hidden audio element */}
      <audio
        ref={audioRef}
        src={streamUrl}
        onTimeUpdate={onTimeUpdate}
        onLoadedMetadata={onLoadedMetadata}
        onEnded={() => setPlaying(false)}
        preload="metadata"
      />
    </div>
  );
}

export default AudioPlayer;
