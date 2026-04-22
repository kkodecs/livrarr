import { describe, it, expect } from "vitest";
import {
  computeTotalPages,
  isPrevDisabled,
  isNextDisabled,
} from "./pagination";

describe("computeTotalPages", () => {
  it("returns 6 pages for 51 items with pageSize 10", () => {
    expect(computeTotalPages(51, 10)).toBe(6);
  });

  it("returns 1 page for 10 items with pageSize 10", () => {
    expect(computeTotalPages(10, 10)).toBe(1);
  });

  it("returns 0 for zero items", () => {
    expect(computeTotalPages(0, 10)).toBe(0);
  });

  it("returns 0 for negative total", () => {
    expect(computeTotalPages(-1, 10)).toBe(0);
  });

  it("returns 0 for zero pageSize", () => {
    expect(computeTotalPages(10, 0)).toBe(0);
  });
});

describe("isPrevDisabled", () => {
  it("disables prev on page 1", () => {
    expect(isPrevDisabled(1)).toBe(true);
  });

  it("enables prev on page 2", () => {
    expect(isPrevDisabled(2)).toBe(false);
  });
});

describe("isNextDisabled", () => {
  it("disables next on last page", () => {
    expect(isNextDisabled(6, 6)).toBe(true);
  });

  it("enables next before last page", () => {
    expect(isNextDisabled(5, 6)).toBe(false);
  });

  it("disables next on single page", () => {
    expect(isNextDisabled(1, 1)).toBe(true);
  });

  it("disables both on zero pages", () => {
    expect(isPrevDisabled(1)).toBe(true);
    expect(isNextDisabled(1, 0)).toBe(true);
  });
});
