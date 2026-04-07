import { useMemo, useState } from "react";
import { Link } from "react-router";
import { useQuery } from "@tanstack/react-query";
import {
  Book,
  Headphones,
  Search,
  AlertCircle,
} from "lucide-react";
import { listWorks } from "@/api";
import { sortWorks } from "@/utils/works";
import type { WorkSortField } from "@/utils/works";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { EmptyState } from "@/components/Page/EmptyState";
import { SortHeader } from "@/components/Page/SortHeader";
import { cn } from "@/utils/cn";
import { formatRelativeDate, getCoverUrl } from "@/utils/format";
import type { WorkDetailResponse } from "@/types/api";

type MissingFilter = "all" | "ebook" | "audiobook";

function isMissingEbook(work: WorkDetailResponse): boolean {
  return !work.libraryItems?.some((li) => li.mediaType === "ebook");
}

function isMissingAudiobook(work: WorkDetailResponse): boolean {
  return !work.libraryItems?.some((li) => li.mediaType === "audiobook");
}

export default function MissingPage() {
  const {
    data: works,
    isLoading,
    error,
    refetch,
  } = useQuery({
    queryKey: ["works"],
    queryFn: listWorks,
  });

  const [filter, setFilter] = useState<MissingFilter>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [sortField, setSortField] = useState<WorkSortField>("title");
  const [sortDir, setSortDir] = useState<"asc" | "desc">("asc");

  const missing = useMemo(() => {
    if (!works) return [];
    let result = works.filter((w) => w.monitored);

    switch (filter) {
      case "ebook":
        result = result.filter(isMissingEbook);
        break;
      case "audiobook":
        result = result.filter(isMissingAudiobook);
        break;
      case "all":
        result = result.filter(
          (w) => isMissingEbook(w) || isMissingAudiobook(w),
        );
        break;
    }

    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      result = result.filter(
        (w) =>
          w.title.toLowerCase().includes(q) ||
          w.authorName.toLowerCase().includes(q),
      );
    }

    return sortWorks(result, sortField, sortDir);
  }, [works, filter, searchQuery, sortField, sortDir]);

  const handleSort = (field: WorkSortField) => {
    if (sortField === field) {
      setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    } else {
      setSortField(field);
      setSortDir("asc");
    }
  };

  // Counts for tab badges
  const counts = useMemo(() => {
    if (!works) return { all: 0, ebook: 0, audiobook: 0 };
    const monitored = works.filter((w) => w.monitored);
    return {
      all: monitored.filter(
        (w) => isMissingEbook(w) || isMissingAudiobook(w),
      ).length,
      ebook: monitored.filter(isMissingEbook).length,
      audiobook: monitored.filter(isMissingAudiobook).length,
    };
  }, [works]);

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error} onRetry={() => refetch()} />;

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-2">
          {(["all", "ebook", "audiobook"] as const).map((key) => {
            const labels = {
              all: "All Missing",
              ebook: "Ebooks",
              audiobook: "Audiobooks",
            };
            const icons = {
              all: AlertCircle,
              ebook: Book,
              audiobook: Headphones,
            };
            const Icon = icons[key];
            const count = counts[key];
            return (
              <button
                key={key}
                onClick={() => setFilter(key)}
                className={cn(
                  "inline-flex items-center gap-1.5 rounded px-3 py-1.5 text-sm",
                  filter === key
                    ? "bg-brand text-white"
                    : "text-zinc-400 hover:bg-zinc-700 hover:text-zinc-100",
                )}
              >
                <Icon size={14} />
                {labels[key]}
                <span
                  className={cn(
                    "ml-1 rounded-full px-1.5 py-0.5 text-xs",
                    filter === key
                      ? "bg-white/20 text-white"
                      : "bg-zinc-700 text-zinc-400",
                  )}
                >
                  {count}
                </span>
              </button>
            );
          })}
        </div>
        <div className="relative">
          <Search
            size={14}
            className="absolute left-2.5 top-1/2 -translate-y-1/2 text-muted"
          />
          <input
            type="text"
            placeholder="Filter..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className="h-8 rounded border border-border bg-zinc-800 pl-8 pr-3 text-sm text-zinc-100 placeholder:text-muted focus:border-brand focus:outline-none"
          />
        </div>
      </PageToolbar>

      <PageContent>
        {missing.length === 0 ? (
          <EmptyState
            icon={<AlertCircle size={32} />}
            title="No missing items"
            description={
              works?.length
                ? "All monitored works have their media files."
                : "Add works to start tracking missing media."
            }
          />
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="border-b border-border">
                <tr>
                  <th className="w-10 px-3 py-2" />
                  <SortHeader
                    field="title"
                    activeField={sortField}
                    dir={sortDir}
                    onSort={handleSort}
                  >
                    Title
                  </SortHeader>
                  <SortHeader
                    field="authorName"
                    activeField={sortField}
                    dir={sortDir}
                    onSort={handleSort}
                  >
                    Author
                  </SortHeader>
                  <SortHeader
                    field="year"
                    activeField={sortField}
                    dir={sortDir}
                    onSort={handleSort}
                  >
                    Year
                  </SortHeader>
                  <th className="px-3 py-2 text-left text-xs font-medium uppercase text-muted">
                    Missing
                  </th>
                  <SortHeader
                    field="addedAt"
                    activeField={sortField}
                    dir={sortDir}
                    onSort={handleSort}
                  >
                    Added
                  </SortHeader>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {missing.map((work) => (
                  <tr key={work.id} className="hover:bg-zinc-800/50">
                    <td className="px-3 py-2">
                      <img
                        src={getCoverUrl(work.id)}
                        alt=""
                        className="h-8 w-8 rounded object-cover"
                        loading="lazy"
                      />
                    </td>
                    <td className="px-3 py-2">
                      <Link
                        to={`/work/${work.id}`}
                        className="font-medium text-zinc-100 hover:text-brand"
                      >
                        {work.title}
                      </Link>
                    </td>
                    <td className="px-3 py-2 text-muted">
                      {work.authorId ? (
                        <Link
                          to={`/author/${work.authorId}`}
                          className="hover:text-brand"
                        >
                          {work.authorName}
                        </Link>
                      ) : (
                        work.authorName
                      )}
                    </td>
                    <td className="px-3 py-2 text-muted">
                      {work.year ?? "\u2014"}
                    </td>
                    <td className="px-3 py-2">
                      <MissingBadges work={work} />
                    </td>
                    <td className="px-3 py-2 text-muted">
                      {formatRelativeDate(work.addedAt)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </PageContent>
    </>
  );
}

function MissingBadges({ work }: { work: WorkDetailResponse }) {
  const missingEbook = isMissingEbook(work);
  const missingAudio = isMissingAudiobook(work);

  return (
    <div className="flex items-center gap-2 text-xs">
      {missingEbook && (
        <Link
          to={`/work/${work.id}?tab=releases`}
          className="inline-flex items-center gap-1 rounded bg-red-900/30 px-2 py-0.5 text-red-400 hover:bg-red-900/50 hover:text-red-300"
        >
          <Book size={12} />
          Ebook
        </Link>
      )}
      {missingAudio && (
        <Link
          to={`/work/${work.id}?tab=releases`}
          className="inline-flex items-center gap-1 rounded bg-orange-900/30 px-2 py-0.5 text-orange-400 hover:bg-orange-900/50 hover:text-orange-300"
        >
          <Headphones size={12} />
          Audiobook
        </Link>
      )}
    </div>
  );
}
