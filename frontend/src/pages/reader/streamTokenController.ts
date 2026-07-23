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
  /**
   * Called when the very first mint (on `start()`) fails, so the caller can
   * surface an error immediately instead of leaving a blank player while
   * the retry below keeps trying in the background.
   */
  onInitialMintError?: (error: unknown) => void;
}

/** Refresh this long before the token's actual expiry. */
export const REFRESH_MARGIN_MS = 5 * 60 * 1000;
/** Never schedule a refresh sooner than this, even if exp is imminent. */
const MIN_REFRESH_DELAY_MS = 1000;
/** A failed mint/refresh retries after this long, doubling on each further
 *  consecutive failure — reset the moment a mint succeeds again. */
export const MINT_RETRY_INITIAL_DELAY_MS = 5 * 1000;
/** ...never waiting longer than this between retries. */
export const MINT_RETRY_MAX_DELAY_MS = 60 * 1000;

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
  private consecutiveMintFailures = 0;

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
    } catch (err) {
      if (this.disposed) return;
      // Leave the current URL (if any) in place, but never leave the cycle
      // dead: re-arm with bounded exponential backoff so a transient outage
      // self-heals instead of requiring a media error (or a reload) to
      // recover. An INITIAL mint failure (no URL has ever been ready) also
      // surfaces immediately — otherwise the player just stays blank with
      // no indication anything is wrong.
      if (!isRefresh) {
        this.deps.onInitialMintError?.(err);
      }
      this.scheduleMintRetry(isRefresh);
      return;
    }
    if (this.disposed) return;

    this.consecutiveMintFailures = 0;
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

  private scheduleMintRetry(isRefresh: boolean): void {
    const delay = Math.min(
      MINT_RETRY_MAX_DELAY_MS,
      MINT_RETRY_INITIAL_DELAY_MS * 2 ** this.consecutiveMintFailures,
    );
    this.consecutiveMintFailures += 1;
    this.clearTimer();
    this.timerHandle = this.deps.scheduleRefresh(delay, () => {
      void this.mintAndSchedule(isRefresh);
    });
  }

  private clearTimer(): void {
    if (this.timerHandle != null) {
      this.deps.cancelRefresh(this.timerHandle);
      this.timerHandle = null;
    }
  }
}
