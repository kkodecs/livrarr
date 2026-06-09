import { EpubCFI } from "epubjs";
import type { AnchorDTO } from "@/types/api";

const cfiComparator = new EpubCFI();

/**
 * Returns the audio timestamp of the last anchor whose CFI is at or before
 * the given CFI (i.e. the nearest anchor at-or-before in reading order).
 * Returns 0 when the given CFI precedes every anchor.
 * Anchors must arrive in ascending ts order (server-guaranteed).
 */
export function resolveTsForCfi(anchors: AnchorDTO[], cfi: string): number {
  // Binary search for the last anchor with compare(anchor.cfi, cfi) <= 0.
  let lo = 0;
  let hi = anchors.length - 1;
  let result = -1;

  while (lo <= hi) {
    const mid = (lo + hi) >>> 1;
    const cmp = cfiComparator.compare(anchors[mid]!.cfi, cfi);
    if (cmp <= 0) {
      result = mid;
      lo = mid + 1;
    } else {
      hi = mid - 1;
    }
  }

  return result >= 0 ? anchors[result]!.ts : 0;
}
