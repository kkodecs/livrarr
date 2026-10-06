import { HelpTip } from "@/components/HelpTip";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm, Controller } from "react-hook-form";
import { toast } from "sonner";
import { BookOpen, Cpu, Globe, Languages, ExternalLink } from "lucide-react";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import type {
  LlmProvider,
  MetadataConfigResponse,
  UpdateMetadataConfigRequest,
} from "@/types/api";
import { SUPPORTED_LANGUAGES } from "@/types/api";
import * as api from "@/api";
import { useState } from "react";

// ── LLM Provider Configs ──

interface ProviderConfig {
  label: string;
  endpoint: string;
  models: { value: string; label: string }[];
  free: boolean;
  apiKeyUrl: string;
  apiKeyHelp: string;
}

const PROVIDER_CONFIGS: Record<string, ProviderConfig> = {
  groq: {
    label: "Groq (Free)",
    endpoint: "https://api.groq.com/openai/v1",
    models: [
      { value: "llama-3.3-70b-versatile", label: "Llama 3.3 70B Versatile (recommended)" },
      { value: "llama-3.1-8b-instant", label: "Llama 3.1 8B Instant (faster, higher limits)" },
    ],
    free: true,
    apiKeyUrl: "https://console.groq.com/keys",
    apiKeyHelp: "Create a free account at groq.com, then go to console.groq.com/keys to generate an API key.",
  },
  gemini: {
    label: "Gemini (Free)",
    endpoint: "https://generativelanguage.googleapis.com/v1beta/openai",
    models: [
      { value: "gemini-3.1-flash-lite", label: "Gemini 3.1 Flash-Lite (recommended)" },
      { value: "gemini-2.5-flash-lite", label: "Gemini 2.5 Flash-Lite" },
      { value: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
    ],
    free: true,
    apiKeyUrl: "https://aistudio.google.com/apikey",
    apiKeyHelp: "Go to aistudio.google.com/apikey to create a free API key with your Google account.",
  },
  openai: {
    label: "OpenAI (Paid)",
    endpoint: "https://api.openai.com/v1",
    models: [
      { value: "gpt-4o-mini", label: "GPT-4o Mini (recommended)" },
      { value: "gpt-4o", label: "GPT-4o" },
    ],
    free: false,
    apiKeyUrl: "https://platform.openai.com/api-keys",
    apiKeyHelp: "Go to platform.openai.com/api-keys. Requires a paid account with billing enabled.",
  },
  custom: {
    label: "Custom (OpenAI-compatible)",
    endpoint: "",
    models: [],
    free: false,
    apiKeyUrl: "",
    apiKeyHelp: "Enter the endpoint URL, API key, and model name for any OpenAI-compatible API.",
  },
};

// ── Form Types ──

interface MetadataForm {
  hardcoverEnabled: boolean;
  hardcoverApiToken: string;
  googleBooksApiKey: string;
  audnexusUrl: string;
  llmEnabled: boolean;
  llmProvider: LlmProvider | "";
  llmEndpoint: string;
  llmApiKey: string;
  llmModel: string;
}

/** The form as the saved settings fill it; secret fields are always blank. */
function formValuesFrom(config: MetadataConfigResponse | undefined): MetadataForm {
  return {
    hardcoverEnabled: config?.hardcoverEnabled ?? true,
    hardcoverApiToken: "",
    googleBooksApiKey: "",
    audnexusUrl: config?.audnexusUrl ?? "",
    llmEnabled: config?.llmEnabled ?? true,
    llmProvider: config?.llmProvider ?? "",
    llmEndpoint: config?.llmEndpoint ?? "",
    llmApiKey: "",
    llmModel: config?.llmModel ?? "",
  };
}

/** Same entries in the same order; the first entry is the primary language. */
function sameLanguageList(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((code, i) => code === b[i]);
}

const SAVE_FIRST_MESSAGE = "Save your changes first. Test checks the saved settings.";

interface TestButtonProps {
  run: () => Promise<void>;
  successMessage: string;
  hasUnsavedChanges: boolean;
}

/**
 * Tests one service against the saved settings. With unsaved changes on the
 * page it sends nothing and asks the user to save first.
 */
function TestButton({ run, successMessage, hasUnsavedChanges }: TestButtonProps) {
  const test = useMutation({
    mutationFn: run,
    onSuccess: () => toast.success(successMessage),
    onError: (e: Error) => toast.error(e.message),
  });

  return (
    <button
      type="button"
      disabled={test.isPending}
      onClick={() => {
        if (hasUnsavedChanges) {
          toast.warning(SAVE_FIRST_MESSAGE);
          return;
        }
        test.mutate();
      }}
      className="ml-auto rounded border border-border px-3 py-1 text-xs font-medium text-zinc-200 hover:bg-zinc-800 disabled:opacity-50"
    >
      {test.isPending ? "Testing..." : "Test"}
    </button>
  );
}

export default function MetadataPage() {
  const qc = useQueryClient();

  const configQ = useQuery({
    queryKey: ["metadataConfig"],
    queryFn: api.getMetadataConfig,
  });

  const defaultLangQ = useQuery({
    queryKey: ["defaultLanguage"],
    queryFn: api.getDefaultLanguage,
  });

  const {
    register,
    handleSubmit,
    control,
    watch,
    setValue,
    reset,
    formState: { isSubmitting, isDirty },
  } = useForm<MetadataForm>({
    values: formValuesFrom(configQ.data),
  });

  // Edited language list and default language; null while they show the saved value.
  const [languageDraft, setLanguageDraft] = useState<string[] | null>(null);
  const [defaultLanguageDraft, setDefaultLanguageDraft] = useState<string | null>(null);

  // A save's reply is the saved value: a read of the same settings sent before
  // that reply is cancelled, and the cancellation settles, before the reply is
  // stored, so the older read cannot replace it. The mutation stays pending,
  // and the page locked, until this handling finishes.
  const updateConfig = useMutation({
    mutationFn: api.updateMetadataConfig,
    onSuccess: async (saved) => {
      await qc.cancelQueries({ queryKey: ["metadataConfig"] });
      qc.setQueryData(["metadataConfig"], saved);
      reset(formValuesFrom(saved));
      setLanguageDraft(null);
      toast.success("Metadata configuration saved");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const updateDefaultLang = useMutation({
    mutationFn: api.updateDefaultLanguage,
    onSuccess: async (saved) => {
      await qc.cancelQueries({ queryKey: ["defaultLanguage"] });
      qc.setQueryData(["defaultLanguage"], saved);
      setDefaultLanguageDraft(null);
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const selectedProvider = watch("llmProvider");
  const providerConfig = selectedProvider ? PROVIDER_CONFIGS[selectedProvider] : null;

  // Custom model toggle — show text input when "other" is selected
  const [customModel, setCustomModel] = useState(false);
  const currentModel = watch("llmModel");

  // Check if current model matches any preset for the selected provider
  const isPresetModel =
    providerConfig?.models.some((m) => m.value === currentModel) ?? false;
  const showCustomModel =
    selectedProvider === "custom" || customModel || (!isPresetModel && currentModel !== "");

  const [langInput, setLangInput] = useState("");

  if (configQ.isLoading) return <PageLoading />;
  if (configQ.error)
    return (
      <ErrorState
        error={configQ.error as Error}
        onRetry={() => configQ.refetch()}
      />
    );

  const config = configQ.data!;

  const languages = languageDraft ?? config.languages;
  const languagesChanged = !sameLanguageList(languages, config.languages);
  const savedDefaultLanguage = defaultLangQ.data?.defaultLanguage;
  const defaultLanguage = defaultLanguageDraft ?? savedDefaultLanguage ?? "";
  const defaultLanguageChanged =
    defaultLanguageDraft !== null &&
    savedDefaultLanguage !== undefined &&
    defaultLanguageDraft !== savedDefaultLanguage;

  // Unsaved changes: any form value, the language list or the default
  // language differing from its saved value.
  const hasUnsavedChanges = isDirty || languagesChanged || defaultLanguageChanged;

  // Every settings writer is disabled from Save until both writes settle.
  const saving = isSubmitting || updateConfig.isPending || updateDefaultLang.isPending;

  const onSubmit = (data: MetadataForm) => {
    const req: UpdateMetadataConfigRequest = {
      hardcoverEnabled: data.hardcoverEnabled,
      llmEnabled: data.llmEnabled,
    };

    if (data.hardcoverApiToken)
      req.hardcoverApiToken = data.hardcoverApiToken;
    if (data.googleBooksApiKey)
      req.googleBooksApiKey = data.googleBooksApiKey;
    if (data.audnexusUrl !== config.audnexusUrl)
      req.audnexusUrl = data.audnexusUrl || null;
    if (data.llmProvider) req.llmProvider = data.llmProvider as LlmProvider;
    else if (config.llmProvider) req.llmProvider = null;
    if (data.llmEndpoint !== (config.llmEndpoint ?? ""))
      req.llmEndpoint = data.llmEndpoint || null;
    if (data.llmApiKey)
      req.llmApiKey = data.llmApiKey;
    if (data.llmModel !== (config.llmModel ?? ""))
      req.llmModel = data.llmModel || null;
    if (languagesChanged) req.languages = languages;

    updateConfig.mutate(req);

    if (defaultLanguageChanged && defaultLanguageDraft) {
      updateDefaultLang.mutate({ defaultLanguage: defaultLanguageDraft });
    }
  };

  const handleProviderChange = (newProvider: string) => {
    setValue("llmProvider", newProvider as LlmProvider | "", { shouldDirty: true });
    const cfg = PROVIDER_CONFIGS[newProvider];
    if (cfg) {
      if (cfg.endpoint) setValue("llmEndpoint", cfg.endpoint, { shouldDirty: true });
      if (cfg.models.length > 0) {
        setValue("llmModel", cfg.models[0]!.value, { shouldDirty: true });
        setCustomModel(false);
      } else {
        setValue("llmModel", "", { shouldDirty: true });
        setCustomModel(true);
      }
    }
  };

  const addLanguage = (code?: string) => {
    const val = code ?? langInput.trim().toLowerCase();
    if (val && !languages.includes(val)) {
      setLanguageDraft([...languages, val]);
    }
    setLangInput("");
  };

  const removeLanguage = (lang: string) => {
    setLanguageDraft(languages.filter((l) => l !== lang));
  };

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Metadata</h1>
      </PageToolbar>

      <PageContent>
        <form onSubmit={handleSubmit(onSubmit)} className="w-full max-w-xl space-y-8">
          {/* ── Hardcover ── */}
          <section data-tour="hardcover-section">
            <div className="flex items-center gap-2 mb-4">
              <BookOpen size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Hardcover
              </h2>
              <HelpTip text="Hardcover.app is a book metadata service with rich data (ratings, series info, covers). Get a free API token at hardcover.app → Settings → API. Optional but recommended for better metadata." />
              <TestButton
                run={api.testHardcover}
                successMessage="Hardcover connection successful"
                hasUnsavedChanges={hasUnsavedChanges}
              />
            </div>
            <label className="flex items-center gap-3 mb-4">
              <input
                type="checkbox"
                {...register("hardcoverEnabled")}
                disabled={saving}
                className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-brand disabled:opacity-50"
              />
              <span className="text-sm text-zinc-200">Enabled</span>
            </label>
            <div>
              <label className="block text-xs text-muted mb-1">
                API Token
              </label>
              <input
                {...register("hardcoverApiToken")}
                disabled={saving}
                type="password"
                placeholder={config.hardcoverApiTokenSet ? "Leave blank to keep saved token" : "Hardcover API token"}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
              />
            </div>
          </section>

          {/* ── Google Books ── */}
          <section data-tour="google-books-section">
            <div className="flex items-center gap-2 mb-4">
              <BookOpen size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Google Books
              </h2>
              <HelpTip text="Google Books provides structured metadata for books in all languages. Required for foreign-language enrichment without an LLM. Get an API key from console.cloud.google.com → APIs & Services → Credentials." />
            </div>
            <div>
              <label className="block text-xs text-muted mb-1">
                API Key
              </label>
              <input
                {...register("googleBooksApiKey")}
                disabled={saving}
                type="password"
                placeholder={config.googleBooksApiKeySet ? "Leave blank to keep saved key" : "Google Books API key"}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
              />
            </div>
          </section>

          {/* ── Audnexus ── */}
          <section>
            <div className="flex items-center gap-2 mb-4">
              <Globe size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Audnexus
              </h2>
              <TestButton
                run={api.testAudnexus}
                successMessage="Audnexus connection successful"
                hasUnsavedChanges={hasUnsavedChanges}
              />
            </div>
            <div>
              <label className="block text-xs text-muted mb-1">URL</label>
              <input
                {...register("audnexusUrl")}
                disabled={saving}
                placeholder="https://api.audnex.us"
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
              />
            </div>
          </section>

          {/* ── LLM ── */}
          <section data-tour="llm-section">
            <div className="flex items-center gap-2 mb-4">
              <Cpu size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                LLM Enrichment
              </h2>
              <HelpTip text="Optional. Uses an LLM to disambiguate search results and clean up author bibliographies. Livrarr only sends publicly available information (book titles, author names, publication years) — never file names, paths, or personal data. Both Groq and Gemini offer free tiers that are more than sufficient for typical use. Note: model names and pricing change frequently. If a model listed here stops working, check the provider's documentation for the latest available models." />
              <TestButton
                run={api.testLlm}
                successMessage="AI connection successful"
                hasUnsavedChanges={hasUnsavedChanges}
              />
            </div>
            <label className="flex items-center gap-3 mb-4">
              <input
                type="checkbox"
                {...register("llmEnabled")}
                disabled={saving}
                className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-brand disabled:opacity-50"
              />
              <span className="text-sm text-zinc-200">Enabled</span>
            </label>
            <div className="space-y-4">

              {/* Provider */}
              <div>
                <label className="block text-xs text-muted mb-1">
                  Provider
                </label>
                <Controller
                  name="llmProvider"
                  control={control}
                  render={({ field }) => (
                    <select
                      value={field.value}
                      onChange={(e) => handleProviderChange(e.target.value)}
                      disabled={saving}
                      className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                    >
                      <option value="">None</option>
                      {Object.entries(PROVIDER_CONFIGS).map(([key, cfg]) => (
                        <option key={key} value={key}>
                          {cfg.label}
                        </option>
                      ))}
                    </select>
                  )}
                />
              </div>

              {/* Provider-specific help */}
              {providerConfig && selectedProvider !== "custom" && (
                <div className="rounded border border-border/50 bg-zinc-800/30 p-3 text-xs text-zinc-400 space-y-1.5">
                  <p>{providerConfig.apiKeyHelp}</p>
                  {providerConfig.apiKeyUrl && (
                    <a
                      href={providerConfig.apiKeyUrl}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="inline-flex items-center gap-1 text-brand hover:text-brand-hover"
                    >
                      Get API key <ExternalLink size={10} />
                    </a>
                  )}
                  <p className="text-zinc-500">
                    Livrarr uses the OpenAI-compatible chat completions API. The pre-filled endpoint and model below should work, but providers update their offerings frequently. If something stops working, check the provider's documentation for current model names and endpoints.
                  </p>
                </div>
              )}

              {/* Endpoint */}
              {selectedProvider && (
                <div>
                  <label className="block text-xs text-muted mb-1">
                    Endpoint URL
                  </label>
                  <input
                    {...register("llmEndpoint")}
                    disabled={saving}
                    placeholder="https://api.example.com/v1"
                    className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                  />
                  {selectedProvider !== "custom" && (
                    <p className="mt-0.5 text-xs text-zinc-500">
                      Auto-filled for {providerConfig?.label}. Edit if your setup differs.
                    </p>
                  )}
                </div>
              )}

              {/* Model */}
              {selectedProvider && (
                <div>
                  <label className="block text-xs text-muted mb-1">
                    Model
                  </label>
                  {providerConfig && providerConfig.models.length > 0 && !showCustomModel ? (
                    <div className="space-y-2">
                      <Controller
                        name="llmModel"
                        control={control}
                        render={({ field }) => (
                          <select
                            value={field.value}
                            onChange={field.onChange}
                            disabled={saving}
                            className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                          >
                            {providerConfig.models.map((m) => (
                              <option key={m.value} value={m.value}>
                                {m.label}
                              </option>
                            ))}
                          </select>
                        )}
                      />
                      <button
                        type="button"
                        onClick={() => setCustomModel(true)}
                        disabled={saving}
                        className="text-xs text-zinc-500 hover:text-zinc-300 disabled:opacity-50"
                      >
                        Use a different model...
                      </button>
                    </div>
                  ) : (
                    <div className="space-y-2">
                      <input
                        {...register("llmModel")}
                        disabled={saving}
                        placeholder="model-name"
                        className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                      />
                      {providerConfig && providerConfig.models.length > 0 && (
                        <button
                          type="button"
                          onClick={() => {
                            setCustomModel(false);
                            setValue("llmModel", providerConfig.models[0]!.value, {
                              shouldDirty: true,
                            });
                          }}
                          disabled={saving}
                          className="text-xs text-zinc-500 hover:text-zinc-300 disabled:opacity-50"
                        >
                          Back to preset models...
                        </button>
                      )}
                    </div>
                  )}
                </div>
              )}

              {/* API Key */}
              {selectedProvider && (
                <div>
                  <label className="block text-xs text-muted mb-1">
                    API Key
                  </label>
                  <input
                    {...register("llmApiKey")}
                    disabled={saving}
                    type="password"
                    placeholder={config.llmApiKeySet ? "Leave blank to keep saved key" : "Paste your API key"}
                    className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
                  />
                </div>
              )}

              {/* Privacy note */}
              {selectedProvider && (
                <div className="rounded border border-zinc-700/50 bg-zinc-800/20 p-3 text-xs text-zinc-500">
                  <p className="font-medium text-zinc-400 mb-1">Privacy</p>
                  <p>
                    Livrarr only sends publicly available book metadata to the LLM provider: book titles, author names, and publication years. No file names, file paths, library structure, personal information, or usage data is ever transmitted. The same information is freely available on any bookstore or library website.
                  </p>
                </div>
              )}
            </div>
          </section>

          {/* ── Languages ── */}
          <section data-tour="languages-section">
            <div className="flex items-center gap-2 mb-4">
              <Languages size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Languages
              </h2>
            </div>
            <div className="mb-4">
              <div className="flex items-center gap-2 mb-1">
                <label className="block text-xs text-muted">
                  Default language for new books
                </label>
                <HelpTip text="Used when a book is added and its language cannot be determined — nothing in the file, the provider record, or your pick says what it is. Books that already declare a language are not affected, and neither are books already in your library." />
              </div>
              <select
                value={defaultLangQ.data ? defaultLanguage : ""}
                onChange={(e) => setDefaultLanguageDraft(e.target.value)}
                disabled={defaultLangQ.isLoading || !!defaultLangQ.error || saving}
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none disabled:opacity-50"
              >
                {defaultLangQ.data ? (
                  SUPPORTED_LANGUAGES.map((l) => (
                    <option key={l.code} value={l.code}>
                      {l.flag} {l.englishName}
                    </option>
                  ))
                ) : (
                  <option value="">
                    {defaultLangQ.error ? "Unavailable" : "Loading…"}
                  </option>
                )}
              </select>
              {!!defaultLangQ.error && (
                <p className="mt-0.5 text-xs text-red-400">
                  Failed to load the current default language.{" "}
                  <button
                    type="button"
                    className="underline"
                    onClick={() => defaultLangQ.refetch()}
                  >
                    Retry
                  </button>
                </p>
              )}
            </div>
            <p className="text-xs text-muted mb-4">
              Select languages for book metadata. The first enabled language is
              your primary &mdash; used by default when searching for new books.
            </p>
            <div className="rounded-lg border border-border overflow-hidden">
              {SUPPORTED_LANGUAGES.map((lang) => {
                const isEnabled = languages.includes(lang.code);
                const isEnglish = lang.code === "en";
                const isPrimary =
                  isEnglish && languages[0] === "en"
                    ? true
                    : languages[0] === lang.code;
                const llmConfigured =
                  config.llmEnabled &&
                  !!config.llmEndpoint &&
                  config.llmApiKeySet &&
                  !!config.llmModel;
                const googleBooksConfigured = !!config.googleBooksApiKeySet || !!watch("googleBooksApiKey")?.trim();
                const needsProvider = !isEnglish && !llmConfigured && !googleBooksConfigured;
                const providerError =
                  config.providerStatus?.[lang.providerName];

                return (
                  <div
                    key={lang.code}
                    className={`flex items-center gap-2 sm:gap-3 px-3 sm:px-4 py-3 border-b border-border last:border-b-0 transition-colors ${
                      isEnabled || isEnglish
                        ? "bg-zinc-800/50"
                        : "bg-zinc-800/20"
                    } ${needsProvider ? "opacity-60" : ""}`}
                  >
                    <span
                      className={`text-lg ${!isEnabled && !isEnglish ? "opacity-40" : ""}`}
                    >
                      {lang.flag}
                    </span>
                    <div className="flex flex-col flex-1 min-w-0">
                      <span
                        className={`text-sm font-medium ${isEnabled || isEnglish ? "text-zinc-100" : "text-zinc-500"}`}
                      >
                        {lang.englishName}
                        {isPrimary && (
                          <span className="ml-2 inline-block rounded bg-brand px-1.5 py-0.5 text-[9px] font-semibold text-white uppercase">
                            Primary
                          </span>
                        )}
                      </span>
                      <span
                        className={`text-[11px] ${isEnabled || isEnglish ? "text-zinc-500" : "text-zinc-600"}`}
                      >
                        {lang.providerName}
                      </span>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      <span
                        className={`text-[10px] font-semibold px-2 py-0.5 rounded ${
                          lang.providerType === "llm"
                            ? "bg-blue-500/15 text-blue-300"
                            : "bg-blue-500/15 text-blue-300"
                        }`}
                      >
                        {lang.providerType.toUpperCase()}
                      </span>
                      {providerError && (
                        <span className="text-[11px] text-red-400">
                          &#9679; Not Responding
                        </span>
                      )}
                      {needsProvider && (
                        <a
                          href="#llm-section"
                          className="text-[11px] text-red-400 underline"
                        >
                          Needs Google Books or LLM &rarr;
                        </a>
                      )}
                      {!isEnglish && (!needsProvider || isEnabled) && (
                        <button
                          type="button"
                          onClick={() => {
                            if (isEnabled) {
                              removeLanguage(lang.code);
                            } else {
                              addLanguage(lang.code);
                            }
                          }}
                          disabled={saving}
                          className={`w-9 h-5 rounded-full relative transition-colors disabled:opacity-50 ${
                            isEnabled ? "bg-brand" : "bg-zinc-600"
                          }`}
                        >
                          <span
                            className={`absolute top-0.5 w-4 h-4 rounded-full transition-all ${
                              isEnabled
                                ? "left-[18px] bg-white"
                                : "left-0.5 bg-zinc-400"
                            }`}
                          />
                        </button>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          </section>

          {/* ── Save ── */}
          <div className="pt-2">
            <button
              type="submit"
              disabled={
                isSubmitting || updateConfig.isPending || updateDefaultLang.isPending
              }
              className="rounded bg-brand px-6 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
            >
              {updateConfig.isPending || updateDefaultLang.isPending
                ? "Saving..."
                : "Save Changes"}
            </button>
          </div>
        </form>
      </PageContent>
    </>
  );
}
