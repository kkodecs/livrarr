import { useState, useEffect, useMemo, useRef } from "react";
import { useSearchParams, useNavigate, Link } from "react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Search, Plus, Loader2, ChevronDown, Check, X } from "lucide-react";
import { toast } from "sonner";
import { lookupWorks, addWork, listWorks, getMetadataConfig, fetchPreaddCovers } from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { EmptyState } from "@/components/Page/EmptyState";
import { BookCover } from "@/components/BookCover";
import { cn } from "@/utils/cn";
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
  const [pickerWork, setPickerWork] = useState<WorkSearchResult | null>(null);

  const { data: metaConfig } = useQuery({
    queryKey: ["metadata-config"],
    queryFn: getMetadataConfig,
  });

  useEffect(() => {
    if (urlLang) {
      setSelectedLang(urlLang);
    } else if (metaConfig) {
      setSelectedLang(metaConfig.languages[0] ?? "en");
    }
  }, [urlLang, metaConfig]);

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

  const { data: allWorks } = useQuery({
    queryKey: ["works"],
    queryFn: () => listWorks(),
    select: (res) => res.items,
  });

  const lowerQuery = query.toLowerCase();
  const libraryMatches = query
    ? (allWorks ?? []).filter(
        (w) =>
          w.title.toLowerCase().includes(lowerQuery) ||
          w.authorName.toLowerCase().includes(lowerQuery),
      )
    : [];

  const [showRaw, setShowRaw] = useState(false);

  const searchQuery = useQuery({
    queryKey: ["work-search", query, selectedLang, showRaw],
    queryFn: () => lookupWorks(query, selectedLang, showRaw),
    enabled: !!query,
  });

  const lookupResp = searchQuery.data ?? null;
  const olResults = lookupResp?.results ?? null;

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
    setPickerWork(null);
  };

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

          {isSearching && (
            <div className="flex items-center justify-center py-12">
              <Loader2 size={24} className="animate-spin text-muted" />
            </div>
          )}

          {selectedLang !== "en" && hasQuery && !isSearching && (
            <div className="mb-4 rounded-lg border border-blue-500/20 bg-blue-500/5 px-4 py-3 text-sm text-blue-300">
              <strong>Tip:</strong> Search by title for best results. Add author name if you get too many matches.
            </div>
          )}

          {/* Cover Picker */}
          {pickerWork && (
            <CoverPicker
              work={pickerWork}
              lang={selectedLang}
              onAdd={(coverUrl, coverManual) => {
                addWorkWithCover(pickerWork, coverUrl, coverManual);
              }}
              onCancel={() => setPickerWork(null)}
            />
          )}

          {!isSearching && hasOlResults && !pickerWork && (
            <section>
              <div className="flex items-center gap-3 mb-3">
                <h2 className="text-sm font-semibold uppercase tracking-wider text-muted">
                  Add to Your Library
                </h2>
                {lookupResp?.rawAvailable && (
                  <div className="flex items-center rounded border border-border text-xs">
                    <button
                      onClick={() => setShowRaw(false)}
                      className={cn(
                        "px-2 py-0.5 rounded-l",
                        !showRaw ? "bg-brand text-white" : "text-muted hover:text-zinc-100",
                      )}
                    >
                      Filtered {lookupResp.filteredCount}
                    </button>
                    <button
                      onClick={() => setShowRaw(true)}
                      className={cn(
                        "px-2 py-0.5 rounded-r",
                        showRaw ? "bg-brand text-white" : "text-muted hover:text-zinc-100",
                      )}
                    >
                      Raw {lookupResp.rawCount}
                    </button>
                  </div>
                )}
              </div>
              <div className="rounded border border-border">
                {filteredOlResults!.map((work, idx) => (
                  <OlResult
                    key={work.olKey ?? `${work.title}-${idx}`}
                    work={work}
                    onSelect={() => setPickerWork(work)}
                  />
                ))}
              </div>
            </section>
          )}

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

  function addWorkWithCover(
    work: WorkSearchResult,
    coverUrl: string | null,
    coverManual: boolean,
  ) {
    addWork({
      olKey: work.olKey,
      title: work.title,
      authorName: work.authorName,
      authorOlKey: work.authorOlKey,
      year: work.year,
      coverUrl,
      metadataSource: work.source,
      language: work.language,
      detailUrl: work.detailUrl,
      coverManual,
      isbn13: work.isbn13,
    })
      .then((data: AddWorkResponse) => {
        setPickerWork(null);
        queryClient.invalidateQueries({ queryKey: ["works"] });
        queryClient.invalidateQueries({ queryKey: ["authors"] });
        data.messages.forEach((msg) => toast.success(msg));
        navigate(`/work/${data.work.id}`);
      })
      .catch((err: Error) => {
        if (err instanceof ApiError && err.status === 409) {
          toast.error("Already in your library");
        } else {
          toast.error(err.message || "Failed to add work");
        }
      });
  }
}

function CoverPicker({
  work,
  lang,
  onAdd,
  onCancel,
}: {
  work: WorkSearchResult;
  lang: string;
  onAdd: (coverUrl: string | null, coverManual: boolean) => void;
  onCancel: () => void;
}) {
  const [selectedUrl, setSelectedUrl] = useState<string | null>(
    work.coverUrl ?? null,
  );
  const [isManual, setIsManual] = useState(!!work.coverUrl);

  const { data: alternatives, isLoading } = useQuery({
    queryKey: ["preadd-covers", work.title, work.authorName, lang],
    queryFn: () =>
      fetchPreaddCovers(work.title, work.authorName, lang, work.isbn13),
  });

  const allCovers = useMemo(() => {
    const covers: { url: string; source: string }[] = [];
    const seen = new Set<string>();
    if (work.coverUrl) {
      covers.push({ url: work.coverUrl, source: work.source ?? "search" });
      seen.add(work.coverUrl);
    }
    for (const alt of alternatives ?? []) {
      if (!seen.has(alt.proxyUrl)) {
        covers.push({ url: alt.proxyUrl, source: alt.source });
        seen.add(alt.proxyUrl);
      }
    }
    return covers;
  }, [work.coverUrl, work.source, alternatives]);

  return (
    <section className="rounded-lg border border-brand/30 bg-zinc-800/50 p-4">
      <div className="flex items-start justify-between mb-4">
        <div>
          <h3 className="text-base font-semibold text-zinc-100">
            {work.title}
          </h3>
          <p className="text-sm text-muted">{work.authorName}</p>
        </div>
        <button
          onClick={onCancel}
          className="p-1 rounded hover:bg-zinc-700 text-muted"
        >
          <X size={16} />
        </button>
      </div>

      <p className="text-xs text-muted mb-3">
        Select a cover, or skip to let enrichment find one later.
      </p>

      <div className="flex flex-wrap gap-3 mb-4">
        {allCovers.map((cover) => (
          <button
            key={cover.url}
            onClick={() => {
              setSelectedUrl(cover.url);
              setIsManual(true);
            }}
            className={cn(
              "relative rounded overflow-hidden border-2 transition-colors",
              selectedUrl === cover.url
                ? "border-brand"
                : "border-transparent hover:border-zinc-500",
            )}
          >
            <img
              src={cover.url}
              alt=""
              className="h-60 w-40 object-cover bg-zinc-700"
            />
            {selectedUrl === cover.url && (
              <div className="absolute inset-0 flex items-center justify-center bg-brand/20">
                <Check size={20} className="text-white" />
              </div>
            )}
            <div className="absolute bottom-0 inset-x-0 bg-black/60 text-[9px] text-zinc-300 text-center py-0.5 truncate">
              {cover.source}
            </div>
          </button>
        ))}

        {isLoading && (
          <div className="flex items-center justify-center h-60 w-40 rounded border border-dashed border-zinc-600">
            <Loader2 size={16} className="animate-spin text-muted" />
          </div>
        )}

        <button
          onClick={() => {
            setSelectedUrl(null);
            setIsManual(false);
          }}
          className={cn(
            "flex items-center justify-center h-60 w-40 rounded border-2 text-xs text-muted transition-colors",
            selectedUrl === null
              ? "border-brand bg-brand/10 text-brand"
              : "border-dashed border-zinc-600 hover:border-zinc-500",
          )}
        >
          Skip
        </button>
      </div>

      <button
        onClick={() => onAdd(selectedUrl, isManual)}
        className="inline-flex items-center gap-1.5 rounded bg-brand px-4 py-2 text-sm font-medium text-white hover:bg-brand-hover"
      >
        <Plus size={14} />
        Add to Library
      </button>
    </section>
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
        authorName={work.authorName}
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
  onSelect,
}: {
  work: WorkSearchResult;
  onSelect: () => void;
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
        <button
          type="button"
          onClick={onSelect}
          className="shrink-0 inline-flex items-center gap-1 rounded bg-brand px-2.5 py-1 text-xs font-medium text-white hover:bg-brand-hover"
        >
          <Plus size={12} />
          Select
        </button>
      </div>
    </div>
  );
}
