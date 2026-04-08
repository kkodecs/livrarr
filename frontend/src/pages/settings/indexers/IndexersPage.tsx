import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm, Controller } from "react-hook-form";
import { HelpTip } from "@/components/HelpTip";
import { toast } from "sonner";
import {
  Plus,
  Trash2,
  Pencil,
  CheckCircle2,
  XCircle,
  AlertTriangle,
  Loader2,
  Zap,
} from "lucide-react";
import { useAuthStore } from "@/stores/auth";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { ConfirmModal } from "@/components/Page/ConfirmModal";
import { FormModal } from "@/components/Page/FormModal";
import type {
  IndexerResponse,
  CreateIndexerRequest,
  UpdateIndexerRequest,
  TestIndexerResponse,
} from "@/types/api";
import { useSort } from "@/hooks/useSort";
import { SortHeader } from "@/components/Page/SortHeader";
import * as api from "@/api";

type IndexerSortField = "name" | "url" | "priority" | "enabled";

// ── Form Types ──

interface IndexerFormData {
  name: string;
  protocol: "torrent" | "usenet";
  url: string;
  apiPath: string;
  apiKey: string;
  categories: string;
  priority: number;
  enableAutomaticSearch: boolean;
  enableInteractiveSearch: boolean;
  enabled: boolean;
}

function parseCategories(s: string): number[] {
  if (!s.trim()) return [];
  return s
    .split(",")
    .map((c) => parseInt(c.trim(), 10))
    .filter((n) => !isNaN(n));
}

function toCreateRequest(data: IndexerFormData): CreateIndexerRequest {
  return {
    name: data.name,
    protocol: data.protocol,
    url: data.url,
    apiPath: data.apiPath || "/",
    apiKey: data.apiKey || null,
    categories: parseCategories(data.categories),
    priority: data.priority,
    enableAutomaticSearch: data.enableAutomaticSearch,
    enableInteractiveSearch: data.enableInteractiveSearch,
    enabled: data.enabled,
  };
}

function toUpdateRequest(data: IndexerFormData): UpdateIndexerRequest {
  return {
    name: data.name,
    url: data.url,
    apiPath: data.apiPath || "/",
    apiKey: data.apiKey,
    categories: parseCategories(data.categories),
    priority: data.priority,
    enableAutomaticSearch: data.enableAutomaticSearch,
    enableInteractiveSearch: data.enableInteractiveSearch,
    enabled: data.enabled,
  };
}

const defaultValues: IndexerFormData = {
  name: "",
  protocol: "torrent",
  url: "",
  apiPath: "/",
  apiKey: "",
  categories: "7020, 3030",
  priority: 1,
  enableAutomaticSearch: true,
  enableInteractiveSearch: true,
  enabled: true,
};

// ── Main Page ──

export default function IndexersPage() {
  const isAdmin = useAuthStore((s) => s.isAdmin);
  const qc = useQueryClient();

  const indexersQ = useQuery({
    queryKey: ["indexers"],
    queryFn: api.listIndexers,
  });

  const createIndexer = useMutation({
    mutationFn: api.createIndexer,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["indexers"] });
      toast.success("Indexer added");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const updateIndexer = useMutation({
    mutationFn: ({
      id,
      data,
    }: {
      id: number;
      data: UpdateIndexerRequest;
    }) => api.updateIndexer(id, data),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["indexers"] });
      toast.success("Indexer updated");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const deleteIndexer = useMutation({
    mutationFn: api.deleteIndexer,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["indexers"] });
      toast.success("Indexer removed");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const [testingId, setTestingId] = useState<number | null>(null);
  const testSavedIndexer = useMutation({
    mutationFn: api.testSavedIndexer,
    onSuccess: (result) => {
      setTestingId(null);
      if (result.ok) {
        toast.success("Connection successful");
      } else {
        toast.error(result.error ?? "Test failed");
      }
    },
    onError: (e: Error) => {
      setTestingId(null);
      toast.error(e.message);
    },
  });

  const handleTestRow = (id: number) => {
    setTestingId(id);
    testSavedIndexer.mutate(id);
  };

  const [modal, setModal] = useState<{
    open: boolean;
    editing: IndexerResponse | null;
  }>({ open: false, editing: null });
  const [deleteTarget, setDeleteTarget] = useState<IndexerResponse | null>(
    null,
  );

  const sorting = useSort<IndexerSortField>("name");

  if (indexersQ.isLoading) return <PageLoading />;
  if (indexersQ.error)
    return (
      <ErrorState
        error={indexersQ.error as Error}
        onRetry={() => indexersQ.refetch()}
      />
    );

  const allIndexers = indexersQ.data ?? [];
  const sortFn = (item: IndexerResponse, field: IndexerSortField) => {
    switch (field) {
      case "name": return item.name;
      case "url": return item.url;
      case "priority": return item.priority;
      case "enabled": return item.enabled ? 0 : 1;
    }
  };
  const torrentIndexers = sorting.sort(allIndexers.filter((i) => i.protocol !== "usenet"), sortFn);
  const usenetIndexers = sorting.sort(allIndexers.filter((i) => i.protocol === "usenet"), sortFn);

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Indexers</h1>
        <HelpTip text="Indexers are search engines that Livrarr queries to find ebook and audiobook releases. You need at least one indexer configured to search. Each indexer requires a URL and API key from your account on that indexer." />
      </PageToolbar>

      <PageContent className="space-y-4">
        {/* ── Torrent Indexers ── */}
        <section>
          <div className="flex items-center gap-2 mb-4">
            <h2 className="text-base font-semibold text-zinc-100">
              Torrent Indexers
            </h2>
            <HelpTip text="Torznab indexers serve torrent files. Popular options for books include MyAnonamouse (MAM), TorrentLeech, and IPTorrents. You'll need an account on the indexer and a Torznab-compatible API URL." />
          </div>

          {torrentIndexers.length > 0 ? (
            <div className="overflow-x-auto rounded border border-border">
              <table className="w-full text-sm">
                <thead className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
                  <tr>
                    <SortHeader field="name" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Name</SortHeader>
                    <SortHeader field="url" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>URL</SortHeader>
                    <SortHeader field="priority" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Priority <HelpTip text="Lower number = higher priority. When multiple indexers return the same release, Livrarr prefers the one from the higher-priority indexer." /></SortHeader>
                    <th className="px-4 py-2">Capabilities</th>
                    <SortHeader field="enabled" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Status</SortHeader>
                    {isAdmin && <th className="px-4 py-2 w-32" />}
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {torrentIndexers.map((idx) => (
                    <tr key={idx.id} className="text-zinc-200">
                      <td className="px-4 py-2 font-medium">{idx.name}</td>
                      <td className="px-4 py-2 font-mono text-xs">
                        {idx.url}
                      </td>
                      <td className="px-4 py-2">{idx.priority}</td>
                      <td className="px-4 py-2">
                        <div className="flex gap-1.5">
                          {idx.supportsBookSearch && (
                            <span className="rounded bg-blue-500/20 px-1.5 py-0.5 text-xs text-blue-400">
                              Book
                            </span>
                          )}
                          {idx.enableInteractiveSearch && (
                            <span className="rounded bg-zinc-600/30 px-1.5 py-0.5 text-xs text-zinc-400">
                              Interactive
                            </span>
                          )}
                          {idx.enableAutomaticSearch && (
                            <span className="rounded bg-zinc-600/30 px-1.5 py-0.5 text-xs text-zinc-400">
                              Auto
                            </span>
                          )}
                        </div>
                      </td>
                      <td className="px-4 py-2">
                        <span
                          className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${idx.enabled ? "bg-green-500/20 text-green-400" : "bg-zinc-600/20 text-zinc-400"}`}
                        >
                          {idx.enabled ? "Enabled" : "Disabled"}
                        </span>
                      </td>
                      {isAdmin && (
                        <td className="px-4 py-2 flex gap-2">
                          <button
                            onClick={() => handleTestRow(idx.id)}
                            disabled={testingId === idx.id}
                            className="text-muted hover:text-blue-400 disabled:opacity-50"
                            title="Test connection"
                          >
                            {testingId === idx.id ? (
                              <Loader2 size={14} className="animate-spin" />
                            ) : (
                              <Zap size={14} />
                            )}
                          </button>
                          <button
                            onClick={() =>
                              setModal({ open: true, editing: idx })
                            }
                            className="text-muted hover:text-zinc-100"
                          >
                            <Pencil size={14} />
                          </button>
                          <button
                            onClick={() => setDeleteTarget(idx)}
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
            <p className="text-sm text-muted">No torrent indexers configured.</p>
          )}
        </section>

        {/* ── Usenet Indexers ── */}
        <section>
          <div className="flex items-center gap-2 mb-4">
            <h2 className="text-base font-semibold text-zinc-100">
              Usenet Indexers
            </h2>
            <HelpTip text="Newznab indexers serve NZB files for Usenet downloads. Popular options include DrunkenSlug, NZBGeek, and NZBFinder. Requires a Usenet provider (separate from the indexer) and a download client like SABnzbd." />
          </div>

          {usenetIndexers.length > 0 ? (
            <div className="overflow-x-auto rounded border border-border">
              <table className="w-full text-sm">
                <thead className="border-b border-border bg-zinc-800/50 text-left text-xs text-muted">
                  <tr>
                    <SortHeader field="name" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Name</SortHeader>
                    <SortHeader field="url" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>URL</SortHeader>
                    <SortHeader field="priority" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Priority <HelpTip text="Lower number = higher priority. When multiple indexers return the same release, Livrarr prefers the one from the higher-priority indexer." /></SortHeader>
                    <th className="px-4 py-2">Capabilities</th>
                    <SortHeader field="enabled" activeField={sorting.field} dir={sorting.dir} onSort={sorting.toggle}>Status</SortHeader>
                    {isAdmin && <th className="px-4 py-2 w-32" />}
                  </tr>
                </thead>
                <tbody className="divide-y divide-border">
                  {usenetIndexers.map((idx) => (
                    <tr key={idx.id} className="text-zinc-200">
                      <td className="px-4 py-2 font-medium">{idx.name}</td>
                      <td className="px-4 py-2 font-mono text-xs">{idx.url}</td>
                      <td className="px-4 py-2">{idx.priority}</td>
                      <td className="px-4 py-2">
                        <div className="flex gap-1.5">
                          {idx.enableInteractiveSearch && (
                            <span className="rounded bg-zinc-600/30 px-1.5 py-0.5 text-xs text-zinc-400">Interactive</span>
                          )}
                          {idx.enableAutomaticSearch && (
                            <span className="rounded bg-zinc-600/30 px-1.5 py-0.5 text-xs text-zinc-400">Auto</span>
                          )}
                        </div>
                      </td>
                      <td className="px-4 py-2">
                        <span className={`inline-flex rounded-full px-2 py-0.5 text-xs font-medium ${idx.enabled ? "bg-green-500/20 text-green-400" : "bg-zinc-600/20 text-zinc-400"}`}>
                          {idx.enabled ? "Enabled" : "Disabled"}
                        </span>
                      </td>
                      {isAdmin && (
                        <td className="px-4 py-2 flex gap-2">
                          <button
                            onClick={() => handleTestRow(idx.id)}
                            disabled={testingId === idx.id}
                            className="text-muted hover:text-blue-400 disabled:opacity-50"
                            title="Test connection"
                          >
                            {testingId === idx.id ? (
                              <Loader2 size={14} className="animate-spin" />
                            ) : (
                              <Zap size={14} />
                            )}
                          </button>
                          <button onClick={() => setModal({ open: true, editing: idx })} className="text-muted hover:text-zinc-100">
                            <Pencil size={14} />
                          </button>
                          <button onClick={() => setDeleteTarget(idx)} className="text-muted hover:text-red-400">
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
            <p className="text-sm text-muted">No Usenet indexers configured.</p>
          )}
        </section>

        {/* ── Inline Add Form ── */}
        {isAdmin && (
          <section data-tour="add-indexer-form" className="rounded-lg border border-border bg-zinc-900/50">
            <div className="flex items-center gap-2 border-b border-border bg-zinc-800/60 px-5 py-3 rounded-t-lg">
              <Plus size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">Add Indexer</h2>
            </div>
            <div className="p-5">
              <InlineIndexerForm
                onSubmit={async (data) => {
                  await createIndexer.mutateAsync(toCreateRequest(data));
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
        title="Delete Indexer"
        description={`Remove "${deleteTarget?.name}"? Releases from this indexer will no longer appear in search results.`}
        confirmLabel="Delete"
        onConfirm={() => {
          if (deleteTarget) return deleteIndexer.mutateAsync(deleteTarget.id);
        }}
      />

      {/* ── Add/Edit Modal ── */}
      <IndexerFormModal
        open={modal.open}
        editing={modal.editing}
        onClose={() => setModal({ open: false, editing: null })}
        onSubmit={async (data) => {
          if (modal.editing) {
            await updateIndexer.mutateAsync({
              id: modal.editing.id,
              data: toUpdateRequest(data),
            });
          } else {
            await createIndexer.mutateAsync(toCreateRequest(data));
          }
        }}
      />
    </>
  );
}

// ── Indexer Form Modal ──

function IndexerFormModal({
  open,
  editing,
  onClose,
  onSubmit,
}: {
  open: boolean;
  editing: IndexerResponse | null;
  onClose: () => void;
  onSubmit: (data: IndexerFormData) => Promise<void>;
}) {
  const [testResult, setTestResult] = useState<TestIndexerResponse | null>(
    null,
  );

  const handleTestResult = (result: TestIndexerResponse) => {
    setTestResult(result);
    if (result.ok) {
      toast.success("Connection successful");
    } else {
      toast.error(result.error ?? "Test failed");
    }
  };
  const handleTestError = (e: Error) => {
    setTestResult({ ok: false, supportsBookSearch: false, error: e.message });
    toast.error(e.message);
  };

  const testIndexer = useMutation({
    mutationFn: api.testIndexer,
    onSuccess: handleTestResult,
    onError: handleTestError,
  });

  const testSaved = useMutation({
    mutationFn: api.testSavedIndexer,
    onSuccess: handleTestResult,
    onError: handleTestError,
  });

  const {
    register,
    handleSubmit,
    getValues,
    control,
    reset,
    formState: { isSubmitting },
  } = useForm<IndexerFormData>({
    values: editing
      ? {
          name: editing.name,
          protocol: editing.protocol,
          url: editing.url,
          apiPath: editing.apiPath,
          apiKey: "",
          categories: editing.categories.join(", "),
          priority: editing.priority,
          enableAutomaticSearch: editing.enableAutomaticSearch,
          enableInteractiveSearch: editing.enableInteractiveSearch,
          enabled: editing.enabled,
        }
      : defaultValues,
  });

  const handleClose = () => {
    onClose();
    setTestResult(null);
    reset(defaultValues);
  };

  return (
    <FormModal
      open={open}
      onOpenChange={(o) => {
        if (!o) handleClose();
      }}
      title={editing ? "Edit Indexer" : "Add Indexer"}
    >
      <form
        onSubmit={handleSubmit(async (data) => {
          await onSubmit(data);
          handleClose();
        })}
        className="space-y-4"
      >
        <div>
          <label className="block text-xs text-muted mb-1">Name</label>
          <input
            {...register("name", { required: true })}
            placeholder="My Indexer"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>

        {!editing && (
          <div>
            <div className="flex items-center gap-2 mb-1">
              <label className="block text-xs text-muted">Protocol</label>
              <HelpTip text="Torznab indexers serve torrent files (e.g., MyAnonamouse, TorrentLeech). Newznab indexers serve NZB/Usenet files (e.g., DrunkenSlug, NZBGeek)." />
            </div>
            <select
              {...register("protocol")}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            >
              <option value="torrent">Torznab (Torrent)</option>
              <option value="usenet">Newznab (Usenet)</option>
            </select>
          </div>
        )}

        {editing && (
          <div className="text-xs text-muted mb-1">
            Protocol: <span className="font-medium text-zinc-300">{editing.protocol === "usenet" ? "Newznab (Usenet)" : "Torznab (Torrent)"}</span>
          </div>
        )}

        <div className="grid grid-cols-3 gap-3">
          <div className="col-span-2">
            <label className="block text-xs text-muted mb-1">URL</label>
            <input
              {...register("url", { required: true })}
              placeholder="https://indexer.example.com"
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
          <div>
            <label className="block text-xs text-muted mb-1">API Path</label>
            <input
              {...register("apiPath")}
              placeholder="/"
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
        </div>

        <div>
          <label className="block text-xs text-muted mb-1">
            API Key{" "}
            {editing?.apiKeySet && (
              <span className="text-green-400 ml-1">(configured)</span>
            )}
          </label>
          <input
            type="password"
            {...register("apiKey")}
            placeholder={editing?.apiKeySet ? "Leave blank to keep" : ""}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>

        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs text-muted mb-1">
              Categories
            </label>
            <input
              {...register("categories")}
              placeholder="7020, 3030"
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
            <p className="mt-0.5 text-xs text-zinc-500">
              7020 = Ebooks, 3030 = Audiobooks
            </p>
          </div>
          <div>
            <div className="flex items-center gap-2 mb-1"><label className="block text-xs text-muted">Priority</label><HelpTip text="Lower number = higher priority. When multiple indexers return the same release, Livrarr prefers the one from the higher-priority indexer." /></div>
            <input
              type="number"
              {...register("priority", {
                required: true,
                valueAsNumber: true,
                min: 1,
              })}
              className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
            />
          </div>
        </div>

        <div className="flex gap-6">
          <Controller
            name="enableInteractiveSearch"
            control={control}
            render={({ field }) => (
              <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
                <input
                  type="checkbox"
                  checked={field.value}
                  onChange={field.onChange}
                  className="rounded border-border"
                />
                Interactive Search
              </label>
            )}
          />
          <Controller
            name="enableAutomaticSearch"
            control={control}
            render={({ field }) => (
              <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
                <input
                  type="checkbox"
                  checked={field.value}
                  onChange={field.onChange}
                  className="rounded border-border"
                />
                Automatic Search
              </label>
            )}
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

        {/* Test result details */}
        {testResult && (
          <div
            className={`rounded border p-3 text-sm ${testResult.ok ? "border-green-500/30 bg-green-500/10" : "border-red-500/30 bg-red-500/10"}`}
          >
            <div className="flex items-center gap-2 mb-1">
              {testResult.ok ? (
                <CheckCircle2 size={16} className="text-green-400" />
              ) : (
                <XCircle size={16} className="text-red-400" />
              )}
              <span className="font-medium text-zinc-200">
                {testResult.ok ? "Connection successful" : "Connection failed"}
              </span>
            </div>
            {testResult.ok && (
              <p className="text-zinc-400">
                Book search:{" "}
                <span
                  className={
                    testResult.supportsBookSearch
                      ? "text-green-400"
                      : "text-zinc-500"
                  }
                >
                  {testResult.supportsBookSearch ? "Supported" : "Not supported"}
                </span>
              </p>
            )}
            {testResult.error && (
              <p className="text-red-400">{testResult.error}</p>
            )}
            {testResult.warnings &&
              testResult.warnings.map((w, i) => (
                <p key={i} className="flex items-center gap-1 text-amber-400">
                  <AlertTriangle size={12} /> {w}
                </p>
              ))}
          </div>
        )}

        <div className="flex items-center gap-3 pt-2">
          <button
            type="button"
            onClick={() => {
              const vals = getValues();
              setTestResult(null);
              // If editing a saved indexer and no new API key entered, use the
              // saved-indexer test endpoint (which uses the stored key).
              if (editing && !vals.apiKey) {
                testSaved.mutate(editing.id);
              } else {
                if (!vals.url) {
                  toast.error("URL is required to test");
                  return;
                }
                testIndexer.mutate({
                  url: vals.url,
                  apiPath: vals.apiPath || "/",
                  apiKey: vals.apiKey || null,
                });
              }
            }}
            disabled={testIndexer.isPending || testSaved.isPending}
            className="rounded border border-border px-4 py-2 text-sm text-zinc-200 hover:bg-zinc-700 disabled:opacity-50"
          >
            {testIndexer.isPending || testSaved.isPending
              ? "Testing..."
              : "Test"}
          </button>
          <div className="flex-1" />
          <button
            type="button"
            onClick={handleClose}
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

// ── Inline Add Indexer Form ──

function InlineIndexerForm({
  onSubmit,
}: {
  onSubmit: (data: IndexerFormData) => Promise<void>;
}) {
  const {
    register,
    handleSubmit,
    control,
    reset,
    getValues,
    formState: { isSubmitting },
  } = useForm<IndexerFormData>({ defaultValues });

  const [testResult, setTestResult] = useState<TestIndexerResponse | null>(null);

  const handleTestResult = (result: TestIndexerResponse) => {
    setTestResult(result);
    if (result.ok) toast.success("Connection successful");
    else toast.error(result.error ?? "Test failed");
  };

  const testIndexer = useMutation({
    mutationFn: api.testIndexer,
    onSuccess: handleTestResult,
    onError: (e: Error) => {
      setTestResult({ ok: false, supportsBookSearch: false, error: e.message });
      toast.error(e.message);
    },
  });

  return (
    <form
      onSubmit={handleSubmit(async (data) => {
        await onSubmit(data);
        reset(defaultValues);
        setTestResult(null);
      })}
      className="space-y-4 max-w-xl"
    >
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="block text-xs text-muted mb-1">Name</label>
          <input
            {...register("name", { required: true })}
            placeholder="My Indexer"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
        <div>
          <div className="flex items-center gap-2 mb-1">
            <label className="block text-xs text-muted">Protocol</label>
            <HelpTip text="Torznab indexers serve torrent files (e.g., MyAnonamouse, TorrentLeech). Newznab indexers serve NZB/Usenet files (e.g., DrunkenSlug, NZBGeek)." />
          </div>
          <select
            {...register("protocol")}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          >
            <option value="torrent">Torznab (Torrent)</option>
            <option value="usenet">Newznab (Usenet)</option>
          </select>
        </div>
      </div>

      <div className="grid grid-cols-3 gap-3">
        <div className="col-span-2">
          <label className="block text-xs text-muted mb-1">URL</label>
          <input
            {...register("url", { required: true })}
            placeholder="https://indexer.example.com"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
        <div>
          <label className="block text-xs text-muted mb-1">API Path</label>
          <input
            {...register("apiPath")}
            placeholder="/"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
      </div>

      <div>
        <label className="block text-xs text-muted mb-1">API Key</label>
        <input
          type="password"
          {...register("apiKey")}
          className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
        />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="block text-xs text-muted mb-1">Categories</label>
          <input
            {...register("categories")}
            placeholder="7020, 3030"
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
          <p className="mt-0.5 text-xs text-zinc-500">7020 = Ebooks, 3030 = Audiobooks</p>
        </div>
        <div>
          <div className="flex items-center gap-2 mb-1"><label className="block text-xs text-muted">Priority</label><HelpTip text="Lower number = higher priority. When multiple indexers return the same release, Livrarr prefers the one from the higher-priority indexer." /></div>
          <input
            type="number"
            {...register("priority", { required: true, valueAsNumber: true, min: 1 })}
            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
          />
        </div>
      </div>

      <div className="flex gap-6">
        <Controller name="enableInteractiveSearch" control={control} render={({ field }) => (
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input type="checkbox" checked={field.value} onChange={field.onChange} className="rounded border-border" />
            Interactive Search
          </label>
        )} />
        <Controller name="enableAutomaticSearch" control={control} render={({ field }) => (
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input type="checkbox" checked={field.value} onChange={field.onChange} className="rounded border-border" />
            Automatic Search
          </label>
        )} />
      </div>

      {testResult && (
        <div className={`rounded border p-3 text-sm ${testResult.ok ? "border-green-500/30 bg-green-500/10" : "border-red-500/30 bg-red-500/10"}`}>
          <div className="flex items-center gap-2">
            {testResult.ok ? <CheckCircle2 size={16} className="text-green-400" /> : <XCircle size={16} className="text-red-400" />}
            <span className="font-medium text-zinc-200">{testResult.ok ? "Connection successful" : "Connection failed"}</span>
          </div>
          {testResult.ok && (
            <p className="text-zinc-400 mt-1">Book search: <span className={testResult.supportsBookSearch ? "text-green-400" : "text-zinc-500"}>{testResult.supportsBookSearch ? "Supported" : "Not supported"}</span></p>
          )}
          {testResult.error && <p className="text-red-400 mt-1">{testResult.error}</p>}
        </div>
      )}

      <div className="flex items-center gap-3 pt-2">
        <button
          type="button"
          onClick={() => {
            const vals = getValues();
            setTestResult(null);
            if (!vals.url) { toast.error("URL is required to test"); return; }
            testIndexer.mutate({ url: vals.url, apiPath: vals.apiPath || "/", apiKey: vals.apiKey || null });
          }}
          disabled={testIndexer.isPending}
          className="rounded border border-border px-4 py-2 text-sm text-zinc-200 hover:bg-zinc-700 disabled:opacity-50"
        >
          {testIndexer.isPending ? "Testing..." : "Test"}
        </button>
        <div className="flex-1" />
        <button
          type="submit"
          disabled={isSubmitting}
          className="rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
        >
          {isSubmitting ? "Adding..." : "Add Indexer"}
        </button>
      </div>
    </form>
  );
}
