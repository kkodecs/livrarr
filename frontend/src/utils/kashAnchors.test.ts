import { describe, it, expect } from "vitest";
import { resolveTsForCfi } from "./kashAnchors";
import type { AnchorDTO } from "@/types/api";

// CFI strings ordered by EpubCFI.compare (verified against epubjs 0.3.93):
//   /6/2!/4/2  < /6/4!/4/2  < /6/8!/4/2  (different spine positions)
//   /6/2!/2/2  < /6/2!/4/2              (earlier node path within same spine)
//   /6/2!/4/2  < /6/2!/4/4              (later sibling within same spine)
const CFI_A = "epubcfi(/6/2!/4/2)";
const CFI_B = "epubcfi(/6/4!/4/2)";
const CFI_C = "epubcfi(/6/8!/4/2)";

// Between A and B: same spine as A but later sibling node
const CFI_MID_AB = "epubcfi(/6/2!/4/4)";
// Before A: earlier node path within the same spine section
const CFI_BEFORE_A = "epubcfi(/6/2!/2/2)";

const anchors: AnchorDTO[] = [
  { cfi: CFI_A, ts: 10 },
  { cfi: CFI_B, ts: 50 },
  { cfi: CFI_C, ts: 90 },
];

describe("resolveTsForCfi", () => {
  it("returns the anchor ts for an exact-match CFI", () => {
    expect(resolveTsForCfi(anchors, CFI_A)).toBe(10);
    expect(resolveTsForCfi(anchors, CFI_B)).toBe(50);
    expect(resolveTsForCfi(anchors, CFI_C)).toBe(90);
  });

  it("returns the preceding anchor ts for a CFI between two anchors", () => {
    expect(resolveTsForCfi(anchors, CFI_MID_AB)).toBe(10);
  });

  it("returns 0 when the CFI is before the first anchor", () => {
    expect(resolveTsForCfi(anchors, CFI_BEFORE_A)).toBe(0);
  });

  it("returns 0 for an empty anchors array", () => {
    expect(resolveTsForCfi([], CFI_A)).toBe(0);
  });

  it("returns the last anchor ts for a CFI beyond the last anchor", () => {
    const beyond = "epubcfi(/6/12!/4/2)";
    expect(resolveTsForCfi(anchors, beyond)).toBe(90);
  });
});
