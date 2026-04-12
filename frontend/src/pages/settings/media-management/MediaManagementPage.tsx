import { HelpTip } from "@/components/HelpTip";
import { useState, useEffect } from "react";
import { Joyride, STATUS } from "react-joyride";
import type { EventData, Controls } from "react-joyride";
import { useUIStore } from "@/stores/ui";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm } from "react-hook-form";
import { toast } from "sonner";
import {
  FolderOpen,
  FolderSearch,
  Trash2,
  Plus,
  ArrowRightLeft,
  GripVertical,
  Settings2,
  FileText,
  HardDrive,
  BookOpen,
  Headphones,
  Mail,
  CheckCircle2,
  XCircle,
  Loader2,
} from "lucide-react";
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
  arrayMove,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { useAuthStore } from "@/stores/auth";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import { PathPicker } from "@/components/PathPicker/PathPicker";
import { MediaTypeBadge } from "@/components/Page/Badge";
import { formatBytes } from "@/utils/format";
import type {
  MediaType,
  RootFolderResponse,
  RemotePathMappingResponse,
} from "@/types/api";
import * as api from "@/api";

// ── Root Folder Add Form ──

interface AddRootFolderForm {
  path: string;
  mediaType: MediaType;
}

function AddRootFolderSection({
  onAdd,
}: {
  onAdd: (path: string, mediaType: MediaType) => void;
}) {
  const {
    register,
    handleSubmit,
    reset,
    setValue,
    watch,
    formState: { isSubmitting },
  } = useForm<AddRootFolderForm>({
    defaultValues: { path: "", mediaType: "ebook" },
  });
  const [showPicker, setShowPicker] = useState(false);
  const currentPath = watch("path");

  return (
    <div className="space-y-3">
      <form
        onSubmit={handleSubmit((data) => {
          onAdd(data.path, data.mediaType);
          reset();
        })}
        className="flex items-end gap-3"
      >
        <div className="flex-1">
          <label className="block text-xs text-muted mb-1">Path</label>
          <div className="flex gap-2">
            <input
              {...register("path", { required: true })}
              placeholder="/books/ebooks"
              className="flex-1 rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
            <button
              type="button"
              onClick={() => setShowPicker(!showPicker)}
              className="rounded border border-border bg-zinc-900 px-2 py-2 text-muted hover:text-zinc-100"
              title="Browse filesystem"
            >
              <FolderSearch size={14} />
            </button>
          </div>
        </div>
        <div>
          <label className="block text-xs text-muted mb-1">Type</label>
          <select
            {...register("mediaType")}
            className="rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          >
            <option value="ebook">Ebook</option>
            <option value="audiobook">Audiobook</option>
          </select>
        </div>
        <button
          type="submit"
          disabled={isSubmitting}
          className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
        >
          <Plus size={14} /> Add
        </button>
      </form>
      {showPicker && (
        <PathPicker
          initialPath={currentPath || "/"}
          onSelect={(selected) => {
            setValue("path", selected);
            setShowPicker(false);
          }}
          onClose={() => setShowPicker(false)}
        />
      )}
    </div>
  );
}

// ── Remote Path Mapping Form ──

interface RPMFormData {
  host: string;
  remotePath: string;
  localPath: string;
}

// ── Main Page ──

export default function MediaManagementPage() {
  const isAdmin = useAuthStore((s) => s.isAdmin);
  const qc = useQueryClient();

  // Queries
  const rootFoldersQ = useQuery({
    queryKey: ["rootFolders"],
    queryFn: api.listRootFolders,
  });
  const remoteMappingsQ = useQuery({
    queryKey: ["remotePathMappings"],
    queryFn: api.listRemotePathMappings,
  });
  const mmConfigQ = useQuery({
    queryKey: ["mediaManagementConfig"],
    queryFn: api.getMediaManagementConfig,
  });
  const namingQ = useQuery({
    queryKey: ["namingConfig"],
    queryFn: api.getNamingConfig,
  });

  // Root folder mutations
  const createRootFolder = useMutation({
    mutationFn: ({ path, mediaType }: { path: string; mediaType: MediaType }) =>
      api.createRootFolder(path, mediaType),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["rootFolders"] });
      toast.success("Root folder added");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const deleteRootFolder = useMutation({
    mutationFn: api.deleteRootFolder,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["rootFolders"] });
      toast.success("Root folder removed");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  // Remote path mapping mutations
  const createRPM = useMutation({
    mutationFn: api.createRemotePathMapping,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["remotePathMappings"] });
      toast.success("Mapping created");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const updateRPM = useMutation({
    mutationFn: ({ id, data }: { id: number; data: RPMFormData }) =>
      api.updateRemotePathMapping(id, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["remotePathMappings"] });
      toast.success("Mapping updated");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const deleteRPM = useMutation({
    mutationFn: api.deleteRemotePathMapping,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["remotePathMappings"] });
      toast.success("Mapping deleted");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  // CWA config mutation
  const updateMMConfig = useMutation({
    mutationFn: api.updateMediaManagementConfig,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["mediaManagementConfig"] });
      toast.success("Configuration saved");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  // Modal state
  const [deleteFolder, setDeleteFolder] = useState<RootFolderResponse | null>(
    null,
  );
  const [rpmModal, setRpmModal] = useState<{
    open: boolean;
    editing: RemotePathMappingResponse | null;
  }>({ open: false, editing: null });
  const [deleteRpmId, setDeleteRpmId] = useState<number | null>(null);

  // CWA form
  const [cwaPath, setCwaPath] = useState<string | null>(null);

  // Format preferences
  const [ebookFormats, setEbookFormats] = useState<string[] | null>(null);
  const [audiobookFormats, setAudiobookFormats] = useState<string[] | null>(null);

  // RPM joyride — triggered from PathNotFound notification link
  const rpmHighlight = useUIStore((s) => s.rpmHighlight);
  const setRpmHighlight = useUIStore((s) => s.setRpmHighlight);
  const [rpmJoyrideRun, setRpmJoyrideRun] = useState(false);

  useEffect(() => {
    if (rpmHighlight) {
      const t = setTimeout(() => setRpmJoyrideRun(true), 400);
      return () => clearTimeout(t);
    }
  }, [rpmHighlight]);

  const handleRpmJoyride = (_data: EventData, _controls: Controls) => {
    const { status } = _data;
    if (status === STATUS.FINISHED || status === STATUS.SKIPPED) {
      setRpmJoyrideRun(false);
      setRpmHighlight(false);
    }
  };

  const isLoading =
    rootFoldersQ.isLoading ||
    remoteMappingsQ.isLoading ||
    mmConfigQ.isLoading ||
    namingQ.isLoading;
  const error =
    rootFoldersQ.error ||
    remoteMappingsQ.error ||
    mmConfigQ.error ||
    namingQ.error;

  if (isLoading) return <PageLoading />;
  if (error)
    return (
      <ErrorState
        error={error as Error}
        onRetry={() => {
          rootFoldersQ.refetch();
          remoteMappingsQ.refetch();
          mmConfigQ.refetch();
          namingQ.refetch();
        }}
      />
    );

  const rootFolders = rootFoldersQ.data ?? [];
  const remoteMappings = remoteMappingsQ.data ?? [];
  const mmConfig = mmConfigQ.data;
  const naming = namingQ.data;

  return (
    <>
      {rpmJoyrideRun && (
        <Joyride
          steps={[
            {
              target: "[data-tour='remote-path-section']",
              content:
                "Your download client reported a file that Livrarr couldn't find locally. Add a remote path mapping below to tell Livrarr where to find the file.\n\nNote: Livrarr does not automatically transfer files from the remote server to the local machine. This needs to be set up by the user.",
              placement: "top",
              skipBeacon: true,
            },
          ]}
          run={rpmJoyrideRun}
          onEvent={handleRpmJoyride}
          locale={{ close: "Got it", last: "Got it" }}
          options={{
            primaryColor: "#6366f1",
            hideOverlay: true,
            disableFocusTrap: true,
          }}
          styles={{
            tooltip: {
              borderRadius: 8,
              padding: 16,
              backgroundColor: "#27272a",
              color: "#e4e4e7",
              zIndex: 10000,
              whiteSpace: "pre-line",
            },
            tooltipContent: {
              color: "#a1a1aa",
            },
          }}
        />
      )}
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">
          Media Management
        </h1>
      </PageToolbar>

      <PageContent className="space-y-6">
        {/* ── Root Folders ── */}
        <section data-tour="root-folders-section" className="rounded-lg border border-border bg-zinc-900/50">
          <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
            <FolderOpen size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">
              Root Folders
            </h2>
            <HelpTip text="Where your library files are stored. Add one folder for ebooks and one for audiobooks. Livrarr organizes files into Author/Title subfolders within each root." />
          </div>
          <div className="p-5">

          {rootFolders.length > 0 ? (
            <div className="overflow-x-auto rounded border border-border">
              <table className="w-full text-sm">
                <thead className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
                  <tr>
                    <th className="px-4 py-2">Path</th>
                    <th className="px-4 py-2">Type</th>
                    <th className="px-4 py-2">Free Space</th>
                    <th className="px-4 py-2">Total Space</th>
                    {isAdmin && <th className="px-4 py-2 w-16" />}
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {rootFolders.map((rf) => (
                    <tr key={rf.id} className="text-zinc-200">
                      <td className="px-4 py-2 font-mono text-xs">{rf.path}</td>
                      <td className="px-4 py-2">
                        <MediaTypeBadge type={rf.mediaType} />
                      </td>
                      <td className="px-4 py-2">
                        {rf.freeSpace != null
                          ? formatBytes(rf.freeSpace)
                          : "Unknown"}
                      </td>
                      <td className="px-4 py-2">
                        {rf.totalSpace != null
                          ? formatBytes(rf.totalSpace)
                          : "Unknown"}
                      </td>
                      {isAdmin && (
                        <td className="px-4 py-2">
                          <button
                            onClick={() => setDeleteFolder(rf)}
                            className="text-muted hover:text-red-400"
                          >
                            <Trash2 size={14} />
                          </button>
                        </td>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <EmptyState
              icon={<FolderOpen size={28} />}
              title="No root folders"
              description="Add a root folder to start organizing your library."
            />
          )}

          {isAdmin && (
            <div className="mt-4">
              <AddRootFolderSection
                onAdd={(path, mediaType) =>
                  createRootFolder.mutate({ path, mediaType })
                }
              />
            </div>
          )}
          </div>
        </section>

        {/* ── Remote Path Mappings ── */}
        <section data-tour="remote-path-section" className="rounded-lg border border-border bg-zinc-900/50">
          <div className="flex items-center justify-between border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
            <div className="flex items-center gap-2">
              <ArrowRightLeft size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Remote Path Mappings
              </h2>
              <HelpTip text="Maps paths between your download client (e.g., seedbox) and your local filesystem. If your download client reports files at /home/user/downloads/ but they appear locally at /media/incoming/, add a mapping here." />
            </div>
            {isAdmin && (
              <button
                onClick={() => setRpmModal({ open: true, editing: null })}
                className="inline-flex items-center gap-1.5 rounded bg-brand px-3 py-1.5 text-xs font-medium text-white hover:bg-brand-hover"
              >
                <Plus size={14} /> Add Mapping
              </button>
            )}
          </div>
          <div className="p-5">
          {remoteMappings.length > 0 ? (
            <div className="overflow-x-auto rounded border border-border">
              <table className="w-full text-sm">
                <thead className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
                  <tr>
                    <th className="px-4 py-2">Host</th>
                    <th className="px-4 py-2">Remote Path</th>
                    <th className="px-4 py-2">Local Path</th>
                    {isAdmin && <th className="px-4 py-2 w-24" />}
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {remoteMappings.map((rpm) => (
                    <tr key={rpm.id} className="text-zinc-200">
                      <td className="px-4 py-2">{rpm.host}</td>
                      <td className="px-4 py-2 font-mono text-xs">
                        {rpm.remotePath}
                      </td>
                      <td className="px-4 py-2 font-mono text-xs">
                        {rpm.localPath}
                      </td>
                      {isAdmin && (
                        <td className="px-4 py-2 flex gap-2">
                          <button
                            onClick={() =>
                              setRpmModal({ open: true, editing: rpm })
                            }
                            className="text-muted hover:text-zinc-100 text-xs"
                          >
                            Edit
                          </button>
                          <button
                            onClick={() => setDeleteRpmId(rpm.id)}
                            className="text-muted hover:text-red-400 text-xs"
                          >
                            Delete
                          </button>
                        </td>
                      )}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <EmptyState
              icon={<ArrowRightLeft size={28} />}
              title="No remote path mappings"
              description="Mappings translate paths between your download client and Livrarr."
            />
          )}
          </div>
        </section>

        {/* ── Preferred Formats ── */}
        <section className="rounded-lg border border-border bg-zinc-900/50">
          <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
            <BookOpen size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">
              Preferred Formats
            </h2>
          </div>
          <div className="p-5">
          <p className="text-xs text-muted mb-4">
            Select and order preferred formats. Checked formats are accepted; order determines preference (top = highest).
          </p>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-6 max-w-2xl">
            <FormatPreferenceList
              label="Ebooks"
              icon={<BookOpen size={14} />}
              allFormats={["epub", "mobi", "azw3", "pdf", "cbz", "cbr"]}
              selected={ebookFormats ?? mmConfig?.preferredEbookFormats ?? ["epub"]}
              onChange={(formats) => {
                setEbookFormats(formats);
                updateMMConfig.mutate({
                  cwaIngestPath: cwaPath ?? mmConfig?.cwaIngestPath ?? null,
                  preferredEbookFormats: formats,
                  preferredAudiobookFormats: audiobookFormats ?? mmConfig?.preferredAudiobookFormats ?? ["m4b"],
                });
              }}
            />
            <FormatPreferenceList
              label="Audiobooks"
              icon={<Headphones size={14} />}
              allFormats={["m4b", "m4a", "mp3", "flac", "ogg", "wma"]}
              selected={audiobookFormats ?? mmConfig?.preferredAudiobookFormats ?? ["m4b"]}
              onChange={(formats) => {
                setAudiobookFormats(formats);
                updateMMConfig.mutate({
                  cwaIngestPath: cwaPath ?? mmConfig?.cwaIngestPath ?? null,
                  preferredEbookFormats: ebookFormats ?? mmConfig?.preferredEbookFormats ?? ["epub"],
                  preferredAudiobookFormats: formats,
                });
              }}
            />
          </div>
          </div>
        </section>

        {/* ── CWA Integration (admin only) ── */}
        {isAdmin && (
          <section data-tour="cwa-section" className="rounded-lg border border-border bg-zinc-900/50">
            <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
              <Settings2 size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                CWA Integration
              </h2>
              <HelpTip text="Calibre-Web Automated (CWA) is a self-hosted ebook manager. When configured, Livrarr hardlinks imported ebooks into CWA's ingest folder so they appear in your Calibre library automatically." />
            </div>
            <div className="p-5">
            <div className="max-w-xl">
              <label className="block text-xs text-muted mb-1">
                CWA Ingest Path
              </label>
              <div className="flex gap-3">
                <input
                  type="text"
                  value={cwaPath ?? mmConfig?.cwaIngestPath ?? ""}
                  onChange={(e) => setCwaPath(e.target.value)}
                  placeholder="/cwa/ingest"
                  className="flex-1 rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
                />
                <button
                  onClick={() =>
                    updateMMConfig.mutate({
                      cwaIngestPath: cwaPath ?? mmConfig?.cwaIngestPath ?? null,
                      preferredEbookFormats: ebookFormats ?? mmConfig?.preferredEbookFormats ?? ["epub"],
                      preferredAudiobookFormats: audiobookFormats ?? mmConfig?.preferredAudiobookFormats ?? ["m4b"],
                    })
                  }
                  disabled={updateMMConfig.isPending}
                  className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
                >
                  Save
                </button>
              </div>
            </div>
            </div>
          </section>
        )}

        {/* ── Email / Send to Kindle (admin only) ── */}
        {isAdmin && <EmailConfigSection />}

        {/* ── Naming (read-only) ── */}
        <section className="rounded-lg border border-border bg-zinc-900/50 opacity-60" title="Coming Soon">
          <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
            <FileText size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">Naming</h2>
          </div>
          <div className="p-5">
          {naming && (
            <div className="space-y-3 max-w-xl">
              <div>
                <label className="block text-xs text-muted mb-1">
                  Author Folder Format
                </label>
                <input
                  type="text"
                  value={naming.authorFolderFormat}
                  disabled
                  className="w-full rounded border border-border bg-zinc-900/50 px-3 py-2 text-sm text-zinc-400 cursor-not-allowed"
                />
              </div>
              <div>
                <label className="block text-xs text-muted mb-1">
                  Book Folder Format
                </label>
                <input
                  type="text"
                  value={naming.bookFolderFormat}
                  disabled
                  className="w-full rounded border border-border bg-zinc-900/50 px-3 py-2 text-sm text-zinc-400 cursor-not-allowed"
                />
              </div>
              <div className="flex gap-6 text-sm text-zinc-400">
                <span>Rename Files: {naming.renameFiles ? "Yes" : "No"}</span>
                <span>
                  Replace Illegal Chars:{" "}
                  {naming.replaceIllegalChars ? "Yes" : "No"}
                </span>
              </div>
            </div>
          )}
          </div>
        </section>

        {/* ── File Management (coming soon) — intentional placeholder for post-alpha features ── */}
        <section className="rounded-lg border border-border bg-zinc-900/50 opacity-60" title="Coming Soon">
          <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
            <HardDrive size={18} className="text-muted" />
            <h2 className="text-base font-semibold text-zinc-100">
              File Management
            </h2>
          </div>
          <div className="p-5">
          <div className="space-y-3 max-w-xl">
            <label className="flex items-center gap-3 text-sm text-zinc-400 cursor-not-allowed">
              <input type="checkbox" disabled className="rounded" />
              Create empty author folders
            </label>
            <label className="flex items-center gap-3 text-sm text-zinc-400 cursor-not-allowed">
              <input type="checkbox" disabled className="rounded" />
              Delete empty folders
            </label>
            <label className="flex items-center gap-3 text-sm text-zinc-400 cursor-not-allowed">
              <input type="checkbox" disabled className="rounded" />
              Import extra files
            </label>
          </div>
          </div>
        </section>
      </PageContent>

      {/* ── Modals ── */}

      {/* Delete root folder */}
      <ConfirmModal
        open={deleteFolder !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteFolder(null);
        }}
        title="Delete Root Folder"
        description={`Remove "${deleteFolder?.path}"? Existing files will not be deleted.`}
        confirmLabel="Delete"
        onConfirm={() => {
          if (deleteFolder)
            return deleteRootFolder.mutateAsync(deleteFolder.id);
        }}
      />

      {/* Delete remote path mapping */}
      <ConfirmModal
        open={deleteRpmId !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteRpmId(null);
        }}
        title="Delete Mapping"
        description="Remove this remote path mapping?"
        confirmLabel="Delete"
        onConfirm={() => {
          if (deleteRpmId) return deleteRPM.mutateAsync(deleteRpmId);
        }}
      />

      {/* Add/Edit remote path mapping */}
      <RPMFormModal
        open={rpmModal.open}
        editing={rpmModal.editing}
        onClose={() => setRpmModal({ open: false, editing: null })}
        onSubmit={(data) => {
          if (rpmModal.editing) {
            return updateRPM.mutateAsync({ id: rpmModal.editing.id, data });
          }
          return createRPM.mutateAsync(data);
        }}
      />
    </>
  );
}

// ── RPM Form Modal ──

function RPMFormModal({
  open,
  editing,
  onClose,
  onSubmit,
}: {
  open: boolean;
  editing: RemotePathMappingResponse | null;
  onClose: () => void;
  onSubmit: (data: RPMFormData) => Promise<unknown>;
}) {
  const { data: clients } = useQuery({
    queryKey: ["downloadClients"],
    queryFn: api.listDownloadClients,
  });

  // Unique hosts from download clients
  const hosts = [...new Set((clients ?? []).map((c) => c.host))];

  const {
    register,
    handleSubmit,
    formState: { isSubmitting },
  } = useForm<RPMFormData>({
    values: editing
      ? {
          host: editing.host,
          remotePath: editing.remotePath,
          localPath: editing.localPath,
        }
      : { host: hosts[0] ?? "", remotePath: "", localPath: "" },
  });

  return (
    <FormModal
      open={open}
      onOpenChange={(o) => {
        if (!o) onClose();
      }}
      title={editing ? "Edit Mapping" : "Add Mapping"}
    >
      <form
        onSubmit={handleSubmit(async (data) => {
          await onSubmit(data);
          onClose();
        })}
        className="space-y-4"
      >
        <div>
          <label className="block text-xs text-muted mb-1">Host</label>
          <select
            {...register("host", { required: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          >
            {hosts.map((h) => (
              <option key={h} value={h}>{h}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-muted mb-1">Remote Path</label>
          <input
            {...register("remotePath", { required: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
        <div>
          <label className="block text-xs text-muted mb-1">Local Path</label>
          <input
            {...register("localPath", { required: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
        <div className="flex justify-end gap-3 pt-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={isSubmitting}
            className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {isSubmitting ? "Saving..." : "Save"}
          </button>
        </div>
      </form>
    </FormModal>
  );
}

// ── Format Preference List ──

function FormatPreferenceList({
  label,
  icon,
  allFormats,
  selected,
  onChange,
}: {
  label: string;
  icon: React.ReactNode;
  allFormats: string[];
  selected: string[];
  onChange: (formats: string[]) => void;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  const toggle = (fmt: string) => {
    if (selected.includes(fmt)) {
      onChange(selected.filter((f) => f !== fmt));
    } else {
      onChange([...selected, fmt]);
    }
  };

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (over && active.id !== over.id) {
      const oldIndex = selected.indexOf(active.id as string);
      const newIndex = selected.indexOf(over.id as string);
      onChange(arrayMove(selected, oldIndex, newIndex));
    }
  };

  return (
    <div>
      <h3 className="mb-2 flex items-center gap-1.5 text-sm font-medium text-zinc-200">
        {icon} {label}
      </h3>

      {/* Selected formats (draggable) */}
      {selected.length > 0 && (
        <DndContext
          sensors={sensors}
          collisionDetection={closestCenter}
          onDragEnd={handleDragEnd}
        >
          <SortableContext
            items={selected}
            strategy={verticalListSortingStrategy}
          >
            <div className="mb-2 space-y-1">
              {selected.map((fmt, idx) => (
                <SortableFormatItem
                  key={fmt}
                  id={fmt}
                  rank={idx + 1}
                  onToggle={() => toggle(fmt)}
                />
              ))}
            </div>
          </SortableContext>
        </DndContext>
      )}

      {/* Unchecked formats */}
      {allFormats
        .filter((f) => !selected.includes(f))
        .map((fmt) => (
          <div key={fmt} className="flex items-center gap-2 px-2 py-1.5">
            <span className="w-5" />
            <input
              type="checkbox"
              checked={false}
              onChange={() => toggle(fmt)}
              aria-label={`Enable ${fmt}`}
            />
            <span className="text-sm text-zinc-500 font-mono">.{fmt}</span>
          </div>
        ))}
    </div>
  );
}

function SortableFormatItem({
  id,
  rank,
  onToggle,
}: {
  id: string;
  rank: number;
  onToggle: () => void;
}) {
  const {
    attributes,
    listeners,
    setNodeRef,
    transform,
    transition,
    isDragging,
  } = useSortable({ id });

  const style = {
    transform: CSS.Transform.toString(transform),
    transition,
    opacity: isDragging ? 0.5 : 1,
  };

  return (
    <div
      ref={setNodeRef}
      style={style}
      className="flex items-center gap-2 rounded bg-zinc-800/50 px-2 py-1.5"
    >
      <button
        {...attributes}
        {...listeners}
        className="cursor-grab text-zinc-500 hover:text-zinc-300 active:cursor-grabbing"
        aria-label={`Drag to reorder ${id}`}
      >
        <GripVertical size={14} />
      </button>
      <input
        type="checkbox"
        checked
        onChange={onToggle}
        aria-label={`${id} enabled`}
      />
      <span className="flex-1 text-sm text-zinc-200 font-mono">.{id}</span>
      <span className="text-xs text-muted">#{rank}</span>
    </div>
  );
}

// ── Email / Send to Kindle Config (MOCK) ──

const SMTP_PRESETS: Record<string, { host: string; port: number; encryption: string }> = {
  gmail: { host: "smtp.gmail.com", port: 587, encryption: "starttls" },
  outlook: { host: "smtp-mail.outlook.com", port: 587, encryption: "starttls" },
  custom: { host: "", port: 587, encryption: "starttls" },
};

function detectPreset(host: string): string {
  if (host === "smtp.gmail.com") return "gmail";
  if (host === "smtp-mail.outlook.com") return "outlook";
  return "custom";
}

function EmailConfigSection() {
  const qc = useQueryClient();
  const emailConfigQ = useQuery({
    queryKey: ["emailConfig"],
    queryFn: api.getEmailConfig,
  });

  const [preset, setPreset] = useState<string | null>(null);
  const [host, setHost] = useState<string | null>(null);
  const [port, setPort] = useState<number | null>(null);
  const [encryption, setEncryption] = useState<string | null>(null);
  const [username, setUsername] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [fromAddress, setFromAddress] = useState<string | null>(null);
  const [recipientEmail, setRecipientEmail] = useState<string | null>(null);
  const [sendOnImport, setSendOnImport] = useState<boolean | null>(null);
  const [testState, setTestState] = useState<"idle" | "loading" | "success" | "fail">("idle");

  // Derive current values from local overrides or server data
  const cfg = emailConfigQ.data;
  const curHost = host ?? cfg?.smtpHost ?? "smtp.gmail.com";
  const curPort = port ?? cfg?.smtpPort ?? 587;
  const curEncryption = encryption ?? cfg?.encryption ?? "starttls";
  const curUsername = username ?? cfg?.username ?? "";
  const curFromAddress = fromAddress ?? cfg?.fromAddress ?? "";
  const curRecipientEmail = recipientEmail ?? cfg?.recipientEmail ?? "";
  const curSendOnImport = sendOnImport ?? cfg?.sendOnImport ?? false;
  const curPreset = preset ?? detectPreset(curHost);

  const updateEmail = useMutation({
    mutationFn: api.updateEmailConfig,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["emailConfig"] });
      // Reset local overrides so we read from server
      setHost(null);
      setPort(null);
      setEncryption(null);
      setUsername(null);
      setPassword("");
      setFromAddress(null);
      setRecipientEmail(null);
      setSendOnImport(null);
      setPreset(null);
      toast.success("Email configuration saved");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const applyPreset = (key: string) => {
    setPreset(key);
    const p = SMTP_PRESETS[key];
    if (p) {
      setHost(p.host || curHost);
      setPort(p.port);
      setEncryption(p.encryption);
    }
    if (key === "gmail" && !curUsername) {
      setUsername("you@gmail.com");
      if (!curFromAddress) setFromAddress("you@gmail.com");
    } else if (key === "outlook" && !curUsername) {
      setUsername("you@outlook.com");
      if (!curFromAddress) setFromAddress("you@outlook.com");
    }
  };

  const handleSave = () => {
    updateEmail.mutate({
      smtpHost: curHost,
      smtpPort: curPort,
      encryption: curEncryption,
      username: curUsername || null,
      ...(password ? { password } : {}),
      fromAddress: curFromAddress || null,
      recipientEmail: curRecipientEmail || null,
      sendOnImport: curSendOnImport,
      enabled: true,
    });
  };

  const handleTest = async () => {
    setTestState("loading");
    try {
      await api.testEmailConfig();
      setTestState("success");
      toast.success("Test email sent");
    } catch (e) {
      setTestState("fail");
      toast.error(e instanceof Error ? e.message : "Test email failed");
    }
  };

  if (emailConfigQ.isLoading) return null;

  return (
    <section data-tour="email-kindle-section" className="rounded-lg border border-border bg-zinc-900/50">
      <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
        <Mail size={18} className="text-muted" />
        <h2 className="text-base font-semibold text-zinc-100">
          Email / Send to Kindle
        </h2>
        <HelpTip text="Send ebooks to your Kindle or eReader via email. Configure your SMTP server and recipient email address. You must add the 'From' address to your Amazon Approved Personal Document Email List." />
      </div>
      <div className="p-5">
        <div className="max-w-xl space-y-4">
          {/* Provider preset */}
          <div>
            <label className="block text-xs text-muted mb-1">Provider</label>
            <select
              value={curPreset}
              onChange={(e) => applyPreset(e.target.value)}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            >
              <option value="custom">Custom SMTP</option>
              <option value="gmail">Gmail</option>
              <option value="outlook">Outlook</option>
            </select>
          </div>

          {/* SMTP Host + Port */}
          <div className="grid grid-cols-3 gap-3">
            <div className="col-span-2">
              <label className="block text-xs text-muted mb-1">SMTP Host</label>
              <input
                type="text"
                value={curHost}
                onChange={(e) => setHost(e.target.value)}
                placeholder="smtp.example.com"
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
            <div>
              <label className="block text-xs text-muted mb-1">Port</label>
              <input
                type="number"
                value={curPort}
                onChange={(e) => setPort(Number(e.target.value))}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
          </div>

          {/* Encryption */}
          <div>
            <label className="block text-xs text-muted mb-1">Encryption</label>
            <select
              value={curEncryption}
              onChange={(e) => setEncryption(e.target.value)}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            >
              <option value="none">None</option>
              <option value="starttls">STARTTLS</option>
              <option value="ssl">SSL/TLS</option>
            </select>
          </div>

          {/* Username + Password */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs text-muted mb-1">Username</label>
              <input
                type="text"
                value={curUsername}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="user@example.com"
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
            <div>
              <label className="flex items-center gap-1 text-xs text-muted mb-1">
                Password
                {curPreset === "gmail" ? (
                  <HelpTip text="Gmail requires an App Password, not your account password. Go to myaccount.google.com → Security → 2-Step Verification → App Passwords. Generate a new app password for 'Mail' and paste it here." />
                ) : curPreset === "outlook" ? (
                  <HelpTip text="If you have 2FA enabled, go to account.microsoft.com → Security → Advanced security options → App passwords. Generate a new app password and paste it here." />
                ) : null}
              </label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={cfg?.passwordSet ? "••••••••" : ""}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
          </div>

          {/* From Address */}
          <div>
            <label className="block text-xs text-muted mb-1">From Address</label>
            <input
              type="email"
              value={curFromAddress}
              onChange={(e) => setFromAddress(e.target.value)}
              placeholder="livrarr@example.com"
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
            <p className="mt-1 text-xs text-muted inline-flex items-center gap-1">
              Must be on your Kindle Approved Email List
              <HelpTip text="To add this address: 1) Go to amazon.com/myk → Preferences → Personal Document Settings. 2) Scroll to 'Approved Personal Document E-mail List'. 3) Click 'Add a new approved e-mail address'. 4) Enter the From Address above and click 'Add Address'. Without this, Amazon will silently reject emails from Livrarr." />
            </p>
          </div>

          {/* Recipient Email */}
          <div>
            <label className="block text-xs text-muted mb-1">Kindle Email</label>
            <input
              type="email"
              value={curRecipientEmail}
              onChange={(e) => setRecipientEmail(e.target.value)}
              placeholder="yourname@kindle.com"
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>

          {/* Auto-send on import */}
          <label className="flex items-center gap-3 text-sm text-zinc-200 cursor-pointer group">
            <input
              type="checkbox"
              checked={curSendOnImport}
              onChange={(e) => setSendOnImport(e.target.checked)}
              className="rounded border-border"
            />
            <span>Send to Kindle automatically on import</span>
            <HelpTip text="Automatically emails EPUB and PDF files to your Kindle when imported. Other formats (MOBI, AZW3, M4B, etc.) are skipped. If the send fails, you'll see a notification — the import itself is not affected." />
          </label>

          {/* Actions */}
          <div className="flex items-center gap-3 pt-2">
            <button
              onClick={handleSave}
              disabled={updateEmail.isPending}
              className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
            >
              {updateEmail.isPending ? "Saving..." : "Save"}
            </button>
            <button
              onClick={handleTest}
              disabled={testState === "loading" || !curHost || !curRecipientEmail}
              className="inline-flex items-center gap-1.5 rounded border border-border px-3 py-2 text-sm text-zinc-300 hover:text-zinc-100 hover:border-zinc-500 disabled:opacity-50"
            >
              {testState === "loading" ? (
                <Loader2 size={14} className="animate-spin" />
              ) : testState === "success" ? (
                <CheckCircle2 size={14} className="text-green-400" />
              ) : testState === "fail" ? (
                <XCircle size={14} className="text-red-400" />
              ) : (
                <Mail size={14} />
              )}
              Send Test Email
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
