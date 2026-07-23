import { describe, it, expect, vi } from "vitest";
import {
  StreamTokenController,
  REFRESH_MARGIN_MS,
  MINT_RETRY_INITIAL_DELAY_MS,
  MINT_RETRY_MAX_DELAY_MS,
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

describe("StreamTokenController — mint-failure retry with bounded backoff (#15)", () => {
  it("surfaces an error and schedules a backoff retry when the INITIAL mint fails", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockRejectedValueOnce(new Error("network down"))
      .mockResolvedValueOnce({ token: "t1", exp: 9_999_999_999 });
    const onInitialMintError = vi.fn();
    const deps = makeDeps({ mint, onInitialMintError });

    const controller = new StreamTokenController(deps);
    await controller.start();

    // (b) the initial mint failure surfaces to the caller immediately —
    // no silent blank player.
    expect(onInitialMintError).toHaveBeenCalledTimes(1);
    expect(onInitialMintError).toHaveBeenCalledWith(expect.any(Error));
    expect(deps.onUrlReady).not.toHaveBeenCalled();

    // (a) ...and the retry chain does not die: a retry is scheduled at the
    // initial backoff delay instead of a bare `return`.
    expect(deps.scheduled).toHaveLength(1);
    expect(deps.scheduled[0]!.delay).toBe(MINT_RETRY_INITIAL_DELAY_MS);

    // Firing the retry recovers normally, with the same "initial mint"
    // semantics (no restoreSnapshot — there was never a prior URL).
    deps.scheduled[0]!.cb();
    await flush();

    expect(mint).toHaveBeenCalledTimes(2);
    expect(deps.onUrlReady).toHaveBeenCalledWith(
      "/api/v1/stream/1?token=t1",
      null,
    );
  });

  it("re-arms with a backoff retry when a PROACTIVE REFRESH mint fails, without surfacing an initial-mint error", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockResolvedValueOnce({ token: "t1", exp: 9_999_999_999 })
      .mockRejectedValueOnce(new Error("network down"));
    const onInitialMintError = vi.fn();
    const deps = makeDeps({ mint, onInitialMintError });

    const controller = new StreamTokenController(deps);
    await controller.start();
    expect(deps.scheduled).toHaveLength(1); // the normal proactive-refresh timer

    // Fire the proactive refresh; its mint rejects.
    deps.scheduled[0]!.cb();
    await flush();

    // (a) the chain doesn't die: a NEW timer is scheduled at the initial
    // backoff delay (the bug: `scheduleProactiveRefresh` only ever ran on
    // the success path, so a refresh failure previously killed the chain
    // for good, leaving only the one-shot `<audio onError>` remint as a
    // partial self-heal).
    expect(deps.scheduled).toHaveLength(2);
    expect(deps.scheduled[1]!.delay).toBe(MINT_RETRY_INITIAL_DELAY_MS);
    // (b) a REFRESH failure (unlike an INITIAL mint failure) does not
    // surface an error — the player is already playing a valid URL.
    expect(onInitialMintError).not.toHaveBeenCalled();
  });

  it("doubles the backoff on each consecutive failure, caps it at 60s, and resets after a success", async () => {
    const mint = vi
      .fn<() => Promise<StreamTokenMint>>()
      .mockRejectedValue(new Error("down"));
    const deps = makeDeps({ mint });

    const controller = new StreamTokenController(deps);
    await controller.start();

    const delays: number[] = [deps.scheduled[0]!.delay];
    for (let i = 0; i < 5; i++) {
      const next = deps.scheduled[deps.scheduled.length - 1]!;
      next.cb();
      await flush();
      delays.push(deps.scheduled[deps.scheduled.length - 1]!.delay);
    }

    expect(delays).toEqual([5_000, 10_000, 20_000, 40_000, 60_000, 60_000]);
    expect(delays[0]).toBe(MINT_RETRY_INITIAL_DELAY_MS);
    expect(delays[4]).toBe(MINT_RETRY_MAX_DELAY_MS);

    // Recover, then fail again — the backoff must restart from the initial
    // delay, not resume climbing (or stay capped) from where it left off.
    mint.mockResolvedValueOnce({ token: "recovered", exp: 9_999_999_999 });
    const lastRetry = deps.scheduled[deps.scheduled.length - 1]!;
    lastRetry.cb();
    await flush();
    expect(deps.onUrlReady).toHaveBeenCalledWith(
      "/api/v1/stream/1?token=recovered",
      null,
    );

    mint.mockRejectedValueOnce(new Error("down again"));
    const proactiveTimer = deps.scheduled[deps.scheduled.length - 1]!; // scheduled by the recovery
    proactiveTimer.cb();
    await flush();
    expect(deps.scheduled[deps.scheduled.length - 1]!.delay).toBe(
      MINT_RETRY_INITIAL_DELAY_MS,
    );
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
