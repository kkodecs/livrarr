import { useState, useCallback } from "react";

export type SortDir = "asc" | "desc";

export interface SortState<F extends string> {
  field: F;
  dir: SortDir;
  toggle: (field: F) => void;
  sort: <T>(items: T[], accessor: (item: T, field: F) => string | number | null) => T[];
}

/**
 * Generic client-side sort hook. Click a field to sort ascending;
 * click again to reverse. Click a different field to sort ascending on that field.
 */
export function useSort<F extends string>(
  defaultField: F,
  defaultDir: SortDir = "asc",
): SortState<F> {
  const [field, setField] = useState<F>(defaultField);
  const [dir, setDir] = useState<SortDir>(defaultDir);

  const toggle = useCallback(
    (f: F) => {
      if (f === field) {
        setDir((d) => (d === "asc" ? "desc" : "asc"));
      } else {
        setField(f);
        setDir("asc");
      }
    },
    [field],
  );

  const sort = useCallback(
    <T,>(items: T[], accessor: (item: T, field: F) => string | number | null): T[] => {
      return [...items].sort((a, b) => {
        const av = accessor(a, field);
        const bv = accessor(b, field);
        if (av == null && bv == null) return 0;
        if (av == null) return 1;
        if (bv == null) return -1;
        let cmp: number;
        if (typeof av === "string" || typeof bv === "string") {
          cmp = String(av).localeCompare(String(bv));
        } else {
          cmp = av - bv;
        }
        return dir === "desc" ? -cmp : cmp;
      });
    },
    [field, dir],
  );

  return { field, dir, toggle, sort };
}
