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
} from "lucide-react";
import { useNavigate } from "react-router";

const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3] as const;

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
  if (h > 0) return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
  return `${m}:${s.toString().padStart(2, "0")}`;
}

export function AudioPlayer({ libraryItemId, workTitle, authorName, workId }: Props) {
  const navigate = useNavigate();
  const audioRef = useRef<HTMLAudioElement>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [playing, setPlaying] = useState(false);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [speedIdx, setSpeedIdx] = useState(2); // 1x
  const [volume, setVolume] = useState(1);
  const [muted, setMuted] = useState(false);

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

  // Save progress periodically and on pause.
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

  const togglePlay = () => {
    if (!audioRef.current) return;
    if (playing) {
      audioRef.current.pause();
      saveProgress(audioRef.current.currentTime, audioRef.current.duration);
    } else {
      audioRef.current.play().catch(() => {});
    }
    setPlaying(!playing);
  };

  const skip = (seconds: number) => {
    if (!audioRef.current) return;
    audioRef.current.currentTime = Math.max(
      0,
      Math.min(audioRef.current.currentTime + seconds, duration),
    );
  };

  const onTimeUpdate = () => {
    if (audioRef.current) setCurrentTime(audioRef.current.currentTime);
  };

  const onLoadedMetadata = () => {
    if (audioRef.current) {
      setDuration(audioRef.current.duration);
      // Restore position after metadata loads.
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

  const cycleSpeed = () => {
    const next = (speedIdx + 1) % SPEEDS.length;
    setSpeedIdx(next);
    if (audioRef.current) audioRef.current.playbackRate = SPEEDS[next] ?? 1;
  };

  const toggleMute = () => {
    setMuted(!muted);
    if (audioRef.current) audioRef.current.muted = !muted;
  };

  const onVolumeChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const v = parseFloat(e.target.value);
    setVolume(v);
    if (audioRef.current) audioRef.current.volume = v;
  };

  return (
    <div className="flex h-screen flex-col bg-zinc-900">
      {/* Top bar */}
      <div className="flex items-center border-b border-zinc-700 bg-zinc-900 px-4 py-2">
        <button
          onClick={() => navigate(-1)}
          className="rounded p-1 text-zinc-400 hover:text-zinc-100"
          title="Back"
        >
          <ArrowLeft size={20} />
        </button>
      </div>

      {/* Main content — album art style */}
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
            <span>-{formatTime(duration - currentTime)}</span>
          </div>
        </div>

        {/* Controls */}
        <div className="flex items-center gap-6">
          <button
            onClick={() => skip(-15)}
            className="text-zinc-400 hover:text-zinc-100"
            title="Back 15s"
          >
            <SkipBack size={24} />
          </button>
          <button
            onClick={togglePlay}
            className="flex h-14 w-14 items-center justify-center rounded-full bg-zinc-100 text-zinc-900 hover:bg-zinc-200"
          >
            {playing ? <Pause size={28} /> : <Play size={28} className="ml-1" />}
          </button>
          <button
            onClick={() => skip(30)}
            className="text-zinc-400 hover:text-zinc-100"
            title="Forward 30s"
          >
            <SkipForward size={24} />
          </button>
        </div>

        {/* Secondary controls */}
        <div className="flex items-center gap-6">
          <button
            onClick={cycleSpeed}
            className="rounded px-2 py-1 text-xs font-medium text-zinc-400 hover:text-zinc-100"
          >
            {SPEEDS[speedIdx]}x
          </button>
          <div className="flex items-center gap-2">
            <button
              onClick={toggleMute}
              className="text-zinc-400 hover:text-zinc-100"
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
        </div>
      </div>

      {/* Hidden audio element — uses token-authenticated stream URL */}
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
