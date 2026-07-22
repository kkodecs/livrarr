// Unit C — scoped, expiring stream token: the client-side state machine.
//
// Framework-free by design (no React, no DOM) so it can be unit tested
// directly with plain mocked callbacks. `useStreamUrl.ts` is the thin React
// wrapper that wires this to a real `<audio>` element and the mint API call.

export interface StreamTokenMint {
  token: string;
  /** Unix-epoch seconds. */
  exp: number;
}

/** Snapshot of the audio element's state, captured before swapping `src`. */
export interface PlaybackSnapshot {
  time: number;
  wasPlaying: boolean;
}

export interface StreamTokenControllerDeps {
  mint: () => Promise<StreamTokenMint>;
  buildUrl: (token: string) => string;
  captureState: () => PlaybackSnapshot;
  /**
   * Called whenever a new URL is ready. `restoreSnapshot` is non-null only
   * for a refresh (proactive or error-triggered) — never for the initial
   * mint, since there is nothing to restore yet.
   */
  onUrlReady: (url: string, restoreSnapshot: PlaybackSnapshot | null) => void;
  scheduleRefresh: (delayMs: number, cb: () => void) => unknown;
  cancelRefresh: (handle: unknown) => void;
  /** Current time in ms since epoch. Injectable for tests. */
  now: () => number;
  /** Called after a second (permanent) media-error failure. */
  onPermanentFailure?: () => void;
}

/** Refresh this long before the token's actual expiry. */
export const REFRESH_MARGIN_MS = 5 * 60 * 1000;
/** Never schedule a refresh sooner than this, even if exp is imminent. */
const MIN_REFRESH_DELAY_MS = 1000;

/**
 * Drives one library item's stream token for the lifetime of an
 * `<audio>` mount: mints, proactively refreshes before expiry, and — on a
 * playback error — reminds at most once, preserving position and play
 * state. A second error after that one remint is treated as permanent
 * (loop-prevention: this can never remint more than once per mount).
 */
export class StreamTokenController {
  private readonly deps: StreamTokenControllerDeps;
  private timerHandle: unknown = null;
  private errorRemintUsed = false;
  private disposed = false;

  constructor(deps: StreamTokenControllerDeps) {
    this.deps = deps;
  }

  /** Mint the initial token for this mount. */
  async start(): Promise<void> {
    await this.mintAndSchedule(false);
  }

  /** Wire to the `<audio onError>` handler. */
  async handleMediaError(): Promise<void> {
    if (this.disposed) return;
    if (this.errorRemintUsed) {
      this.deps.onPermanentFailure?.();
      return;
    }
    this.errorRemintUsed = true;
    await this.mintAndSchedule(true);
  }

  /** Cancel any pending timer / stale in-flight mint. Call on unmount. */
  dispose(): void {
    this.disposed = true;
    this.clearTimer();
  }

  private async mintAndSchedule(isRefresh: boolean): Promise<void> {
    const snapshot = isRefresh ? this.deps.captureState() : null;
    let result: StreamTokenMint;
    try {
      result = await this.deps.mint();
    } catch {
      // Leave the current URL (if any) in place; the caller can retry via
      // the same error path (proactive refresh will also try again next
      // cycle if this was that path).
      return;
    }
    if (this.disposed) return;

    const url = this.deps.buildUrl(result.token);
    this.deps.onUrlReady(url, snapshot);
    this.scheduleProactiveRefresh(result.exp);
  }

  private scheduleProactiveRefresh(expSeconds: number): void {
    this.clearTimer();
    const delay = Math.max(
      MIN_REFRESH_DELAY_MS,
      expSeconds * 1000 - this.deps.now() - REFRESH_MARGIN_MS,
    );
    this.timerHandle = this.deps.scheduleRefresh(delay, () => {
      void this.mintAndSchedule(true);
    });
  }

  private clearTimer(): void {
    if (this.timerHandle != null) {
      this.deps.cancelRefresh(this.timerHandle);
      this.timerHandle = null;
    }
  }
}
