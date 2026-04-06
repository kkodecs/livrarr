import type { SortDir } from "@/hooks/useSort";

/**
 * Sortable table column header. Click to sort, click again to reverse.
 * Shows an arrow indicator when this column is the active sort.
 */
export function SortHeader<F extends string>({
  field,
  activeField,
  dir,
  onSort,
  className,
  children,
}: {
  field: F;
  activeField: F;
  dir: SortDir;
  onSort: (field: F) => void;
  className?: string;
  children: React.ReactNode;
}) {
  const isActive = field === activeField;
  return (
    <th
      className={`cursor-pointer select-none px-3 py-2 text-left text-xs font-medium uppercase text-muted hover:text-zinc-100 ${className ?? ""}`}
      onClick={() => onSort(field)}
    >
      {children}
      {isActive && (dir === "asc" ? " \u2191" : " \u2193")}
    </th>
  );
}
