import { useState, useEffect, useMemo, useRef } from "react";
import { useSearchParams, useNavigate, Link } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Search, Plus, Loader2, ChevronDown } from "lucide-react";
import { toast } from "sonner";
import { lookupWorks, addWork, listWorks, getMetadataConfig } from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { EmptyState } from "@/components/Page/EmptyState";
import { BookCover } from "@/components/BookCover";
import type {
  WorkSearchResult,
  AddWorkResponse,
  WorkDetailResponse,
} from "@/types/api";
import { SUPPORTED_LANGUAGES } from "@/types/api";
import { ApiError } from "@/api/client";

export default function SearchPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [searchParams, setSearchParams] = useSearchParams();
  const query = searchParams.get("q")?.trim() ?? "";
  const urlLang = searchParams.get("lang") ?? "";
  const [term, setTerm] = useState(query);
  const [selectedLang, setSelectedLang] = useState<string>(urlLang || "en");
  const [langOpen, setLangOpen] = useState(false);
  const langRef = useRef<HTMLDivElement>(null);

  // Load metadata config to get enabled languages
  const { data: metaConfig } = useQuery({
    queryKey: ["metadata-config"],
    queryFn: getMetadataConfig,
  });

  // Sync selectedLang from URL on every navigation (header search changes URL params)
  useEffect(() => {
    if (urlLang) {
      setSelectedLang(urlLang);
    } else if (metaConfig) {
      setSelectedLang(metaConfig.languages[0] ?? "en");
    }
  }, [urlLang, metaConfig]);

  // Click-outside handler for language dropdown
  useEffect(() => {
    if (!langOpen) return;
    const handler = (e: MouseEvent) => {
      if (langRef.current && !langRef.current.contains(e.target as Node)) {
        setLangOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [langOpen]);

  const enabledLanguages = useMemo(() => {
    const codes = metaConfig?.languages ?? ["en"];
    return SUPPORTED_LANGUAGES.filter((l) => codes.includes(l.code));
  }, [metaConfig]);

  // Local library data
  const { data: allWorks } = useQuery({
    queryKey: ["works"],
    queryFn: listWorks,
    select: (res) => res.items,
  });

  // Filter local works by search term
  const lowerQuery = query.toLowerCase();
  const libraryMatches = query
    ? (allWorks ?? []).filter(
        (w) =>
          w.title.toLowerCase().includes(lowerQuery) ||
          w.authorName.toLowerCase().includes(lowerQuery),
      )
    : [];

  // Work search — pass language
  const searchQuery = useQuery({
    queryKey: ["work-search", query, selectedLang],
    queryFn: () => lookupWorks(query, selectedLang),
    enabled: !!query,
  });

  const olResults = searchQuery.data ?? null;

  const [addingKey, setAddingKey] = useState<string | null>(null);
  const addMutation = useMutation({
    mutationFn: (work: WorkSearchResult) => {
      setAddingKey(work.olKey ?? `${work.title}-${work.year ?? ''}-${work.authorName}`);
      return addWork({
        olKey: work.olKey,
        title: work.title,
        authorName: work.authorName,
        authorOlKey: work.authorOlKey,
        year: work.year,
        coverUrl: work.coverUrl,
        metadataSource: work.source,
        language: work.language,
        detailUrl: work.detailUrl,
      });
    },
    onSuccess: (data: AddWorkResponse) => {
      setAddingKey(null);
      queryClient.invalidateQueries({ queryKey: ["works"] });
      queryClient.invalidateQueries({ queryKey: ["authors"] });
      data.messages.forEach((msg) => toast.success(msg));
      navigate(`/work/${data.work.id}`);
    },
    onError: (err: Error) => {
      setAddingKey(null);
      if (err instanceof ApiError && err.status === 409) {
        toast.error("Already in your library");
      } else {
        toast.error(err.message || "Failed to add work");
      }
    },
  });

  // Keep term input in sync when URL changes externally
  useEffect(() => {
    setTerm(query);
  }, [query]);

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    const q = term.trim();
    if (!q) return;
    const params: Record<string, string> = { q };
    if (selectedLang !== "en") params.lang = selectedLang;
    setSearchParams(params);
  };

  // Filter add-results to exclude works already in the library
  const libraryOlKeys = useMemo(
    () => new Set((allWorks ?? []).map((w) => w.olKey).filter(Boolean)),
    [allWorks],
  );
  const libraryTitleAuthor = useMemo(
    () =>
      new Set(
        (allWorks ?? []).map(
          (w) => `${w.title.toLowerCase()}|${w.authorName.toLowerCase()}`,
        ),
      ),
    [allWorks],
  );
  const filteredOlResults =
    olResults?.filter(
      (r) =>
        !(r.olKey && libraryOlKeys.has(r.olKey)) &&
        !libraryTitleAuthor.has(
          `${r.title.toLowerCase()}|${r.authorName.toLowerCase()}`,
        ),
    ) ?? null;

  const hasQuery = !!query;
  const hasLibraryResults = libraryMatches.length > 0;
  const hasOlResults =
    filteredOlResults !== null && filteredOlResults.length > 0;
  const isSearching = searchQuery.isFetching;
  const showNoResults =
    hasQuery &&
    !isSearching &&
    !hasLibraryResults &&
    filteredOlResults !== null &&
    filteredOlResults.length === 0;

  const currentLangInfo = SUPPORTED_LANGUAGES.find(
    (l) => l.code === selectedLang,
  );

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Search</h1>
      </PageToolbar>

      <PageContent>
        <form onSubmit={handleSearch} className="flex flex-col sm:flex-row gap-2">
          {/* Language selector */}
          {enabledLanguages.length > 1 && (
            <div className="relative" ref={langRef}>
              <button
                type="button"
                onClick={() => setLangOpen(!langOpen)}
                className="flex items-center gap-1.5 rounded border border-border bg-zinc-800 px-3 py-2 text-sm text-zinc-300 hover:border-zinc-500 whitespace-nowrap"
              >
                <span>{currentLangInfo?.flag}</span>
                <span className="text-zinc-400">{currentLangInfo?.englishName}</span>
                <ChevronDown size={12} className="text-zinc-500" />
              </button>
              {langOpen && (
                <div className="absolute top-full left-0 mt-1 z-10 min-w-[200px] rounded-lg border border-border bg-zinc-800 py-1 shadow-xl">
                  {enabledLanguages.map((lang) => (
                    <button
                      key={lang.code}
                      type="button"
                      onClick={() => {
                        setSelectedLang(lang.code);
                        setLangOpen(false);
                      }}
                      className={`flex items-center gap-2.5 w-full px-3 py-2 text-sm text-left hover:bg-blue-500/10 ${
                        selectedLang === lang.code ? "bg-blue-500/10" : ""
                      }`}
                    >
                      <span>{lang.flag}</span>
                      <div className="flex-1">
                        <div className="text-zinc-100">{lang.englishName}</div>
                        <div className="text-[10px] text-zinc-500">
                          {lang.providerName}
                        </div>
                      </div>
                      {selectedLang === lang.code && (
                        <span className="text-brand text-sm">&#10003;</span>
                      )}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
          <div className="relative flex-1">
            <Search
              size={16}
              className="absolute left-3 top-1/2 -translate-y-1/2 text-muted"
            />
            <input
              type="text"
              value={term}
              onChange={(e) => setTerm(e.target.value)}
              placeholder="Search by title, author, or ISBN..."
              className="w-full rounded border border-border bg-zinc-800 py-2 pl-9 pr-3 text-sm text-zinc-100 placeholder:text-muted focus:border-brand focus:outline-none"
              autoFocus
            />
          </div>
          <button
            type="submit"
            disabled={isSearching || !term.trim()}
            className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
          >
            {isSearching ? (
              <Loader2 size={14} className="animate-spin" />
            ) : (
              <Search size={14} />
            )}
            Search
          </button>
        </form>

        <div className="mt-6 space-y-8">
          {/* ── In Your Library ── */}
          {hasLibraryResults && (
            <section>
              <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted">
                In Your Library
              </h2>
              <div className="rounded border border-border">
                {libraryMatches.map((work) => (
                  <LibraryResult key={work.id} work={work} />
                ))}
              </div>
            </section>
          )}

          {/* ── Loading ── */}
          {isSearching && (
            <div className="flex items-center justify-center py-12">
              <Loader2 size={24} className="animate-spin text-muted" />
            </div>
          )}

          {/* ── SRP tip for non-English searches ── */}
          {selectedLang !== "en" && hasQuery && !isSearching && (
            <div className="mb-4 rounded-lg border border-blue-500/20 bg-blue-500/5 px-4 py-3 text-sm text-blue-300">
              <strong>Tip:</strong> Search by title for best results. Add author name if you get too many matches. For titles that are similar or identical in multiple languages, add the language to the search term.
            </div>
          )}

          {/* ── Add to Your Library ── */}
          {!isSearching && hasOlResults && (
            <section>
              <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-muted">
                Add to Your Library
              </h2>
              <div className="rounded border border-border">
                {filteredOlResults!.map((work, idx) => (
                  <OlResult
                    key={work.olKey ?? `${work.title}-${idx}`}
                    work={work}
                    onAdd={() => addMutation.mutate(work)}
                    isAdding={addingKey === (work.olKey ?? `${work.title}-${work.year ?? ''}-${work.authorName}`)}
                  />
                ))}
              </div>
            </section>
          )}

          {/* ── No Results ── */}
          {showNoResults && (
            <EmptyState
              icon={<Search size={32} />}
              title="No results"
              description="Try a different search term."
            />
          )}
        </div>
      </PageContent>
    </>
  );
}

function LibraryResult({ work }: { work: WorkDetailResponse }) {
  return (
    <Link
      to={`/work/${work.id}`}
      className="flex items-center gap-2 border-b border-border/50 px-2 py-2 sm:py-1.5 hover:bg-zinc-800/50"
    >
      <BookCover
        workId={work.id}
        title={work.title}
        className="h-8 w-6"
        iconSize={10}
      />
      <span className="min-w-0 truncate font-medium text-sm text-zinc-100">
        {work.title}
      </span>
      {work.seriesName && (
        <span className="hidden sm:inline shrink-0 text-xs text-zinc-500">
          {work.seriesName}
          {work.seriesPosition != null && ` #${work.seriesPosition}`}
        </span>
      )}
      <span className="flex-1" />
      <span className="shrink-0 text-xs text-muted">{work.authorName}</span>
      <span className="hidden sm:inline shrink-0 w-10 text-right text-xs text-zinc-500">
        {work.year ?? ""}
      </span>
    </Link>
  );
}

function OlResult({
  work,
  onAdd,
  isAdding,
}: {
  work: WorkSearchResult;
  onAdd: () => void;
  isAdding: boolean;
}) {
  return (
    <div className="flex flex-col sm:flex-row sm:items-center gap-2 border-b border-border/50 px-2 py-2 sm:py-1.5">
      <div className="flex items-center gap-2 min-w-0 flex-1">
        {work.coverUrl ? (
          <img
            src={work.coverUrl}
            alt=""
            className="h-8 w-6 shrink-0 rounded bg-zinc-700 object-cover"
          />
        ) : (
          <div className="flex h-8 w-6 shrink-0 items-center justify-center rounded bg-zinc-700 text-[8px] text-zinc-500">
            ?
          </div>
        )}
        <span className="min-w-0 truncate font-medium text-sm text-zinc-100">
          {work.title}
        </span>
        {work.seriesName && (
          <span className="hidden sm:inline shrink-0 text-xs text-zinc-500">
            {work.seriesName}
            {work.seriesPosition != null && ` #${work.seriesPosition}`}
          </span>
        )}
        {work.source && (
          <span className="shrink-0 text-[10px] px-1.5 py-0.5 rounded bg-blue-500/12 text-blue-300">
            {work.source}
          </span>
        )}
      </div>
      <div className="flex items-center gap-2 pl-8 sm:pl-0">
        <span className="shrink-0 text-xs text-muted">{work.authorName}</span>
        {work.rating && <span className="text-xs text-yellow-400">{work.rating} ★</span>}
        <span className="shrink-0 w-10 text-right text-xs text-zinc-500">
          {work.year ?? ""}
        </span>
        <button
          type="button"
          onClick={onAdd}
          disabled={isAdding}
          className="shrink-0 inline-flex items-center gap-1 rounded bg-brand px-2.5 py-1 text-xs font-medium text-white hover:bg-brand-hover disabled:opacity-50"
        >
          {isAdding ? (
            <Loader2 size={12} className="animate-spin" />
          ) : (
            <Plus size={12} />
          )}
          Add
        </button>
      </div>
    </div>
  );
}
