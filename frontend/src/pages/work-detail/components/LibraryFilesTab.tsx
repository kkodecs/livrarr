import { useState } from "react";
import { useNavigate } from "react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { Book, Headphones, BookOpen, Play, Mail, Loader2, Trash2 } from "lucide-react";
import { deleteLibraryFile, sendFileEmail } from "@/api";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { formatBytes, formatRelativeDate } from "@/utils/format";
import ProgressBadge from "@/components/ProgressBadge";
import type { WorkDetailResponse } from "@/types/api";

// Formats Amazon accepts via Send to Kindle email
const KINDLE_ACCEPTED_FORMATS = new Set([
  "epub", "pdf", "docx", "doc", "rtf", "htm", "html", "txt",
]);

const MAX_EMAIL_SIZE = 50 * 1024 * 1024; // 50 MB

function getFileExtension(path: string): string {
  const dot = path.lastIndexOf(".");
  return dot >= 0 ? path.slice(dot + 1).toLowerCase() : "";
}

const READABLE_FORMATS = new Set(["epub", "pdf"]);
const LISTENABLE_FORMATS = new Set(["m4b", "m4a", "mp3", "flac", "ogg"]);

export function LibraryFilesTab({ work }: { work: WorkDetailResponse }) {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [sendingId, setSendingId] = useState<number | null>(null);
  const [sentIds, setSentIds] = useState<Set<number>>(new Set());

  const deleteFileMutation = useMutation({
    mutationFn: (fileId: number) => deleteLibraryFile(fileId),
    onSuccess: () => {
      toast.success("File deleted");
      queryClient.invalidateQueries({ queryKey: ["work"] });
      setConfirmDelete(null);
    },
    onError: () => toast.error("Failed to delete file"),
  });

  const sendEmailMutation = useMutation({
    mutationFn: sendFileEmail,
    onSuccess: (_data, itemId) => {
      setSendingId(null);
      setSentIds((prev) => new Set(prev).add(itemId));
      toast.success("Sent to Kindle");
    },
    onError: (e: Error) => {
      setSendingId(null);
      toast.error(e.message || "Failed to send email");
    },
  });

  const handleSendEmail = (itemId: number) => {
    setSendingId(itemId);
    sendEmailMutation.mutate(itemId);
  };

  if (work.libraryItems.length === 0) {
    return (
      <EmptyState
        icon={<Book size={24} />}
        title="No library files"
        description="Files will appear here after import."
      />
    );
  }

  return (
    <>
      <div className="overflow-x-auto">
        <table className="w-full text-sm">
          <thead className="border-b border-border">
            <tr>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Path
              </th>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Type
              </th>
              <th className="px-3 py-2 text-right text-xs font-medium uppercase text-muted">
                Size
              </th>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Imported
              </th>
              <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                Progress
              </th>
              <th className="w-20 px-3 py-2" />
            </tr>
          </thead>
          <tbody className="divide-y divide-border">
            {work.libraryItems.map((item) => {
              const ext = getFileExtension(item.path);
              const canSend = KINDLE_ACCEPTED_FORMATS.has(ext);
              const tooLarge = item.fileSize > MAX_EMAIL_SIZE;
              const isSending = sendingId === item.id;
              const wasSent = sentIds.has(item.id);

              return (
                <tr key={item.id} className="hover:bg-zinc-800/50">
                  <td
                    className="max-w-md truncate px-3 py-2 font-mono text-xs text-zinc-300"
                    title={item.path}
                  >
                    {item.path}
                  </td>
                  <td className="px-3 py-2">
                    <div className="inline-flex items-center gap-1 text-zinc-400">
                      {item.mediaType === "ebook" ? (
                        <Book size={14} />
                      ) : (
                        <Headphones size={14} />
                      )}
                      <span className="text-xs capitalize">{item.mediaType}</span>
                    </div>
                  </td>
                  <td className="px-3 py-2 text-right text-muted">
                    {formatBytes(item.fileSize)}
                  </td>
                  <td className="px-3 py-2 text-muted">
                    {formatRelativeDate(item.importedAt)}
                  </td>
                  <td className="px-3 py-2">
                    {item.progressPct != null && item.progressPct > 0 ? (
                      <div className="flex items-center gap-2">
                        <div className="w-16 h-1.5 bg-zinc-700 rounded-full overflow-hidden">
                          <div
                            className="h-full bg-brand rounded-full"
                            style={{ width: `${Math.min(item.progressPct * 100, 100)}%` }}
                          />
                        </div>
                        <ProgressBadge
                          progressPct={item.progressPct}
                          mediaType={item.mediaType}
                          durationSeconds={item.durationSeconds}
                          finishedAt={item.finishedAt}
                        />
                      </div>
                    ) : null}
                  </td>
                  <td className="px-3 py-2 flex items-center justify-end gap-1">
                    {READABLE_FORMATS.has(ext) && (
                      <button
                        onClick={() => navigate(`/read/${item.id}`)}
                        className="rounded p-1 text-muted hover:text-brand"
                        title="Read"
                      >
                        <BookOpen size={14} />
                      </button>
                    )}
                    {LISTENABLE_FORMATS.has(ext) && (
                      <button
                        onClick={() => navigate(`/listen/${item.id}?workId=${work.id}`)}
                        className="rounded p-1 text-muted hover:text-brand"
                        title="Listen"
                      >
                        <Play size={14} />
                      </button>
                    )}
                    {canSend && (
                      <button
                        onClick={() => handleSendEmail(item.id)}
                        disabled={isSending || tooLarge}
                        className={`rounded p-1 hover:text-brand disabled:opacity-40 ${tooLarge ? "disabled:cursor-not-allowed text-muted" : isSending ? "cursor-wait text-brand" : wasSent ? "text-green-400" : "text-muted"}`}
                        title={
                          tooLarge
                            ? `File too large (${formatBytes(item.fileSize)}). Amazon limit is 50 MB.`
                            : wasSent
                              ? "Sent to Kindle"
                              : "Send to Kindle"
                        }
                      >
                        {isSending ? (
                          <Loader2 size={14} className="animate-spin text-brand" />
                        ) : (
                          <Mail size={14} className={wasSent ? "text-green-400" : ""} />
                        )}
                      </button>
                    )}
                    <button
                      onClick={() => setConfirmDelete(item.id)}
                      className="rounded p-1 text-muted hover:text-red-400"
                      title="Delete file"
                    >
                      <Trash2 size={14} />
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>

      <ConfirmModal
        open={confirmDelete !== null}
        onOpenChange={(open) => {
          if (!open) setConfirmDelete(null);
        }}
        title="Delete File"
        description="Are you sure you want to delete this library file?"
        confirmLabel="Delete"
        variant="danger"
        onConfirm={() => {
          if (confirmDelete !== null)
            return deleteFileMutation.mutateAsync(confirmDelete);
        }}
      />
    </>
  );
}
