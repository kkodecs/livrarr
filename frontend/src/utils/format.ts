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
  let date = parseISO(iso);
  // parseISO returns Invalid Date for non-ISO formats (e.g. RFC 2822 from Torznab).
  // Fall back to native Date() which handles RFC 2822.
  if (isNaN(date.getTime())) date = new Date(iso);
  if (isNaN(date.getTime())) return iso;
  return format(date, "MMM d, yyyy HH:mm");
}

export function formatUptime(startupIso: string, now: Date = new Date()): string {
  let start = parseISO(startupIso);
  if (isNaN(start.getTime())) start = new Date(startupIso);
  if (isNaN(start.getTime())) return startupIso;
  const totalMinutes = Math.max(0, Math.floor((now.getTime() - start.getTime()) / 60000));
  if (totalMinutes === 0) return "under a minute";
  const days = Math.floor(totalMinutes / 1440);
  const hours = Math.floor((totalMinutes % 1440) / 60);
  const minutes = totalMinutes % 60;
  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
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

export function getCoverUrl(workId: number, v?: number, mediaType?: "ebook" | "audiobook"): string {
  const file = mediaType === "audiobook" ? "audiocover.jpg" : "cover.jpg";
  const base = `/api/v1/mediacover/${workId}/${file}`;
  return v ? `${base}?v=${v}` : base;
}

export function getCoverThumbUrl(workId: number, v?: number, mediaType?: "ebook" | "audiobook"): string {
  const file = mediaType === "audiobook" ? "audiocover_thumb.jpg" : "thumb.jpg";
  const base = `/api/v1/mediacover/${workId}/${file}`;
  return v ? `${base}?v=${v}` : base;
}
