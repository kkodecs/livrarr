import { describe, it, expect, vi } from "vitest";
import {
  StreamTokenController,
  REFRESH_MARGIN_MS,
  type StreamTokenControllerDeps,
  type StreamTokenMint,
  type PlaybackSnapshot,
} from "./streamTokenController";

/** Flush pending microtasks (the controller's internal fire-and-forget awaits). */
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function makeDeps(
  overrides: Partial<StreamTokenControllerDeps> = {},
): StreamTokenControllerDeps & {
  scheduled: Array<{ delay: number; cb: () => void }>;
} {
  const scheduled: Array<{ delay: number; cb: () => void }> = [];
  const deps: StreamTokenControllerDeps & {
    scheduled: Array<{ delay: number; cb: () => void }>;
  } = {
    mint: vi.fn(),
    buildUrl: (token: string) => `/api/v1/stream/1?token=${token}`,
    captureState: (): PlaybackSnapshot => ({ time: 0, wasPlaying: false }),
    onUrlReady: vi.fn(),
    scheduleRefresh: (delay: number, cb: () => void) => {
      scheduled.push({ delay, cb });
      return scheduled.length;
    },
    cancelRefresh: vi.fn(),
    now: () => 0,
    scheduled,
    ...overrides,
  };
  return deps;
}

describe("StreamTokenController — proactive refresh", () => {
  it("schedules a refresh REFRESH_MARGIN_MS before the token's expiry", async () => {
    const nowMs = Date.parse("2026-01-01T00:00:00Z");
    const firstExpSeconds = nowMs / 1000 + 3600; // 1h TTL for this test
    const secondExpSeconds = firstExpSeconds + 3600;
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValueOnce({ token: "t1", exp: firstExpSeconds })
      .mockResolvedValueOnce({ token: "t2", exp: secondExpSeconds });
    const deps = makeDeps({ mint, now: () => nowMs });

    const controller = new StreamTokenController(deps);
    await controller.start();

    expect(mint).toHaveBeenCalledTimes(1);
    expect(deps.onUrlReady).toHaveBeenCalledWith(
      "/api/v1/stream/1?token=t1",
      null,
    );
    expect(deps.scheduled).toHaveLength(1);
    expect(deps.scheduled[0]!.delay).toBe(3600_000 - REFRESH_MARGIN_MS);

    // Fire the scheduled proactive refresh.
    deps.scheduled[0]!.cb();
    await flush();

    expect(mint).toHaveBeenCalledTimes(2);
    expect(deps.onUrlReady).toHaveBeenLastCalledWith(
      "/api/v1/stream/1?token=t2",
      { time: 0, wasPlaying: false },
    );
    // Reschedules for the new token too.
    expect(deps.scheduled).toHaveLength(2);
  });

  it("clamps the delay to a minimum instead of firing immediately/negatively", async () => {
    const nowMs = 1_000_000;
    // exp is only 10 seconds out — far less than the 5-minute margin.
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValue({ token: "t1", exp: nowMs / 1000 + 10 });
    const deps = makeDeps({ mint, now: () => nowMs });

    await new StreamTokenController(deps).start();

    expect(deps.scheduled[0]!.delay).toBeGreaterThan(0);
  });
});

describe("StreamTokenController — media-error remint with loop-prevention", () => {
  it("reminst at most once per mount, preserving position and play state", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValueOnce({ token: "t1", exp: 9_999_999_999 })
      .mockResolvedValueOnce({ token: "t2", exp: 9_999_999_999 });
    const captureState = vi
      .fn<() => PlaybackSnapshot>()
      .mockReturnValue({ time: 42.5, wasPlaying: true });
    const onPermanentFailure = vi.fn();
    const deps = makeDeps({ mint, captureState, onPermanentFailure });

    const controller = new StreamTokenController(deps);
    await controller.start();
    expect(captureState).not.toHaveBeenCalled(); // never captured on initial mint

    await controller.handleMediaError();
    expect(mint).toHaveBeenCalledTimes(2);
    expect(captureState).toHaveBeenCalledTimes(1);
    expect(deps.onUrlReady).toHaveBeenLastCalledWith(
      "/api/v1/stream/1?token=t2",
      { time: 42.5, wasPlaying: true },
    );
    expect(onPermanentFailure).not.toHaveBeenCalled();

    // A second error is permanent — no third remint.
    await controller.handleMediaError();
    expect(mint).toHaveBeenCalledTimes(2);
    expect(onPermanentFailure).toHaveBeenCalledTimes(1);
  });

  it("treats a third error the same as the second — still no further remint", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValue({ token: "t1", exp: 9_999_999_999 });
    const onPermanentFailure = vi.fn();
    const deps = makeDeps({ mint, onPermanentFailure });
    const controller = new StreamTokenController(deps);
    await controller.start();

    await controller.handleMediaError(); // 1st error — consumes the one remint
    await controller.handleMediaError(); // 2nd error — permanent
    await controller.handleMediaError(); // 3rd error — still permanent, no new attempt

    expect(mint).toHaveBeenCalledTimes(2); // initial + exactly one remint
    expect(onPermanentFailure).toHaveBeenCalledTimes(2); // 2nd and 3rd calls only
  });
});

describe("StreamTokenController — dispose cancels stale work", () => {
  it("ignores an in-flight mint that resolves after dispose (item-change cancellation)", async () => {
    let resolveMint: ((v: StreamTokenMint) => void) | undefined;
    const mint = vi.fn<() => Promise<StreamTokenMint>>(
      () =>
        new Promise((resolve) => {
          resolveMint = resolve;
        }),
    );
    const deps = makeDeps({ mint });
    const controller = new StreamTokenController(deps);

    const startPromise = controller.start();
    controller.dispose();
    resolveMint?.({ token: "late", exp: 9_999_999_999 });
    await startPromise;

    expect(deps.onUrlReady).not.toHaveBeenCalled();
    expect(deps.scheduled).toHaveLength(0);
  });

  it("clears a pending proactive-refresh timer on dispose", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValue({ token: "t1", exp: 9_999_999_999 });
    const cancelRefresh = vi.fn();
    const deps = makeDeps({ mint, cancelRefresh });
    const controller = new StreamTokenController(deps);
    await controller.start();

    controller.dispose();

    expect(cancelRefresh).toHaveBeenCalledTimes(1);
  });

  it("ignores a media error after dispose", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValue({ token: "t1", exp: 9_999_999_999 });
    const deps = makeDeps({ mint });
    const controller = new StreamTokenController(deps);
    await controller.start();
    controller.dispose();

    await controller.handleMediaError();

    expect(mint).toHaveBeenCalledTimes(1); // no remint attempted post-dispose
  });
});
