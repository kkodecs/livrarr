import { HelpTip } from "@/components/HelpTip";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useForm, Controller } from "react-hook-form";
import { toast } from "sonner";
import { BookOpen, Cpu, Globe, Languages, X } from "lucide-react";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import type { LlmProvider, UpdateMetadataConfigRequest } from "@/types/api";
import * as api from "@/api";
import { useState } from "react";

interface MetadataForm {
  hardcoverEnabled: boolean;
  hardcoverApiToken: string;
  audnexusUrl: string;
  llmEnabled: boolean;
  llmProvider: LlmProvider | "";
  llmEndpoint: string;
  llmApiKey: string;
  llmModel: string;
}

export default function MetadataPage() {
  const qc = useQueryClient();

  const configQ = useQuery({
    queryKey: ["metadataConfig"],
    queryFn: api.getMetadataConfig,
  });

  const updateConfig = useMutation({
    mutationFn: api.updateMetadataConfig,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["metadataConfig"] });
      toast.success("Metadata configuration saved");
    },
    onError: (e: Error) => toast.error(e.message),
  });

  const {
    register,
    handleSubmit,
    control,
    formState: { isSubmitting },
  } = useForm<MetadataForm>({
    values: {
      hardcoverEnabled: configQ.data?.hardcoverEnabled ?? true,
      hardcoverApiToken: configQ.data?.hardcoverApiToken ?? "",
      audnexusUrl: configQ.data?.audnexusUrl ?? "",
      llmEnabled: configQ.data?.llmEnabled ?? true,
      llmProvider: configQ.data?.llmProvider ?? "",
      llmEndpoint: configQ.data?.llmEndpoint ?? "",
      llmApiKey: configQ.data?.llmApiKey ?? "",
      llmModel: configQ.data?.llmModel ?? "",
    },
  });

  // Languages editing
  const [languages, setLanguages] = useState<string[]>([]);
  const [langInput, setLangInput] = useState("");
  const [langDirty, setLangDirty] = useState(false);

  // Sync languages from query data on first load
  if (
    configQ.data &&
    !langDirty &&
    languages.length === 0 &&
    configQ.data.languages.length > 0
  ) {
    setLanguages(configQ.data.languages);
  }

  if (configQ.isLoading) return <PageLoading />;
  if (configQ.error)
    return (
      <ErrorState
        error={configQ.error as Error}
        onRetry={() => configQ.refetch()}
      />
    );

  const config = configQ.data!;

  const onSubmit = (data: MetadataForm) => {
    const req: UpdateMetadataConfigRequest = {
      hardcoverEnabled: data.hardcoverEnabled,
      llmEnabled: data.llmEnabled,
    };

    if (data.hardcoverApiToken !== (config.hardcoverApiToken ?? ""))
      req.hardcoverApiToken = data.hardcoverApiToken || null;
    if (data.audnexusUrl !== config.audnexusUrl)
      req.audnexusUrl = data.audnexusUrl || null;
    if (data.llmProvider) req.llmProvider = data.llmProvider as LlmProvider;
    else if (config.llmProvider) req.llmProvider = null;
    if (data.llmEndpoint !== (config.llmEndpoint ?? ""))
      req.llmEndpoint = data.llmEndpoint || null;
    if (data.llmApiKey !== (config.llmApiKey ?? ""))
      req.llmApiKey = data.llmApiKey || null;
    if (data.llmModel !== (config.llmModel ?? ""))
      req.llmModel = data.llmModel || null;
    if (langDirty) req.languages = languages;

    updateConfig.mutate(req);
  };

  const addLanguage = () => {
    const val = langInput.trim().toLowerCase();
    if (val && !languages.includes(val)) {
      setLanguages([...languages, val]);
      setLangDirty(true);
    }
    setLangInput("");
  };

  const removeLanguage = (lang: string) => {
    setLanguages(languages.filter((l) => l !== lang));
    setLangDirty(true);
  };

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Metadata</h1>
      </PageToolbar>

      <PageContent>
        <form onSubmit={handleSubmit(onSubmit)} className="max-w-xl space-y-8">
          {/* ── Hardcover ── */}
          <section>
            <div className="flex items-center gap-2 mb-4">
              <BookOpen size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Hardcover
              </h2>
              <HelpTip text="Hardcover.app is a book metadata service with rich data (ratings, series info, covers). Get a free API token at hardcover.app → Settings → API. Optional but recommended for better metadata." />
            </div>
            <label className="flex items-center gap-3 mb-4">
              <input
                type="checkbox"
                {...register("hardcoverEnabled")}
                className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-brand"
              />
              <span className="text-sm text-zinc-200">Enabled</span>
            </label>
            <div>
              <label className="block text-xs text-muted mb-1">
                API Token
              </label>
              <input
                {...register("hardcoverApiToken")}
                type="text"
                placeholder="Hardcover API token"
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-zinc-100 focus:border-brand focus:outline-none"
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
            </div>
            <div>
              <label className="block text-xs text-muted mb-1">URL</label>
              <input
                {...register("audnexusUrl")}
                placeholder="https://api.audnex.us"
                className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
            </div>
          </section>

          {/* ── LLM ── */}
          <section>
            <div className="flex items-center gap-2 mb-4">
              <Cpu size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                LLM Enrichment
              </h2>
              <HelpTip text="Optional. Uses an LLM to disambiguate search results and clean bibliographies. Only sends book titles and author names — no file names, no personal data." />
            </div>
            <label className="flex items-center gap-3 mb-4">
              <input
                type="checkbox"
                {...register("llmEnabled")}
                className="h-4 w-4 rounded border-zinc-600 bg-zinc-900 text-brand"
              />
              <span className="text-sm text-zinc-200">Enabled</span>
            </label>
            <div className="space-y-4">
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
                      onChange={field.onChange}
                      className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
                    >
                      <option value="">None</option>
                      <option value="groq">Groq</option>
                      <option value="gemini">Gemini</option>
                      <option value="openai">OpenAI</option>
                      <option value="custom">Custom</option>
                    </select>
                  )}
                />
              </div>
              <div>
                <label className="block text-xs text-muted mb-1">
                  Endpoint URL
                </label>
                <input
                  {...register("llmEndpoint")}
                  placeholder="https://api.groq.com/openai/v1"
                  className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs text-muted mb-1">
                  API Key
                </label>
                <input
                  {...register("llmApiKey")}
                  type="text"
                  placeholder="LLM API key"
                  className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm font-mono text-zinc-100 focus:border-brand focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs text-muted mb-1">
                  Model Name
                </label>
                <input
                  {...register("llmModel")}
                  placeholder="llama-3.3-70b-versatile"
                  className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
                />
              </div>
            </div>
          </section>

          {/* ── Languages ── */}
          <section>
            <div className="flex items-center gap-2 mb-4">
              <Languages size={18} className="text-muted" />
              <h2 className="text-base font-semibold text-zinc-100">
                Languages
              </h2>
            </div>
            <div className="flex flex-wrap gap-2 mb-3">
              {languages.map((lang) => (
                <span
                  key={lang}
                  className="inline-flex items-center gap-1 rounded-full bg-zinc-700 px-2.5 py-1 text-xs text-zinc-200"
                >
                  {lang}
                  <button
                    type="button"
                    onClick={() => removeLanguage(lang)}
                    className="text-muted hover:text-red-400"
                  >
                    <X size={12} />
                  </button>
                </span>
              ))}
              {languages.length === 0 && (
                <span className="text-xs text-muted">
                  No languages configured
                </span>
              )}
            </div>
            <div className="flex gap-2">
              <input
                type="text"
                value={langInput}
                onChange={(e) => setLangInput(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    addLanguage();
                  }
                }}
                placeholder="en"
                className="w-24 rounded border border-border bg-zinc-900 px-3 py-1.5 text-sm text-zinc-100 focus:border-brand focus:outline-none"
              />
              <button
                type="button"
                onClick={addLanguage}
                className="rounded border border-border px-3 py-1.5 text-xs text-zinc-200 hover:bg-zinc-700"
              >
                Add
              </button>
            </div>
          </section>

          {/* ── Save ── */}
          <div className="pt-2">
            <button
              type="submit"
              disabled={isSubmitting || updateConfig.isPending}
              className="rounded bg-brand px-6 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
            >
              {updateConfig.isPending ? "Saving..." : "Save Changes"}
            </button>
          </div>
        </form>
      </PageContent>
    </>
  );
}
