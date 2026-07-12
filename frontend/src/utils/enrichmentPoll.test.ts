import { describe, it, expect } from "vitest";
import { nextEnrichmentPollIntervalMs } from "./enrichmentPoll";

describe("nextEnrichmentPollIntervalMs", () => {
  describe("1.5s phase (0ms - <15s)", () => {
    it("returns 1500 at the very start", () => {
      expect(nextEnrichmentPollIntervalMs(0)).toBe(1_500);
    });

    it("returns 1500 mid-phase", () => {
      expect(nextEnrichmentPollIntervalMs(7_500)).toBe(1_500);
    });

    it("returns 1500 just before the 15s boundary", () => {
      expect(nextEnrichmentPollIntervalMs(14_999)).toBe(1_500);
    });
  });

  describe("5s phase (15s - <60s)", () => {
    it("switches to 5000 exactly at the 15s boundary", () => {
      expect(nextEnrichmentPollIntervalMs(15_000)).toBe(5_000);
    });

    it("returns 5000 mid-phase", () => {
      expect(nextEnrichmentPollIntervalMs(40_000)).toBe(5_000);
    });

    it("returns 5000 just before the 60s cap", () => {
      expect(nextEnrichmentPollIntervalMs(59_999)).toBe(5_000);
    });
  });

  describe("60s hard cap", () => {
    it("stops polling exactly at 60000ms", () => {
      expect(nextEnrichmentPollIntervalMs(60_000)).toBe(false);
    });

    it("stays stopped well past the cap", () => {
      expect(nextEnrichmentPollIntervalMs(120_000)).toBe(false);
    });
  });
});
