import { useCallback, useEffect, useRef, useState, type RefObject } from "react";
import { mintStreamToken } from "@/api";
import {
  StreamTokenController,
  type PlaybackSnapshot,
} from "./streamTokenController";

/** The path of a library item's stream; its address adds the token as a query. */
export function streamPath(libraryItemId: number): string {
  return `/api/v1/stream/${libraryItemId}`;
}

/** A refresh's position snapshot, with the caller's tag from when it was captured. */
export interface RestoreSnapshot extends PlaybackSnapshot {
  tag: number | null;
}

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
 *
 * `onPermanentFailure` runs when the stream has failed for good: a media
 * error after the one re-mint. `snapshotTag` is read each time a snapshot
 * is captured, and its value comes back with that snapshot's restore.
 */
export function useStreamUrl(
  libraryItemId: number,
  audioRef: RefObject<HTMLAudioElement | null>,
  onPermanentFailure: () => void,
  snapshotTag: () => number | null,
) {
  const [streamUrl, setStreamUrl] = useState<string | null>(null);
  const controllerRef = useRef<StreamTokenController | null>(null);
  const pendingRestoreRef = useRef<RestoreSnapshot | null>(null);
  const onPermanentFailureRef = useRef(onPermanentFailure);
  const snapshotTagRef = useRef(snapshotTag);
  useEffect(() => {
    onPermanentFailureRef.current = onPermanentFailure;
    snapshotTagRef.current = snapshotTag;
  }, [onPermanentFailure, snapshotTag]);

  useEffect(() => {
    setStreamUrl(null);
    pendingRestoreRef.current = null;
    // The tag of each captured snapshot, found again when its URL is ready.
    const tags = new WeakMap<PlaybackSnapshot, number | null>();

    const controller = new StreamTokenController({
      mint: () => mintStreamToken(libraryItemId),
      buildUrl: (token) =>
        `${streamPath(libraryItemId)}?token=${encodeURIComponent(token)}`,
      captureState: () => {
        const snapshot = {
          time: audioRef.current?.currentTime ?? 0,
          wasPlaying: audioRef.current ? !audioRef.current.paused : false,
        };
        tags.set(snapshot, snapshotTagRef.current());
        return snapshot;
      },
      onUrlReady: (url, restoreSnapshot) => {
        pendingRestoreRef.current = restoreSnapshot
          ? { ...restoreSnapshot, tag: tags.get(restoreSnapshot) ?? null }
          : null;
        setStreamUrl(url);
      },
      scheduleRefresh: (delay, cb) => window.setTimeout(cb, delay),
      cancelRefresh: (handle) =>
        window.clearTimeout(handle as ReturnType<typeof window.setTimeout>),
      now: () => Date.now(),
      onPermanentFailure: () => onPermanentFailureRef.current(),
    });

    controllerRef.current = controller;
    void controller.start();

    return () => {
      controller.dispose();
      controllerRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- audioRef is a ref object; libraryItemId is the only real dependency
  }, [libraryItemId]);

  const handleMediaError = useCallback(() => {
    void controllerRef.current?.handleMediaError();
  }, []);

  /** Consume any pending time/play-state restore. Call from onLoadedMetadata. */
  const consumeRestore = useCallback((): RestoreSnapshot | null => {
    const snapshot = pendingRestoreRef.current;
    pendingRestoreRef.current = null;
    return snapshot;
  }, []);

  return { streamUrl, handleMediaError, consumeRestore };
}
