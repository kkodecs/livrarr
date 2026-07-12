import { describe, it, expect } from "vitest";
import { deriveEnrichmentPillState } from "./enrichmentPill";

const SETTLED_STATUSES = ["unenriched", "enriched", "thin", "failed"] as const;

describe("deriveEnrichmentPillState", () => {
  describe("enriching = true", () => {
    for (const status of SETTLED_STATUSES) {
      it(`is "fetching" regardless of enrichmentStatus ("${status}")`, () => {
        expect(deriveEnrichmentPillState(true, status)).toBe("fetching");
      });
    }

    it('is "fetching" even for an unrecognized status', () => {
      expect(deriveEnrichmentPillState(true, "some-future-status")).toBe("fetching");
    });
  });

  describe("enriching = false", () => {
    it('"enriched" is "complete"', () => {
      expect(deriveEnrichmentPillState(false, "enriched")).toBe("complete");
    });

    it('"thin" is "complete" — never presented as an error', () => {
      expect(deriveEnrichmentPillState(false, "thin")).toBe("complete");
    });

    it('"failed" is "attention"', () => {
      expect(deriveEnrichmentPillState(false, "failed")).toBe("attention");
    });

    it('"unenriched" is "attention" — needs recovery, not a permanent spinner', () => {
      expect(deriveEnrichmentPillState(false, "unenriched")).toBe("attention");
    });

    it('an unrecognized status defaults to "attention", never "complete"', () => {
      expect(deriveEnrichmentPillState(false, "some-future-status")).toBe("attention");
    });
  });
});
