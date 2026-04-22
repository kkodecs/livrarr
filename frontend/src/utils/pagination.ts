export function computeTotalPages(total: number, pageSize: number): number {
  if (total <= 0 || pageSize <= 0) return 0;
  return Math.ceil(total / pageSize);
}

export function isPrevDisabled(page: number): boolean {
  return page <= 1;
}

export function isNextDisabled(page: number, totalPages: number): boolean {
  return page >= totalPages;
}
