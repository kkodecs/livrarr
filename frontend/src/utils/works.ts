import type { WorkDetailResponse } from "@/types/api";

export type WorkSortField = "title" | "authorName" | "year" | "addedAt";

export function sortWorks(
  works: WorkDetailResponse[],
  field: WorkSortField,
  dir: "asc" | "desc",
): WorkDetailResponse[] {
  const sorted = [...works].sort((a, b) => {
    let cmp = 0;
    switch (field) {
      case "title":
        cmp = (a.sortTitle ?? a.title).localeCompare(b.sortTitle ?? b.title);
        break;
      case "authorName":
        cmp = a.authorName.localeCompare(b.authorName);
        break;
      case "year":
        cmp = (a.year ?? 0) - (b.year ?? 0);
        break;
      case "addedAt":
        cmp = a.addedAt.localeCompare(b.addedAt);
        break;
    }
    return dir === "desc" ? -cmp : cmp;
  });
  return sorted;
}

/**
 * Look up a work title by ID from a list of works.
 * Returns the title, or a fallback string if not found.
 */
export function workName(
  works: WorkDetailResponse[] | undefined,
  id: number | null,
): string {
  if (!id) return "\u2014";
  const w = works?.find((w) => w.id === id);
  return w ? w.title : `Work #${id}`;
}
