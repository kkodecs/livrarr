/**
 * Format Utilities — Implementation Tests
 *
 * Tests the real formatting functions with edge cases and boundary values.
 */
import { describe, it, expect } from "vitest";
import {
  formatBytes,
  formatDuration,
  formatEta,
  getCoverUrl,
  formatRelativeDate,
  formatAbsoluteDate,
} from "@/utils/format";

describe("formatBytes", () => {
  it("0 bytes -> '0 B'", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  it("1023 bytes -> '1023 B' (not '1 KB')", () => {
    expect(formatBytes(1023)).toBe("1023 B");
  });

  it("1024 bytes -> '1 KB'", () => {
    expect(formatBytes(1024)).toBe("1 KB");
  });

  it("1536 bytes -> '1.5 KB'", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("1048576 -> '1 MB'", () => {
    expect(formatBytes(1048576)).toBe("1 MB");
  });

  it("1073741824 -> '1 GB'", () => {
    expect(formatBytes(1073741824)).toBe("1 GB");
  });

  it("1099511627776 -> '1 TB'", () => {
    expect(formatBytes(1099511627776)).toBe("1 TB");
  });
});

describe("formatDuration", () => {
  it("0 seconds -> '0m'", () => {
    expect(formatDuration(0)).toBe("0m");
  });

  it("59 seconds -> '0m'", () => {
    expect(formatDuration(59)).toBe("0m");
  });

  it("60 seconds -> '1m'", () => {
    expect(formatDuration(60)).toBe("1m");
  });

  it("3661 seconds -> '1h 1m'", () => {
    expect(formatDuration(3661)).toBe("1h 1m");
  });
});

describe("formatEta", () => {
  it("null -> em dash", () => {
    expect(formatEta(null)).toBe("\u2014");
  });

  it("120 -> '2m'", () => {
    expect(formatEta(120)).toBe("2m");
  });
});

describe("getCoverUrl", () => {
  it("returns correct path pattern", () => {
    expect(getCoverUrl(42)).toBe("/api/v1/mediacover/42/cover.jpg");
    expect(getCoverUrl(1)).toBe("/api/v1/mediacover/1/cover.jpg");
  });
});

describe("formatRelativeDate", () => {
  it("recent ISO date contains 'ago'", () => {
    // Use a date a few minutes in the past
    const recent = new Date(Date.now() - 5 * 60 * 1000).toISOString();
    const result = formatRelativeDate(recent);
    expect(result).toContain("ago");
  });
});

describe("formatAbsoluteDate", () => {
  it("ISO date -> formatted string", () => {
    const result = formatAbsoluteDate("2026-03-31T14:30:00Z");
    // Should contain month, day, year, and time components
    expect(result).toContain("Mar");
    expect(result).toContain("31");
    expect(result).toContain("2026");
  });
});
