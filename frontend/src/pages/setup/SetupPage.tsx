import { useState } from "react";
import { useForm } from "react-hook-form";
import { useNavigate } from "react-router";
import { toast } from "sonner";
import { useAuthStore } from "@/stores/auth";
import {
  createRootFolder,
  testIndexer,
  createIndexer,
  testDownloadClient,
  createDownloadClient,
  updateMetadataConfig,
} from "@/api";

type Step =
  | "account"
  | "rootFolders"
  | "indexer"
  | "downloadClient"
  | "metadata"
  | "summary";

const STEPS: Step[] = [
  "account",
  "rootFolders",
  "indexer",
  "downloadClient",
  "metadata",
  "summary",
];

interface AccountForm {
  username: string;
  password: string;
  confirmPassword: string;
}

interface RootFolderForm {
  path: string;
  mediaType: "ebook" | "audiobook";
}

interface IndexerForm {
  name: string;
  url: string;
  apiPath: string;
  apiKey: string;
}

interface DownloadClientForm {
  name: string;
  host: string;
  port: number;
  username: string;
  password: string;
  category: string;
}

function toCreateDCRequest(
  d: DownloadClientForm,
): import("@/types/api").CreateDownloadClientRequest {
  return {
    ...d,
    implementation: "qBittorrent",
    useSsl: false,
    skipSslValidation: false,
    urlBase: null,
    enabled: true,
  };
}

interface MetadataForm {
  hardcoverApiToken: string;
  audnexusUrl: string;
}

export function SetupPage() {
  const navigate = useNavigate();
  const setupAction = useAuthStore((s) => s.setupAction);
  const [step, setStep] = useState<Step>("account");
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Collected config for summary
  const [config, setConfig] = useState<{
    username?: string;
    rootFolders: RootFolderForm[];
    indexer?: IndexerForm;
    downloadClient?: DownloadClientForm;
    metadata?: MetadataForm;
  }>({ rootFolders: [] });

  const stepIndex = STEPS.indexOf(step);
  const goNext = () => {
    const next = STEPS[stepIndex + 1];
    if (next) setStep(next);
  };

  return (
    <div className="flex min-h-screen items-center justify-center bg-zinc-900 text-zinc-100 p-4">
      <div className="w-full max-w-lg space-y-6">
        <div className="text-center">
          <h1 className="text-3xl font-bold">Livrarr Setup</h1>
          <p className="mt-1 text-sm text-zinc-400">
            Step {stepIndex + 1} of {STEPS.length}
          </p>
        </div>

        {step === "account" && (
          <AccountStep
            error={error}
            onSubmit={async (data) => {
              setError(null);
              try {
                const key = await setupAction(data.username, data.password);
                setApiKey(key);
                setConfig((c) => ({ ...c, username: data.username }));
                goNext();
              } catch (e: any) {
                setError(e?.message ?? "Setup failed");
              }
            }}
            apiKey={apiKey}
          />
        )}

        {step === "rootFolders" && (
          <RootFolderStep
            onAdd={async (data) => {
              try {
                await createRootFolder(data.path, data.mediaType);
                setConfig((c) => ({
                  ...c,
                  rootFolders: [...c.rootFolders, data],
                }));
                toast.success("Root folder added");
              } catch (e: any) {
                toast.error(e?.message ?? "Failed to add root folder");
              }
            }}
            folders={config.rootFolders}
            onNext={goNext}
            onSkip={goNext}
          />
        )}

        {step === "indexer" && (
          <IndexerStep
            onSave={async (data) => {
              try {
                await createIndexer({
                  name: data.name,
                  url: data.url,
                  apiPath: data.apiPath || "/",
                  apiKey: data.apiKey || null,
                  categories: [7020, 3030],
                });
                setConfig((c) => ({ ...c, indexer: data }));
                toast.success("Indexer added");
                goNext();
              } catch (e: any) {
                toast.error(e?.message ?? "Failed to add indexer");
              }
            }}
            onSkip={goNext}
          />
        )}

        {step === "downloadClient" && (
          <DownloadClientStep
            onSave={async (data) => {
              try {
                await createDownloadClient(toCreateDCRequest(data));
                setConfig((c) => ({ ...c, downloadClient: data }));
                toast.success("Download client configured");
                goNext();
              } catch (e: any) {
                toast.error(e?.message ?? "Failed to save download client");
              }
            }}
            onSkip={goNext}
          />
        )}

        {step === "metadata" && (
          <MetadataStep
            onSave={async (data) => {
              try {
                await updateMetadataConfig(data);
                setConfig((c) => ({ ...c, metadata: data }));
                toast.success("Metadata configured");
                goNext();
              } catch (e: any) {
                toast.error(e?.message ?? "Failed to save metadata config");
              }
            }}
            onSkip={goNext}
          />
        )}

        {step === "summary" && (
          <SummaryStep
            config={config}
            apiKey={apiKey}
            onFinish={() => navigate("/")}
          />
        )}
      </div>
    </div>
  );
}

// --- Sub-step components ---

function AccountStep({
  onSubmit,
  error,
  apiKey,
}: {
  onSubmit: (d: AccountForm) => Promise<void>;
  error: string | null;
  apiKey: string | null;
}) {
  const {
    register,
    handleSubmit,
    watch,
    formState: { isSubmitting, errors },
  } = useForm<AccountForm>();

  return (
    <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
      <h2 className="text-xl font-semibold">Create Account</h2>
      <Field label="Username" error={errors.username?.message}>
        <input
          {...register("username", { required: "Required" })}
          className="input-field"
          autoFocus
        />
      </Field>
      <Field label="Password" error={errors.password?.message}>
        <input
          {...register("password", {
            required: "Required",
            minLength: { value: 8, message: "Min 8 characters" },
          })}
          type="password"
          className="input-field"
        />
      </Field>
      <Field label="Confirm Password" error={errors.confirmPassword?.message}>
        <input
          {...register("confirmPassword", {
            required: "Required",
            validate: (v) => v === watch("password") || "Passwords don't match",
          })}
          type="password"
          className="input-field"
        />
      </Field>
      {error && <p className="text-sm text-red-400">{error}</p>}
      {apiKey && (
        <div className="rounded bg-zinc-800 p-3 text-sm">
          <p className="font-medium text-zinc-300">Your API Key:</p>
          <code className="mt-1 block break-all text-amber-400">{apiKey}</code>
          <p className="mt-1 text-xs text-zinc-500">
            Save this — it won't be shown again.
          </p>
        </div>
      )}
      <button
        type="submit"
        disabled={isSubmitting}
        className="btn-primary w-full"
      >
        {isSubmitting ? "Creating..." : "Create Account & Continue"}
      </button>
    </form>
  );
}

function RootFolderStep({
  onAdd,
  folders,
  onNext,
  onSkip,
}: {
  onAdd: (d: RootFolderForm) => Promise<void>;
  folders: RootFolderForm[];
  onNext: () => void;
  onSkip: () => void;
}) {
  const {
    register,
    handleSubmit,
    reset,
    formState: { isSubmitting, errors },
  } = useForm<RootFolderForm>();

  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold">Root Folders</h2>
      <p className="text-sm text-zinc-400">Where your media files live.</p>
      {folders.length > 0 && (
        <ul className="space-y-1 text-sm">
          {folders.map((f, i) => (
            <li key={i} className="rounded bg-zinc-800 px-3 py-2">
              {f.path} — <span className="text-zinc-400">{f.mediaType}</span>
            </li>
          ))}
        </ul>
      )}
      <form
        onSubmit={handleSubmit(async (d) => {
          await onAdd(d);
          reset();
        })}
        className="space-y-3"
      >
        <Field label="Path" error={errors.path?.message}>
          <input
            {...register("path", { required: "Required" })}
            className="input-field"
            placeholder="/media/books"
          />
        </Field>
        <Field label="Media Type" error={errors.mediaType?.message}>
          <select
            {...register("mediaType", { required: "Required" })}
            className="input-field"
          >
            <option value="ebook">Ebook</option>
            <option value="audiobook">Audiobook</option>
          </select>
        </Field>
        <button type="submit" disabled={isSubmitting} className="btn-secondary">
          Add Folder
        </button>
      </form>
      <StepNav
        onSkip={onSkip}
        onNext={folders.length > 0 ? onNext : undefined}
      />
    </div>
  );
}

function IndexerStep({
  onSave,
  onSkip,
}: {
  onSave: (d: IndexerForm) => Promise<void>;
  onSkip: () => void;
}) {
  const {
    register,
    handleSubmit,
    getValues,
    formState: { isSubmitting, errors },
  } = useForm<IndexerForm>({
    defaultValues: { apiPath: "/" },
  });
  const [testing, setTesting] = useState(false);

  const handleTest = async () => {
    setTesting(true);
    try {
      const vals = getValues();
      const result = await testIndexer({
        url: vals.url,
        apiPath: vals.apiPath || "/",
        apiKey: vals.apiKey || null,
      });
      if (result.ok) {
        toast.success(
          result.supportsBookSearch
            ? "Connected — book search supported"
            : "Connected — freetext search only",
        );
      } else {
        toast.error(result.error ?? "Test failed");
      }
    } catch {
      toast.error("Connection failed");
    } finally {
      setTesting(false);
    }
  };

  return (
    <form onSubmit={handleSubmit(onSave)} className="space-y-4">
      <h2 className="text-xl font-semibold">Torznab Indexer</h2>
      <p className="text-sm text-zinc-400">
        Add a Torznab-compatible indexer (e.g. Jackett, Prowlarr, or a native
        tracker).
      </p>
      <Field label="Name" error={errors.name?.message}>
        <input
          {...register("name", { required: "Required" })}
          className="input-field"
          placeholder="My Indexer"
        />
      </Field>
      <div className="grid grid-cols-3 gap-3">
        <Field label="URL" error={errors.url?.message} className="col-span-2">
          <input
            {...register("url", { required: "Required" })}
            className="input-field"
            placeholder="https://indexer.example.com"
          />
        </Field>
        <Field label="API Path" error={errors.apiPath?.message}>
          <input
            {...register("apiPath")}
            className="input-field"
            placeholder="/"
          />
        </Field>
      </div>
      <Field label="API Key" error={errors.apiKey?.message}>
        <input {...register("apiKey")} className="input-field" />
      </Field>
      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleTest}
          disabled={testing}
          className="btn-secondary"
        >
          {testing ? "Testing..." : "Test"}
        </button>
        <button type="submit" disabled={isSubmitting} className="btn-primary">
          Save & Continue
        </button>
      </div>
      <StepNav onSkip={onSkip} />
    </form>
  );
}

function DownloadClientStep({
  onSave,
  onSkip,
}: {
  onSave: (d: DownloadClientForm) => Promise<void>;
  onSkip: () => void;
}) {
  const {
    register,
    handleSubmit,
    getValues,
    formState: { isSubmitting, errors },
  } = useForm<DownloadClientForm>({
    defaultValues: { port: 8080, category: "livrarr" },
  });
  const [testing, setTesting] = useState(false);

  const handleTest = async () => {
    setTesting(true);
    try {
      await testDownloadClient(toCreateDCRequest(getValues()));
      toast.success("Connection successful");
    } catch {
      toast.error("Connection failed");
    } finally {
      setTesting(false);
    }
  };

  return (
    <form onSubmit={handleSubmit(onSave)} className="space-y-4">
      <h2 className="text-xl font-semibold">Download Client (qBittorrent)</h2>
      <Field label="Name" error={errors.name?.message}>
        <input
          {...register("name", { required: "Required" })}
          className="input-field"
          placeholder="qBittorrent"
        />
      </Field>
      <div className="grid grid-cols-3 gap-3">
        <Field label="Host" error={errors.host?.message} className="col-span-2">
          <input
            {...register("host", { required: "Required" })}
            className="input-field"
            placeholder="localhost"
          />
        </Field>
        <Field label="Port" error={errors.port?.message}>
          <input
            {...register("port", { required: "Required", valueAsNumber: true })}
            type="number"
            className="input-field"
          />
        </Field>
      </div>
      <div className="grid grid-cols-2 gap-3">
        <Field label="Username" error={errors.username?.message}>
          <input {...register("username")} className="input-field" />
        </Field>
        <Field label="Password" error={errors.password?.message}>
          <input
            {...register("password")}
            type="password"
            className="input-field"
          />
        </Field>
      </div>
      <Field label="Category" error={errors.category?.message}>
        <input {...register("category")} className="input-field" />
      </Field>
      <div className="flex gap-2">
        <button
          type="button"
          onClick={handleTest}
          disabled={testing}
          className="btn-secondary"
        >
          {testing ? "Testing..." : "Test"}
        </button>
        <button type="submit" disabled={isSubmitting} className="btn-primary">
          Save & Continue
        </button>
      </div>
      <StepNav onSkip={onSkip} />
    </form>
  );
}

function MetadataStep({
  onSave,
  onSkip,
}: {
  onSave: (d: MetadataForm) => Promise<void>;
  onSkip: () => void;
}) {
  const {
    register,
    handleSubmit,
    formState: { isSubmitting, errors },
  } = useForm<MetadataForm>({
    defaultValues: { audnexusUrl: "https://api.audnex.us" },
  });

  return (
    <form onSubmit={handleSubmit(onSave)} className="space-y-4">
      <h2 className="text-xl font-semibold">Metadata Providers</h2>
      <Field
        label="Hardcover API Token"
        error={errors.hardcoverApiToken?.message}
      >
        <input {...register("hardcoverApiToken")} className="input-field" />
      </Field>
      <Field label="Audnexus URL" error={errors.audnexusUrl?.message}>
        <input {...register("audnexusUrl")} className="input-field" />
      </Field>
      <button
        type="submit"
        disabled={isSubmitting}
        className="btn-primary w-full"
      >
        Save & Continue
      </button>
      <StepNav onSkip={onSkip} />
    </form>
  );
}

function SummaryStep({
  config,
  apiKey,
  onFinish,
}: {
  config: {
    username?: string;
    rootFolders: RootFolderForm[];
    indexer?: IndexerForm;
    downloadClient?: DownloadClientForm;
    metadata?: MetadataForm;
  };
  apiKey: string | null;
  onFinish: () => void;
}) {
  return (
    <div className="space-y-4">
      <h2 className="text-xl font-semibold">Setup Complete</h2>
      <dl className="space-y-2 text-sm">
        <SummaryRow label="User" value={config.username} />
        <SummaryRow
          label="Root Folders"
          value={
            config.rootFolders.length > 0
              ? config.rootFolders
                  .map((f) => `${f.path} (${f.mediaType})`)
                  .join(", ")
              : "None"
          }
        />
        <SummaryRow
          label="Indexer"
          value={config.indexer ? config.indexer.name : "Skipped"}
        />
        <SummaryRow
          label="Download Client"
          value={config.downloadClient ? config.downloadClient.name : "Skipped"}
        />
        <SummaryRow
          label="Metadata"
          value={config.metadata?.hardcoverApiToken ? "Configured" : "Skipped"}
        />
      </dl>
      {apiKey && (
        <div className="rounded bg-zinc-800 p-3 text-sm">
          <p className="font-medium text-zinc-300">
            API Key (last chance to copy):
          </p>
          <code className="mt-1 block break-all text-amber-400">{apiKey}</code>
        </div>
      )}
      <button onClick={onFinish} className="btn-primary w-full">
        Finish
      </button>
    </div>
  );
}

// --- Shared helpers ---

function Field({
  label,
  error,
  className,
  children,
}: {
  label: string;
  error?: string;
  className?: string;
  children: React.ReactNode;
}) {
  return (
    <label className={`block ${className ?? ""}`}>
      <span className="mb-1 block text-sm font-medium text-zinc-300">
        {label}
      </span>
      {children}
      {error && (
        <span className="mt-0.5 block text-xs text-red-400">{error}</span>
      )}
    </label>
  );
}

function StepNav({
  onSkip,
  onNext,
}: {
  onSkip: () => void;
  onNext?: () => void;
}) {
  return (
    <div className="flex justify-between pt-2">
      <button
        type="button"
        onClick={onSkip}
        className="text-sm text-zinc-400 hover:text-zinc-200"
      >
        Skip
      </button>
      {onNext && (
        <button type="button" onClick={onNext} className="btn-primary">
          Next
        </button>
      )}
    </div>
  );
}

function SummaryRow({ label, value }: { label: string; value?: string }) {
  return (
    <div className="flex justify-between rounded bg-zinc-800 px-3 py-2">
      <dt className="text-zinc-400">{label}</dt>
      <dd>{value ?? "—"}</dd>
    </div>
  );
}
