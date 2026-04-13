import { useState, useEffect, useRef } from "react";
import {
  Bell,
  BookPlus,
  BookCheck,
  RefreshCw,
  Layers,
  AlertOctagon,
  AlertTriangle,
  FolderX,
  Trash2,
  Check,
  Rss,
} from "lucide-react";
import { Link } from "react-router";
import * as Popover from "@radix-ui/react-popover";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import * as api from "@/api";
import type { NotificationType, NotificationResponse } from "@/types/api";
import { formatRelativeDate } from "@/utils/format";
import { LoadingSpinner } from "@/components/Page/LoadingSpinner";
import { useUIStore } from "@/stores/ui";
import type { ReactNode } from "react";

const notificationIcons: Record<NotificationType, ReactNode> = {
  newWorkDetected: <BookPlus size={16} className="text-blue-400" />,
  workAutoAdded: <BookCheck size={16} className="text-green-400" />,
  metadataUpdated: <RefreshCw size={16} className="text-yellow-400" />,
  bulkEnrichmentComplete: <Layers size={16} className="text-purple-400" />,
  jobPanicked: <AlertOctagon size={16} className="text-red-400" />,
  rateLimitHit: <AlertTriangle size={16} className="text-orange-400" />,
  pathNotFound: <FolderX size={16} className="text-red-400" />,
  rssGrabbed: <Rss size={16} className="text-green-400" />,
  rssGrabFailed: <Rss size={16} className="text-red-400" />,
};

export function NotificationBell() {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();
  const toastedIds = useRef<Set<number>>(new Set());
  const setRpmHighlight = useUIStore((s) => s.setRpmHighlight);

  const { data: unreadNotifications } = useQuery({
    queryKey: ["notifications", "unread"],
    queryFn: () => api.listNotifications(true),
    select: (res) => res.items,
    refetchInterval: 30_000,
    refetchOnWindowFocus: true,
    staleTime: 0,
  });

  // Show persistent toast for pathNotFound notifications (once per notification).
  useEffect(() => {
    if (!unreadNotifications) return;
    for (const n of unreadNotifications) {
      if (n.notificationType === "pathNotFound" && !toastedIds.current.has(n.id)) {
        toastedIds.current.add(n.id);
        const d = (n.data ?? {}) as Record<string, unknown>;
        const title = (d.title as string) || "";
        const configuredRemote = (d.configuredRemotePath as string) || "NOT SET";
        const configuredLocal = (d.configuredLocalPath as string) || "NOT SET";
        const remoteHost = (d.clientHost as string) || "NOT SET";
        const contentDir = (d.contentDir as string) || "unknown";
        const redactedPath = `${contentDir}/[REDACTED grab ${d.grabId}]`;
        const question = `My download client ${d.clientName} completed a download but Livrarr says the file is not available locally.\n\nDownload client path: ${redactedPath}\nConfigured Remote Path: ${configuredRemote}\nConfigured Local Path: ${configuredLocal}\nRemote Host: ${remoteHost}\n\nHow do I set up a remote path mapping to fix this?`;
        toast.error(
          <div>
            {String(d.clientName)} reports that {title ? <strong className="font-semibold text-white">{title}</strong> : <strong className="font-semibold text-white">Grab {String(d.grabId)}</strong>} (grab {String(d.grabId)} in the <a href="/activity/queue" className="text-brand hover:underline">queue</a>) has downloaded, but it does not seem to be available locally. You may need a remote path mapping.
          </div>,
          {
            duration: Infinity,
            description: (
              <ul className="mt-1.5 space-y-0.5 text-xs list-disc pl-4">
                <li>
                  <a href="/settings/mediamanagement" onClick={() => setRpmHighlight(true)} className="text-brand hover:underline">
                    Configure path mapping
                  </a>
                </li>
                <li>
                  <a href={`/help?question=${encodeURIComponent(question)}`} className="text-brand hover:underline">
                    Get AI help
                  </a>
                </li>
              </ul>
            ),
          },
        );
      }
    }
  }, [unreadNotifications]);

  const { data: allNotifications, isLoading } = useQuery({
    queryKey: ["notifications", "all"],
    queryFn: () => api.listNotifications(false),
    select: (res) => res.items,
    enabled: open,
  });

  const markRead = useMutation({
    mutationFn: api.markNotificationRead,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });

  const dismiss = useMutation({
    mutationFn: api.dismissNotification,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });

  const dismissAll = useMutation({
    mutationFn: api.dismissAllNotifications,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["notifications"] });
    },
  });

  const unreadCount = unreadNotifications?.length ?? 0;

  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger className="relative rounded p-1.5 text-zinc-400 hover:bg-surface-hover hover:text-zinc-100">
        <Bell size={18} />
        {unreadCount > 0 && (
          <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-brand px-1 text-[10px] font-bold text-white">
            {unreadCount > 99 ? "99+" : unreadCount}
          </span>
        )}
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          align="end"
          className="z-50 w-80 rounded border border-border bg-zinc-800 shadow-xl"
        >
          <div className="flex items-center justify-between border-b border-border px-4 py-3">
            <span className="text-sm font-medium text-zinc-100">
              Notifications
            </span>
            {(allNotifications?.length ?? 0) > 0 && (
              <button
                onClick={() => dismissAll.mutate()}
                className="text-xs text-muted hover:text-zinc-100"
                disabled={dismissAll.isPending}
              >
                Dismiss All
              </button>
            )}
          </div>
          <div className="max-h-80 overflow-y-auto">
            {isLoading ? (
              <div className="flex justify-center py-8">
                <LoadingSpinner />
              </div>
            ) : !allNotifications?.length ? (
              <div className="py-8 text-center text-sm text-muted">
                No notifications
              </div>
            ) : (
              allNotifications.slice(0, 50).map((n: NotificationResponse) => (
                <div
                  key={n.id}
                  className={`flex items-start gap-3 border-b border-border/50 px-4 py-3 ${
                    !n.read ? "bg-brand/5" : ""
                  }`}
                >
                  <div className="mt-0.5 shrink-0">
                    {notificationIcons[n.notificationType]}
                  </div>
                  <div className="min-w-0 flex-1">
                    {n.notificationType === "pathNotFound" && n.data ? (() => {
                      const d = n.data as Record<string, unknown>;
                      const title = (d.title as string) || "";
                      const configuredRemote = (d.configuredRemotePath as string) || "NOT SET";
                      const configuredLocal = (d.configuredLocalPath as string) || "NOT SET";
                      const remoteHost = (d.clientHost as string) || "NOT SET";
                      const contentDir = (d.contentDir as string) || "unknown";
                      const redactedPath = `${contentDir}/[REDACTED grab ${d.grabId}]`;
                      const question = `My download client ${d.clientName} completed a download but Livrarr says the file is not available locally.\n\nDownload client path: ${redactedPath}\nConfigured Remote Path: ${configuredRemote}\nConfigured Local Path: ${configuredLocal}\nRemote Host: ${remoteHost}\n\nHow do I set up a remote path mapping to fix this?`;
                      return (
                        <>
                          <p className="text-sm text-zinc-200">
                            {String(d.clientName)} reports that {title ? <strong className="font-semibold text-white">{title}</strong> : <strong className="font-semibold text-white">Grab {String(d.grabId)}</strong>} (grab {String(d.grabId)} in the <Link to="/activity/queue" onClick={() => setOpen(false)} className="text-brand hover:underline">queue</Link>) has downloaded, but it does not seem to be available locally. You may need a remote path mapping.
                          </p>
                          <ul className="mt-1.5 space-y-0.5 text-xs list-disc pl-4">
                            <li>
                              <Link to="/settings/mediamanagement" onClick={() => { setOpen(false); setRpmHighlight(true); }} className="text-brand hover:underline">
                                Configure path mapping
                              </Link>
                            </li>
                            <li>
                              <Link to={`/help?question=${encodeURIComponent(question)}`} onClick={() => setOpen(false)} className="text-brand hover:underline">
                                Get AI help
                              </Link>
                            </li>
                          </ul>
                        </>
                      );
                    })() : (
                      <p className="text-sm text-zinc-200">{n.message}</p>
                    )}
                    <p className="mt-0.5 text-xs text-muted">
                      {formatRelativeDate(n.createdAt)}
                    </p>
                  </div>
                  <div className="flex shrink-0 gap-1">
                    {!n.read && (
                      <button
                        onClick={() => markRead.mutate(n.id)}
                        className="rounded p-1 text-muted hover:text-zinc-100"
                        title="Mark as read"
                      >
                        <Check size={14} />
                      </button>
                    )}
                    <button
                      onClick={() => dismiss.mutate(n.id)}
                      className="rounded p-1 text-muted hover:text-red-400"
                      title="Dismiss"
                    >
                      <Trash2 size={14} />
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
