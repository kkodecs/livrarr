import { ChevronLeft, ChevronRight } from "lucide-react";
import { isPrevDisabled, isNextDisabled } from "@/utils/pagination";

interface PaginationProps {
  page: number;
  totalPages: number;
  total: number;
  pageSize: number;
  onPageChange: (page: number) => void;
}

export function Pagination({
  page,
  totalPages,
  total,
  pageSize,
  onPageChange,
}: PaginationProps) {
  if (totalPages <= 1) return null;

  const start = (page - 1) * pageSize + 1;
  const end = Math.min(page * pageSize, total);

  return (
    <div className="flex items-center justify-end gap-4 text-sm text-muted">
      <button
        onClick={() => onPageChange(Math.max(1, page - 1))}
        disabled={isPrevDisabled(page)}
        className="rounded p-1 hover:text-zinc-100 disabled:opacity-30"
      >
        <ChevronLeft size={16} />
      </button>
      <span>
        {start}-{end} of {total}
      </span>
      <button
        onClick={() => onPageChange(Math.min(totalPages, page + 1))}
        disabled={isNextDisabled(page, totalPages)}
        className="rounded p-1 hover:text-zinc-100 disabled:opacity-30"
      >
        <ChevronRight size={16} />
      </button>
    </div>
  );
}
