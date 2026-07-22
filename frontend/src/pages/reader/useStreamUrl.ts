import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { toast } from "sonner";
import { mintStreamToken } from "@/api";
import {
  StreamTokenController,
  type PlaybackSnapshot,
} from "./streamTokenController";

/**
 * Unit C — scoped, expiring stream token, wired to a real `<audio>`
 * element. Mints on mount, proactively refreshes before expiry, and
 * reminds at most once on a media error (loop-prevention), preserving
 * `currentTime` and play state across the reload. A stale mint from a
 * previous `libraryItemId` is cancelled automatically.
 *
 * The caller's existing `onLoadedMetadata` handler must call
 * `consumeRestore()` first and, if it returns non-null, apply the
 * snapshot and skip its normal (server-round-trip) position restore —
 * that path is for the very first load only.
 */
export function useStreamUrl(
  libraryItemId: number,
  audioRef: RefObject<HTMLAudioElement | null>,
) {
  const [streamUrl, setStreamUrl] = useState<string | null>(null);
  const controllerRef = useRef<StreamTokenController | null>(null);
  const pendingRestoreRef = useRef<PlaybackSnapshot | null>(null);

  useEffect(() => {
    setStreamUrl(null);
    pendingRestoreRef.current = null;

    const controller = new StreamTokenController({
      mint: () => mintStreamToken(libraryItemId),
      buildUrl: (token) =>
        `/api/v1/stream/${libraryItemId}?token=${encodeURIComponent(token)}`,
      captureState: () => ({
        time: audioRef.current?.currentTime ?? 0,
        wasPlaying: audioRef.current ? !audioRef.current.paused : false,
      }),
      onUrlReady: (url, restoreSnapshot) => {
        pendingRestoreRef.current = restoreSnapshot;
        setStreamUrl(url);
      },
      scheduleRefresh: (delay, cb) => window.setTimeout(cb, delay),
      cancelRefresh: (handle) =>
        window.clearTimeout(handle as ReturnType<typeof window.setTimeout>),
      now: () => Date.now(),
      onPermanentFailure: () => {
        toast.error("Playback error — try reloading the page");
      },
    });

    controllerRef.current = controller;
    void controller.start();

    return () => {
      controller.dispose();
      controllerRef.current = null;
    };
    // audioRef is a stable ref object; mint/buildUrl close over libraryItemId
    // directly, so it is the only real dependency.
  }, [libraryItemId]);

  const handleMediaError = useCallback(() => {
    void controllerRef.current?.handleMediaError();
  }, []);

  /** Consume any pending time/play-state restore. Call from onLoadedMetadata. */
  const consumeRestore = useCallback((): PlaybackSnapshot | null => {
    const snapshot = pendingRestoreRef.current;
    pendingRestoreRef.current = null;
    return snapshot;
  }, []);

  return { streamUrl, handleMediaError, consumeRestore };
}
