import { useState, useEffect, useRef, useCallback } from "react";
import {
  getStreamUrl,
  getPlaybackProgress,
  updatePlaybackProgress,
} from "@/api";
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
} from "lucide-react";
import { useNavigate } from "react-router";
import * as Popover from "@radix-ui/react-popover";
import { cn } from "@/utils/cn";

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

  // Sleep timer
  const [sleepMinutes, setSleepMinutes] = useState<number | null>(null);
  const [sleepRemaining, setSleepRemaining] = useState<number | null>(null);
  const sleepTimerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const sleepDeadlineRef = useRef<number | null>(null);

  useEffect(() => {
    localStorage.setItem("livrarr_skip_back", String(skipBack));
  }, [skipBack]);
  useEffect(() => {
    localStorage.setItem("livrarr_skip_fwd", String(skipFwd));
  }, [skipFwd]);

  const streamUrl = getStreamUrl(libraryItemId);
  const coverUrl = `/api/v1/mediacover/${workId}/cover.jpg`;

  // Load saved progress.
  useEffect(() => {
    getPlaybackProgress(libraryItemId)
      .then((p) => {
        if (p?.position) {
          const t = parseFloat(p.position);
          if (!isNaN(t) && t > 0) {
            setCurrentTime(t);
            if (audioRef.current) audioRef.current.currentTime = t;
          }
        }
      })
      .catch(() => {});
  }, [libraryItemId]);

  const saveProgress = useCallback(
    (time: number, dur: number) => {
      if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
      saveTimerRef.current = setTimeout(() => {
        const pct = dur > 0 ? time / dur : 0;
        updatePlaybackProgress(libraryItemId, String(time), pct).catch(
          () => {},
        );
      }, 2000);
    },
    [libraryItemId],
  );

  // Periodic save while playing.
  useEffect(() => {
    if (!playing) return;
    const interval = setInterval(() => {
      if (audioRef.current) {
        const t = audioRef.current.currentTime;
        const d = audioRef.current.duration;
        if (isFinite(t) && isFinite(d)) {
          const pct = d > 0 ? t / d : 0;
          updatePlaybackProgress(libraryItemId, String(t), pct).catch(
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
      audioRef.current.currentTime = Math.max(
        0,
        Math.min(audioRef.current.currentTime + seconds, duration),
      );
    },
    [duration],
  );

  const onTimeUpdate = () => {
    if (audioRef.current) setCurrentTime(audioRef.current.currentTime);
  };

  const onLoadedMetadata = () => {
    if (audioRef.current) {
      setDuration(audioRef.current.duration);
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
      saveProgress(t, duration);
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

  // Sleep timer
  const startSleepTimer = (minutes: number) => {
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
  const rawRemaining = duration - currentTime;
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
        <div className="h-64 w-64 overflow-hidden rounded-lg bg-zinc-800 shadow-2xl">
          <img
            src={coverUrl}
            alt={workTitle}
            className="h-full w-full object-cover"
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

        {/* Seek bar */}
        <div className="w-full max-w-md">
          <input
            type="range"
            min={0}
            max={duration || 1}
            step={0.1}
            value={currentTime}
            onChange={onSeek}
            className="w-full accent-brand"
          />
          <div className="flex justify-between text-xs text-zinc-500">
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
        <div className="flex items-center gap-6">
          <button
            onClick={() => skip(-skipBack)}
            className="relative text-zinc-400 hover:text-zinc-100"
            title={`Back ${skipBack}s`}
          >
            <SkipBack size={24} />
            <span className="absolute -bottom-3 left-1/2 -translate-x-1/2 text-[10px] text-zinc-500">
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
            <span className="absolute -bottom-3 left-1/2 -translate-x-1/2 text-[10px] text-zinc-500">
              {skipFwd}
            </span>
          </button>
        </div>

        {/* Secondary controls */}
        <div className="flex items-center gap-4">
          {/* Speed */}
          <button
            onClick={cycleSpeed}
            className="rounded px-2 py-1 text-xs font-medium text-zinc-400 hover:text-zinc-100"
            title="Playback speed (S)"
          >
            {speed}x
          </button>

          {/* Sleep timer */}
          <Popover.Root>
            <Popover.Trigger asChild>
              <button
                className={cn(
                  "flex items-center gap-1 rounded px-2 py-1 text-xs",
                  sleepMinutes
                    ? "text-brand"
                    : "text-zinc-400 hover:text-zinc-100",
                )}
                title="Sleep timer"
              >
                <Timer size={14} />
                {sleepRemaining != null && (
                  <span>{formatTime(sleepRemaining)}</span>
                )}
              </button>
            </Popover.Trigger>
            <Popover.Content
              className="rounded-lg border border-zinc-700 bg-zinc-900 p-2 shadow-xl z-50"
              sideOffset={8}
            >
              {SLEEP_OPTIONS.map((m) => (
                <button
                  key={m}
                  onClick={() => startSleepTimer(m)}
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
              <Popover.Arrow className="fill-zinc-700" />
            </Popover.Content>
          </Popover.Root>
        </div>
      </div>

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
