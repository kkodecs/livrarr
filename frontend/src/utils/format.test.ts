import { describe, expect, it } from "vitest";
import { formatUptime } from "@/utils/format";

const NOW = new Date("2026-10-10T12:00:00Z");
const S = 1000;
const M = 60 * S;
const H = 60 * M;
const D = 24 * H;

const ago = (ms: number) => new Date(NOW.getTime() - ms).toISOString();

describe("formatUptime", () => {
  it.each([
    ["30s", 30 * S, "under a minute"],
    ["59s", 59 * S, "under a minute"],
    ["exactly 1m", M, "1m"],
    ["15m", 15 * M, "15m"],
    ["3h15m", 3 * H + 15 * M, "3h 15m"],
    ["2d3h15m", 2 * D + 3 * H + 15 * M, "2d 3h 15m"],
    ["1d0h5m", D + 5 * M, "1d 0h 5m"],
    ["exactly 1d", D, "1d 0h 0m"],
  ])("%s -> %s", (_name, ms, expected) => {
    expect(formatUptime(ago(ms), NOW)).toBe(expected);
  });

  it("treats a start time in the future as under a minute", () => {
    expect(formatUptime(ago(-10 * M), NOW)).toBe("under a minute");
  });

  it("returns an unparsable string unchanged", () => {
    expect(formatUptime("not a date", NOW)).toBe("not a date");
  });
});
