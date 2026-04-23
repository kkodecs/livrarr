import { Link, useParams } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { getSeriesDetail, updateSeries } from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { PageLoading } from "@/components/Page/LoadingSpinner";
import { ErrorState } from "@/components/Page/ErrorState";
import { MediaStatusRow } from "@/components/MediaStatusRow";
import { BookCover } from "@/components/BookCover";
import { cn } from "@/utils/cn";

export default function SeriesDetailPage() {
  const { id } = useParams<{ id: string }>();
  const queryClient = useQueryClient();
  const seriesId = Number(id);

  const { data, isLoading, error, refetch } = useQuery({
    queryKey: ["series-detail", id],
    queryFn: () => getSeriesDetail(seriesId),
    enabled: !isNaN(seriesId),
  });

  const updateMutation = useMutation({
    mutationFn: (req: { monitorEbook: boolean; monitorAudiobook: boolean }) =>
      updateSeries(seriesId, req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["series-detail", id] });
      queryClient.invalidateQueries({ queryKey: ["series-all"] });
    },
    onError: () => toast.error("Failed to update series"),
  });

  if (isLoading) return <PageLoading />;
  if (error) return <ErrorState error={error as Error} onRetry={refetch} />;
  if (!data) return <ErrorState error={new Error("Series not found")} />;

  const ebookCount = data.works.filter((w) =>
    w.libraryItems.some((li) => li.mediaType === "ebook"),
  ).length;
  const audioCount = data.works.filter((w) =>
    w.libraryItems.some((li) => li.mediaType === "audiobook"),
  ).length;
  const totalBooks = data.works.length;

  return (
    <>
      <PageToolbar>
        <div className="flex items-center gap-3">
          <div className="flex items-baseline gap-2">
            <h1 className="text-lg font-semibold text-zinc-100">
              {data.name}
            </h1>
            <span className="text-xs text-zinc-600">#{data.id}</span>
          </div>
        </div>
        <div className="flex items-center gap-2" />
      </PageToolbar>

      <PageContent>
        {/* Series header */}
        <div className="mb-6 flex flex-wrap items-center gap-4 text-sm text-muted">
          <Link
            to={`/author/${data.authorId}`}
            className="text-brand hover:underline"
          >
            {data.authorName}
          </Link>
          <span>
            {data.bookCount} {data.bookCount === 1 ? "book" : "books"}
          </span>
          <button
            onClick={() =>
              updateMutation.mutate({
                monitorEbook: !data.monitorEbook,
                monitorAudiobook: data.monitorAudiobook,
              })
            }
            disabled={updateMutation.isPending}
            className={cn(
              "inline-flex items-center gap-1.5 rounded border px-2.5 py-1 text-xs transition-colors",
              data.monitorEbook
                ? "border-green-700 bg-green-900/20 text-green-400 hover:bg-green-900/40"
                : "border-border text-zinc-500 hover:bg-surface-hover hover:text-zinc-300",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                data.monitorEbook ? "bg-green-500" : "bg-zinc-600",
              )}
            />
            Ebook {ebookCount}/{totalBooks}
          </button>
          <button
            onClick={() =>
              updateMutation.mutate({
                monitorEbook: data.monitorEbook,
                monitorAudiobook: !data.monitorAudiobook,
              })
            }
            disabled={updateMutation.isPending}
            className={cn(
              "inline-flex items-center gap-1.5 rounded border px-2.5 py-1 text-xs transition-colors",
              data.monitorAudiobook
                ? "border-green-700 bg-green-900/20 text-green-400 hover:bg-green-900/40"
                : "border-border text-zinc-500 hover:bg-surface-hover hover:text-zinc-300",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                data.monitorAudiobook ? "bg-green-500" : "bg-zinc-600",
              )}
            />
            Audiobook {audioCount}/{totalBooks}
          </button>
        </div>

        {/* Works list */}
        <div className="space-y-2">
          {data.works.map((work) => (
            <Link
              key={work.id}
              to={`/work/${work.id}`}
              className="flex items-center gap-3 sm:gap-4 rounded-lg border border-border bg-surface p-2 sm:p-3 hover:border-brand"
            >
              <BookCover
                workId={work.id}
                title={work.title}
                authorName={work.authorName}
                className="h-12 w-8 sm:h-16 sm:w-11"
                iconSize={14}
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  {work.seriesPosition != null && (
                    <span className="shrink-0 text-xs font-medium text-zinc-500">
                      #{work.seriesPosition}
                    </span>
                  )}
                  <p className="truncate font-medium text-sm sm:text-base text-zinc-100">
                    {work.title}
                  </p>
                </div>
                <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                  {work.year && <span>{work.year}</span>}
                  <MediaStatusRow work={work} />
                </div>
              </div>
            </Link>
          ))}
        </div>
      </PageContent>
    </>
  );
}
