import type { Page } from "@playwright/test";

// Read-only views of the EPUB reader's live epub.js rendition.

/**
 * The start CFI of the rendition's current location. react-reader keeps the
 * epub.js rendition on its viewer component; this walks from the viewer's DOM
 * node to that component and reads `rendition.location` without changing it.
 */
export async function renditionStartCfi(page: Page): Promise<string | null> {
  return page.evaluate(() => {
    const viewer = document.querySelector(".epub-container")?.parentElement;
    if (!viewer) return null;
    const key = Object.keys(viewer).find((k) => k.startsWith("__reactFiber$"));
    type Fiber = {
      return: Fiber | null;
      stateNode?: { rendition?: { location?: { start?: { cfi?: string } } } };
    };
    let fiber = key ? (viewer as unknown as Record<string, Fiber>)[key] : null;
    while (fiber) {
      const rendition = fiber.stateNode?.rendition;
      if (rendition) return rendition.location?.start?.cfi ?? null;
      fiber = fiber.return;
    }
    return null;
  });
}

/**
 * Whether `cfi` lies within the rendition's displayed range, from its current
 * location's start to its end, compared by epub.js's own CFI ordering. Reads
 * the rendition the same way as `renditionStartCfi`; null when there is none.
 */
export async function renditionShows(
  page: Page,
  cfi: string,
): Promise<boolean | null> {
  return page.evaluate((target) => {
    const viewer = document.querySelector(".epub-container")?.parentElement;
    if (!viewer) return null;
    const key = Object.keys(viewer).find((k) => k.startsWith("__reactFiber$"));
    type Rendition = {
      location?: { start?: { cfi?: string }; end?: { cfi?: string } };
      epubcfi: { compare: (a: string, b: string) => number };
    };
    type Fiber = { return: Fiber | null; stateNode?: { rendition?: Rendition } };
    let fiber = key ? (viewer as unknown as Record<string, Fiber>)[key] : null;
    while (fiber) {
      const rendition = fiber.stateNode?.rendition;
      if (rendition) {
        const start = rendition.location?.start?.cfi;
        const end = rendition.location?.end?.cfi;
        if (!start || !end) return null;
        return (
          rendition.epubcfi.compare(start, target) <= 0 &&
          rendition.epubcfi.compare(target, end) <= 0
        );
      }
      fiber = fiber.return;
    }
    return null;
  }, cfi);
}

/** The integer steps of a point CFI, in document order. */
export function cfiSteps(cfi: string): number[] {
  const inner = cfi.replace(/^epubcfi\(/, "").replace(/\)$/, "");
  return (inner.replace(/\[[^\]]*\]/g, "").match(/\d+/g) ?? []).map(Number);
}

export function cfiAfter(later: string, earlier: string): boolean {
  const a = cfiSteps(later);
  const b = cfiSteps(earlier);
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const x = a[i] ?? -1;
    const y = b[i] ?? -1;
    if (x !== y) return x > y;
  }
  return false;
}

/** The spine step of a CFI: which section of the book it is in. */
export function spineStep(cfi: string | null): number | null {
  if (!cfi) return null;
  return cfiSteps(cfi)[1] ?? null;
}
