import { useState } from "react";
import {
  Bell,
  BookPlus,
  BookCheck,
  RefreshCw,
  Layers,
  AlertOctagon,
  AlertTriangle,
  Trash2,
  Check,
} from "lucide-react";
import * as Popover from "@radix-ui/react-popover";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import * as api from "@/api";
import type { NotificationType, NotificationResponse } from "@/types/api";
import { formatRelativeDate } from "@/utils/format";
import { LoadingSpinner } from "@/components/Page/LoadingSpinner";
import type { ReactNode } from "react";

const notificationIcons: Record<NotificationType, ReactNode> = {
  newWorkDetected: <BookPlus size={16} className="text-blue-400" />,
  workAutoAdded: <BookCheck size={16} className="text-green-400" />,
  metadataUpdated: <RefreshCw size={16} className="text-yellow-400" />,
  bulkEnrichmentComplete: <Layers size={16} className="text-purple-400" />,
  jobPanicked: <AlertOctagon size={16} className="text-red-400" />,
  rateLimitHit: <AlertTriangle size={16} className="text-orange-400" />,
};

export function NotificationBell() {
  const [open, setOpen] = useState(false);
  const queryClient = useQueryClient();

  const { data: unreadNotifications } = useQuery({
    queryKey: ["notifications", "unread"],
    queryFn: () => api.listNotifications(true),
    select: (res) => res.items,
    refetchInterval: 30_000,
    refetchOnWindowFocus: true,
    staleTime: 0,
  });

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
                    <p className="text-sm text-zinc-200">{n.message}</p>
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
