import { useState, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm, Controller } from "react-hook-form";
import { toast } from "sonner";
import {
  Plus,
  Trash2,
  Pencil,
  CheckCircle2,
  XCircle,
  Star,
  Download,
  ChevronDown,
  ChevronRight,
  AlertTriangle,
} from "lucide-react";
import { useAuthStore } from "@/stores/auth";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import type {
  DownloadClientResponse,
  CreateDownloadClientRequest,
  UpdateDownloadClientRequest,
  DownloadClientImplementation,
  ProwlarrImportResponse,
} from "@/types/api";
import { useSort } from "@/hooks/useSort";
import { SortHeader } from "@/components/Page/SortHeader";
import * as api from "@/api";

type ClientSortField = "name" | "host" | "enabled";

// ── Form Types ──

interface ClientFormData {
  implementation: DownloadClientImplementation;
  name: string;
  host: string;
  port: number;
  useSsl: boolean;
  skipSslValidation: boolean;
  urlBase: string;
  username: string;
  password: string;
  apiKey: string;
  category: string;
  enabled: boolean;
  isDefaultForProtocol: boolean;
}

function toRequest(data: ClientFormData): CreateDownloadClientRequest {
  return {
    name: data.name,
    implementation: data.implementation,
    host: data.host,
    port: data.port,
    useSsl: data.useSsl,
    skipSslValidation: data.skipSslValidation,
    urlBase: data.urlBase || null,
    username: data.implementation === "qBittorrent" ? data.username || null : null,
    password: data.implementation === "qBittorrent" ? data.password || null : null,
    category: data.category,
    enabled: data.enabled,
    apiKey: data.implementation === "sabnzbd" ? data.apiKey || null : null,
    isDefaultForProtocol: data.isDefaultForProtocol,
  };
}

/** Build an update payload that omits empty credential fields so the backend
 *  preserves existing saved credentials (undefined is stripped by JSON.stringify,
 *  whereas null means "clear this field"). */
function toUpdateRequest(data: ClientFormData): UpdateDownloadClientRequest {
  const req: UpdateDownloadClientRequest = {
    name: data.name,
    host: data.host,
    port: data.port,
    useSsl: data.useSsl,
    skipSslValidation: data.skipSslValidation,
    urlBase: data.urlBase || null,
    username: data.implementation === "qBittorrent" ? data.username || null : null,
    category: data.category,
    enabled: data.enabled,
    isDefaultForProtocol: data.isDefaultForProtocol,
  };

  // Only include password when the user actually typed a new one.
  if (data.implementation === "qBittorrent" && data.password) {
    req.password = data.password;
  }
  // Only include apiKey when the user actually typed a new one.
  if (data.implementation === "sabnzbd" && data.apiKey) {
    req.apiKey = data.apiKey;
  }

  return req;
}

const defaultValues: ClientFormData = {
  implementation: "qBittorrent",
  name: "",
  host: "localhost",
  port: 8080,
  useSsl: false,
  skipSslValidation: false,
  urlBase: "",
  username: "",
  password: "",
  apiKey: "",
  category: "livrarr",
  enabled: true,
  isDefaultForProtocol: false,
};

// ── Client Table ──

function ClientTable({
  clients,
  sorting,
  isAdmin,
  onEdit,
  onDelete,
  onSetDefault,
}: {
  clients: DownloadClientResponse[];
  sorting: ReturnType<typeof useSort<ClientSortField>>;
  isAdmin: boolean;
  onEdit: (c: DownloadClientResponse) => void;
  onDelete: (c: DownloadClientResponse) => void;
  onSetDefault: (c: DownloadClientResponse) => void;
}) {
  if (clients.length === 0) return null;

  return (
    <div className="overflow-x-auto rounded border border-border">
      <table className="w-full text-sm">
        <thead className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
          <tr>
            <SortHeader field="name" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Name</SortHeader>
            <th className="px-4 py-2">Implementation</th>
            <SortHeader field="host" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Host</SortHeader>
            <SortHeader field="enabled" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Status</SortHeader>
            <th className="px-4 py-2">Default</th>
            {isAdmin && <th className="px-4 py-2 w-24" />}
          </tr>
        </thead>
        <tbody className="divide-y divide-border">
          {clients.map((c) => (
            <tr key={c.id} className="text-zinc-200">
              <td className="px-4 py-2 font-medium">{c.name}</td>
              <td className="px-4 py-2">{c.implementation}</td>
              <td className="px-4 py-2 font-mono text-xs">
                {c.useSsl ? "https" : "http"}://{c.host}:{c.port}
              </td>
              <td className="px-4 py-2">
                <span
                  className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${c.enabled ? "bg-green-500/20 text-green-400" : "bg-zinc-600/20 text-zinc-400"}`}
                >
                  {c.enabled ? "Enabled" : "Disabled"}
                </span>
              </td>
              <td className="px-4 py-2">
                {c.isDefaultForProtocol ? (
                  <Star size={14} className="text-yellow-400 fill-yellow-400" />
                ) : isAdmin ? (
                  <button
                    onClick={() => onSetDefault(c)}
                    className="text-muted hover:text-yellow-400"
                    title="Set as default"
                  >
                    <Star size={14} />
                  </button>
                ) : null}
              </td>
              {isAdmin && (
                <td className="px-4 py-2 flex gap-2">
                  <button
                    onClick={() => onEdit(c)}
                    className="text-muted hover:text-zinc-100"
                  >
                    <Pencil size={14} />
                  </button>
                  <button
                    onClick={() => onDelete(c)}
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
  );
}

// ── Main Page ──

export default function DownloadClientsPage() {
  const isAdmin = useAuthStore((s) => s.isAdmin);
  const qc = useQueryClient();

  const clientsQ = useQuery({
    queryKey: ["downloadClients"],
    queryFn: api.listDownloadClients,
  });

  const createClient = useMutation({
    mutationFn: api.createDownloadClient,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["downloadClients"] });
      toast.success("Download client added");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const updateClient = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: number;
      data: UpdateDownloadClientRequest;
    }) => api.updateDownloadClient(id, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["downloadClients"] });
      toast.success("Download client updated");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const deleteClient = useMutation({
    mutationFn: api.deleteDownloadClient,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["downloadClients"] });
      toast.success("Download client removed");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const setDefault = useMutation({
    mutationFn: (c: DownloadClientResponse) =>
      api.updateDownloadClient(c.id, { isDefaultForProtocol: true }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["downloadClients"] });
      toast.success("Default client updated");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const [modal, setModal] = useState<{
    open: boolean;
    editing: DownloadClientResponse | null;
  }>({ open: false, editing: null });
  const [deleteTarget, setDeleteTarget] =
    useState<DownloadClientResponse | null>(null);

  const sorting = useSort<ClientSortField>("name");

  if (clientsQ.isLoading) return <PageLoading />;
  if (clientsQ.error)
    return (
      <ErrorState
        error={clientsQ.error as Error}
        onRetry={() => clientsQ.refetch()}
      />
    );

  const allClients = clientsQ.data ?? [];
  const torrentClients = sorting.sort(
    allClients.filter((c) => c.implementation !== "sabnzbd"),
    (item, field) => {
      switch (field) {
        case "name": return item.name;
        case "host": return item.host;
        case "enabled": return item.enabled ? 0 : 1;
      }
    },
  );
  const usenetClients = sorting.sort(
    allClients.filter((c) => c.implementation === "sabnzbd"),
    (item, field) => {
      switch (field) {
        case "name": return item.name;
        case "host": return item.host;
        case "enabled": return item.enabled ? 0 : 1;
      }
    },
  );

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">
          Download Clients
        </h1>
      </PageToolbar>

      <PageContent className="space-y-4">
        {/* ── Torrent Clients ── */}
        <section>
          <h2 className="text-base font-semibold text-zinc-100 mb-4">
            Torrent Clients
          </h2>
          {torrentClients.length > 0 ? (
            <ClientTable
              clients={torrentClients}
              sorting={sorting}
              isAdmin={isAdmin}
              onEdit={(c) => setModal({ open: true, editing: c })}
              onDelete={(c) => setDeleteTarget(c)}
              onSetDefault={(c) => setDefault.mutate(c)}
            />
          ) : (
            <p className="text-sm text-muted">No torrent clients configured.</p>
          )}
        </section>

        {/* ── Usenet Clients ── */}
        <section>
          <h2 className="text-base font-semibold text-zinc-100 mb-4">
            Usenet Clients
          </h2>
          {usenetClients.length > 0 ? (
            <ClientTable
              clients={usenetClients}
              sorting={sorting}
              isAdmin={isAdmin}
              onEdit={(c) => setModal({ open: true, editing: c })}
              onDelete={(c) => setDeleteTarget(c)}
              onSetDefault={(c) => setDefault.mutate(c)}
            />
          ) : (
            <p className="text-sm text-muted">No Usenet clients configured.</p>
          )}
        </section>

        {/* ── Import from Prowlarr ── */}
        {isAdmin && (
          <ProwlarrImportSection
            onImported={() => qc.invalidateQueries({ queryKey: ["downloadClients"] })}
          />
        )}

        {/* ── Inline Add Form ── */}
        {isAdmin && (
          <section data-tour="add-client-form" className="rounded-lg border border-border bg-zinc-900/50">
            <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
              <Plus size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">Add Download Client</h2>
            </div>
            <div className="p-5">
              <InlineClientForm
                onSubmit={async (data) => {
                  await createClient.mutateAsync(toRequest(data));
                }}
              />
            </div>
          </section>
        )}
      </PageContent>

      {/* ── Delete Confirm ── */}
      <ConfirmModal
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) setDeleteTarget(null);
        }}
        title="Delete Download Client"
        description={`Remove "${deleteTarget?.name}"?`}
        confirmLabel="Delete"
        onConfirm={() => {
          if (deleteTarget) return deleteClient.mutateAsync(deleteTarget.id);
        }}
      />

      {/* ── Add/Edit Modal ── */}
      <ClientFormModal
        open={modal.open}
        editing={modal.editing}
        onClose={() => setModal({ open: false, editing: null })}
        onSubmit={async (data) => {
          if (modal.editing) {
            await updateClient.mutateAsync({ id: modal.editing.id, data: toUpdateRequest(data) });
          } else {
            await createClient.mutateAsync(toRequest(data));
          }
        }}
      />
    </>
  );
}

// ── Client Form Modal ──

function ClientFormModal({
  open,
  editing,
  onClose,
  onSubmit,
}: {
  open: boolean;
  editing: DownloadClientResponse | null;
  onClose: () => void;
  onSubmit: (data: ClientFormData) => Promise<void>;
}) {
  const [testResult, setTestResult] = useState<"success" | "fail" | null>(null);

  const testClient = useMutation({
    mutationFn: api.testDownloadClient,
    onSuccess: () => {
      setTestResult("success");
      toast.success("Connection successful");
    },
    onError: (e: Error) => {
      setTestResult("fail");
      toast.error(e.message);
    },
  });

  const testSaved = useMutation({
    mutationFn: api.testSavedDownloadClient,
    onSuccess: () => {
      setTestResult("success");
      toast.success("Connection successful");
    },
    onError: (e: Error) => {
      setTestResult("fail");
      toast.error(e.message);
    },
  });

  const {
    register,
    handleSubmit,
    watch,
    getValues,
    control,
    formState: { isSubmitting },
  } = useForm<ClientFormData>({
    values: editing
      ? {
          implementation: editing.implementation,
          name: editing.name,
          host: editing.host,
          port: editing.port,
          useSsl: editing.useSsl,
          skipSslValidation: editing.skipSslValidation,
          urlBase: editing.urlBase ?? "",
          username: editing.username ?? "",
          password: "",
          apiKey: "",
          category: editing.category,
          enabled: editing.enabled,
          isDefaultForProtocol: editing.isDefaultForProtocol,
        }
      : defaultValues,
  });

  const useSsl = watch("useSsl");
  const impl = watch("implementation");
  const isSabnzbd = impl === "sabnzbd";

  return (
    <FormModal
      open={open}
      onOpenChange={(o) => {
        if (!o) {
          onClose();
          setTestResult(null);
        }
      }}
      title={editing ? "Edit Download Client" : "Add Download Client"}
    >
      <form
        onSubmit={handleSubmit(async (data) => {
          await onSubmit(data);
          onClose();
          setTestResult(null);
        })}
        className="space-y-4"
      >
        {/* Type selector — only for new clients */}
        {!editing && (
          <div>
            <label className="block text-xs text-muted mb-1">Type</label>
            <select
              {...register("implementation")}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            >
              <option value="qBittorrent">qBittorrent</option>
              <option value="sabnzbd">SABnzbd</option>
            </select>
          </div>
        )}

        {editing && (
          <div className="text-xs text-muted mb-1">
            Implementation:{" "}
            <span className="font-medium text-zinc-300">{editing.implementation === "sabnzbd" ? "SABnzbd" : "qBittorrent"}</span>
          </div>
        )}

        <div>
          <label className="block text-xs text-muted mb-1">Name</label>
          <input
            {...register("name", { required: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>

        <div className="grid grid-cols-3 gap-3">
          <div className="col-span-2">
            <label className="block text-xs text-muted mb-1">Host</label>
            <input
              {...register("host", { required: true })}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-xs text-muted mb-1">Port</label>
            <input
              type="number"
              {...register("port", { required: true, valueAsNumber: true })}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
        </div>

        <div className="flex gap-6">
          <Controller
            name="useSsl"
            control={control}
            render={({ field }) => (
              <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
                <input
                  type="checkbox"
                  checked={field.value}
                  onChange={field.onChange}
                  className="rounded border-border"
                />
                Use SSL
              </label>
            )}
          />
          {useSsl && (
            <Controller
              name="skipSslValidation"
              control={control}
              render={({ field }) => (
                <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={field.value}
                    onChange={field.onChange}
                    className="rounded border-border"
                  />
                  Skip SSL Validation
                </label>
              )}
            />
          )}
        </div>

        <div>
          <label className="block text-xs text-muted mb-1">URL Base</label>
          <input
            {...register("urlBase")}
            placeholder={isSabnzbd ? "/sabnzbd" : "/qbittorrent"}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>

        {/* Conditional auth fields */}
        {isSabnzbd ? (
          <div>
            <label className="block text-xs text-muted mb-1">API Key</label>
            <input
              type="password"
              {...register("apiKey")}
              placeholder={editing?.apiKeySet ? "Leave blank to keep" : ""}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none font-mono text-xs"
            />
          </div>
        ) : (
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-xs text-muted mb-1">Username</label>
              <input
                {...register("username")}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
            <div>
              <label className="block text-xs text-muted mb-1">Password</label>
              <input
                type="password"
                {...register("password")}
                placeholder={editing ? "Leave blank to keep" : ""}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
          </div>
        )}

        <div>
          <label className="block text-xs text-muted mb-1">Category</label>
          <input
            {...register("category", { required: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>

        <Controller
          name="enabled"
          control={control}
          render={({ field }) => (
            <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
              <input
                type="checkbox"
                checked={field.value}
                onChange={field.onChange}
                className="rounded border-border"
              />
              Enabled
            </label>
          )}
        />

        <div className="flex items-center gap-3 pt-2">
          <button
            type="button"
            onClick={() => {
              setTestResult(null);
              const vals = getValues();
              // When editing and no new credentials entered, test the saved
              // client so the server reads credentials from DB.
              if (editing && !vals.password && !vals.apiKey) {
                testSaved.mutate(editing.id);
              } else {
                testClient.mutate(toRequest(vals));
              }
            }}
            disabled={testClient.isPending || testSaved.isPending}
            className="rounded border border-border px-4 py-2 text-sm text-zinc-200 hover:bg-zinc-700 disabled:opacity-50"
          >
            {testClient.isPending || testSaved.isPending ? "Testing..." : "Test Connection"}
          </button>
          {testResult === "success" && (
            <CheckCircle2 size={18} className="text-green-400" />
          )}
          {testResult === "fail" && (
            <XCircle size={18} className="text-red-400" />
          )}
          <div className="flex-1" />
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

// ── Inline Add Client Form ──

function InlineClientForm({
  onSubmit,
}: {
  onSubmit: (data: ClientFormData) => Promise<void>;
}) {
  const {
    register,
    handleSubmit,
    watch,
    getValues,
    control,
    reset,
    formState: { isSubmitting },
  } = useForm<ClientFormData>({ defaultValues });

  const [testResult, setTestResult] = useState<"success" | "fail" | null>(null);

  const testClient = useMutation({
    mutationFn: api.testDownloadClient,
    onSuccess: () => { setTestResult("success"); toast.success("Connection successful"); },
    onError: (e: Error) => { setTestResult("fail"); toast.error(e.message); },
  });

  const useSsl = watch("useSsl");
  const impl = watch("implementation");
  const isSabnzbd = impl === "sabnzbd";

  return (
    <form
      onSubmit={handleSubmit(async (data) => {
        await onSubmit(data);
        reset(defaultValues);
        setTestResult(null);
      })}
      className="space-y-4 max-w-xl"
    >
      <div>
        <label className="block text-xs text-muted mb-1">Type</label>
        <select
          {...register("implementation")}
          className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
        >
          <option value="qBittorrent">qBittorrent</option>
          <option value="sabnzbd">SABnzbd</option>
        </select>
      </div>

      <div>
        <label className="block text-xs text-muted mb-1">Name</label>
        <input
          {...register("name", { required: true })}
          className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
        />
      </div>

      <div className="grid grid-cols-3 gap-3">
        <div className="col-span-2">
          <label className="block text-xs text-muted mb-1">Host</label>
          <input
            {...register("host", { required: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
        <div>
          <label className="block text-xs text-muted mb-1">Port</label>
          <input
            type="number"
            {...register("port", { required: true, valueAsNumber: true })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
      </div>

      <div className="flex gap-6">
        <Controller name="useSsl" control={control} render={({ field }) => (
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input type="checkbox" checked={field.value} onChange={field.onChange} className="rounded border-border" />
            Use SSL
          </label>
        )} />
        {useSsl && (
          <Controller name="skipSslValidation" control={control} render={({ field }) => (
            <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
              <input type="checkbox" checked={field.value} onChange={field.onChange} className="rounded border-border" />
              Skip SSL Validation
            </label>
          )} />
        )}
      </div>

      <div>
        <label className="block text-xs text-muted mb-1">URL Base</label>
        <input
          {...register("urlBase")}
          placeholder={isSabnzbd ? "/sabnzbd" : "/qbittorrent"}
          className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
        />
      </div>

      {isSabnzbd ? (
        <div>
          <label className="block text-xs text-muted mb-1">API Key</label>
          <input
            type="password"
            {...register("apiKey")}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-xs text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
      ) : (
        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs text-muted mb-1">Username</label>
            <input
              {...register("username")}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-xs text-muted mb-1">Password</label>
            <input
              type="password"
              {...register("password")}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
        </div>
      )}

      <div>
        <label className="block text-xs text-muted mb-1">Category</label>
        <input
          {...register("category", { required: true })}
          className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
        />
      </div>

      <div className="flex items-center gap-3 pt-2">
        <button
          type="button"
          onClick={() => { setTestResult(null); testClient.mutate(toRequest(getValues())); }}
          disabled={testClient.isPending}
          className="rounded border border-border px-4 py-2 text-sm text-zinc-200 hover:bg-zinc-700 disabled:opacity-50"
        >
          {testClient.isPending ? "Testing..." : "Test Connection"}
        </button>
        {testResult === "success" && <CheckCircle2 size={18} className="text-green-400" />}
        {testResult === "fail" && <XCircle size={18} className="text-red-400" />}
        <div className="flex-1" />
        <button
          type="submit"
          disabled={isSubmitting}
          className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
        >
          {isSubmitting ? "Adding..." : "Add Client"}
        </button>
      </div>
    </form>
  );
}

// ── Prowlarr Import Section ──

function ProwlarrImportSection({ onImported }: { onImported: () => void }) {
  const [open, setOpen] = useState(false);
  const [url, setUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [result, setResult] = useState<ProwlarrImportResponse | null>(null);

  // Always fetch config so we can show status on the collapsed header.
  const configQ = useQuery({
    queryKey: ["prowlarrConfig"],
    queryFn: api.getProwlarrConfig,
  });
  // Pre-fill URL when section is first opened and config is available.
  useEffect(() => {
    if (open && !loaded && configQ.data) {
      if (configQ.data.url) setUrl(configQ.data.url);
      setLoaded(true);
    }
  }, [open, loaded, configQ.data]);

  const importMutation = useMutation({
    mutationFn: api.importDownloadClientsFromProwlarr,
    onSuccess: (data) => {
      setResult(data);
      if (data.imported > 0) {
        toast.success(`Imported ${data.imported} download client${data.imported !== 1 ? "s" : ""}`);
        onImported();
      } else if (data.skipped > 0) {
        toast.info("All download clients already exist — nothing to import");
      }
    },
    onError: (e: Error) => toast.error(e.message),
  });

  return (
    <section data-tour="prowlarr-import-dc" className="rounded-lg border border-border bg-zinc-900/50">
      <button
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-2 px-5 py-3 text-left rounded-t-lg bg-zinc-800/60 hover:bg-zinc-800/80"
      >
        {open ? (
          <ChevronDown size={16} className="text-muted" />
        ) : (
          <ChevronRight size={16} className="text-muted" />
        )}
        <Download size={16} className="text-muted" />
        <h2 className="text-base font-semibold text-zinc-100">
          Import from Prowlarr
        </h2>
      </button>

      {open && (
        <div className="p-5 space-y-4 max-w-xl">
          <p className="text-sm text-muted">
            Import download clients from your Prowlarr instance. Only qBittorrent and SABnzbd are
            supported. Duplicates (matching host + port + type) will be skipped.
          </p>

          <div>
            <label className="block text-xs text-muted mb-1">Prowlarr URL</label>
            <input
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="http://localhost:9696"
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>

          <div>
            <label className="block text-xs text-muted mb-1">
              API Key{" "}
              {configQ.data?.apiKeySet && !apiKey && (
                <span className="text-green-400 ml-1">(saved — leave blank to reuse)</span>
              )}
            </label>
            <input
              type="password"
              value={apiKey}
              onChange={(e) => setApiKey(e.target.value)}
              placeholder={configQ.data?.apiKeySet ? "Leave blank to use saved key" : ""}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-xs text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>

          {result && (
            <div
              className={`rounded border p-3 text-sm ${
                result.errors.length > 0
                  ? "border-amber-500/30 bg-amber-500/10"
                  : "border-green-500/30 bg-green-500/10"
              }`}
            >
              <p className="text-zinc-200">
                Imported: <span className="font-medium text-green-400">{result.imported}</span>
                {" · "}Skipped: <span className="font-medium text-zinc-400">{result.skipped}</span>
              </p>
              {result.errors.map((err, i) => (
                <p key={i} className="flex items-center gap-1 text-amber-400 mt-1">
                  <AlertTriangle size={12} /> {err}
                </p>
              ))}
            </div>
          )}

          <button
            onClick={() => {
              setResult(null);
              importMutation.mutate({ url, apiKey });
            }}
            disabled={importMutation.isPending || !url || (!apiKey && !configQ.data?.apiKeySet)}
            className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {importMutation.isPending ? "Importing..." : "Import Download Clients"}
          </button>
        </div>
      )}
    </section>
  );
}
