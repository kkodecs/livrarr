import { formatDistanceToNow, format, parseISO } from "date-fns";

export function formatRelativeDate(dateStr: string): string {
  const d = parseISO(dateStr);
  // parseISO returns Invalid Date for non-ISO formats (e.g. RFC 2822 from Torznab).
  // Fall back to native Date() which handles RFC 2822.
  const date = isNaN(d.getTime()) ? new Date(dateStr) : d;
  if (isNaN(date.getTime())) return dateStr;
  return formatDistanceToNow(date, { addSuffix: true });
}

export function formatAbsoluteDate(iso: string): string {
  return format(parseISO(iso), "MMM d, yyyy HH:mm");
}

export function formatMB(bytes: number): string {
  if (bytes === 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${mb.toFixed(1)} MB`;
  return `${Math.round(mb)} MB`;
}

export function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(1))} ${sizes[i]}`;
}

export function formatDuration(seconds: number): string {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  if (h > 0) return `${h}h ${m}m`;
  return `${m}m`;
}

export function formatEta(seconds: number | null): string {
  if (seconds === null) return "\u2014";
  return formatDuration(seconds);
}

export function getCoverUrl(workId: number, v?: number): string {
  const base = `/api/v1/mediacover/${workId}/cover.jpg`;
  return v ? `${base}?v=${v}` : base;
}

export function getCoverThumbUrl(workId: number): string {
  return `/api/v1/mediacover/${workId}/thumb.jpg`;
}
