import { useState } from "react";
import { useNavigate } from "react-router";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Search, Plus, Loader2, UserPlus } from "lucide-react";
import { toast } from "sonner";
import { lookupAuthors, addAuthor } from "@/api";
import { PageToolbar } from "@/components/Page/PageToolbar";
import { PageContent } from "@/components/Page/PageContent";
import { EmptyState } from "@/components/Page/EmptyState";
import type { AuthorSearchResult } from "@/types/api";
import { ApiError } from "@/api/client";

export default function AuthorSearchPage() {
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const [term, setTerm] = useState("");
  const [searchTerm, setSearchTerm] = useState("");

  const {
    data: results,
    isFetching: isSearching,
  } = useQuery({
    queryKey: ["authorSearch", searchTerm],
    queryFn: () => lookupAuthors(searchTerm),
    enabled: !!searchTerm,
  });

  const addMutation = useMutation({
    mutationFn: (author: AuthorSearchResult) =>
      addAuthor({
        name: author.name,
        sortName: author.sortName,
        olKey: author.olKey,
      }),
    onSuccess: (data) => {
      queryClient.invalidateQueries({ queryKey: ["authors"] });
      toast.success(`Added ${data.name}`);
      navigate(`/author/${data.id}`);
    },
    onError: (err: Error) => {
      if (err instanceof ApiError && err.status === 409) {
        toast.error("Author already in your library");
      } else {
        toast.error(err.message || "Failed to add author");
      }
    },
  });

  const handleSearch = (e: React.FormEvent) => {
    e.preventDefault();
    const q = term.trim();
    if (!q) return;
    setSearchTerm(q);
  };

  const hasSearched = !!searchTerm;

  return (
    <>
      <PageToolbar>
        <h1 className="text-lg font-semibold text-zinc-100">Add New Author</h1>
      </PageToolbar>

      <PageContent>
        <form onSubmit={handleSearch} className="flex gap-2">
          <div className="relative flex-1">
            <Search
              size={16}
              className="absolute left-3 top-1/2 -translate-y-1/2 text-muted"
            />
            <input
              type="text"
              value={term}
              onChange={(e) => setTerm(e.target.value)}
              placeholder="Search by author name..."
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

        <div className="mt-6">
          {isSearching && (
            <div className="flex items-center justify-center py-12">
              <Loader2 size={24} className="animate-spin text-muted" />
            </div>
          )}

          {!isSearching &&
            hasSearched &&
            results !== undefined &&
            results.length === 0 && (
              <EmptyState
                icon={<Search size={32} />}
                title="No results"
                description="Try a different search term."
              />
            )}

          {!isSearching && results && results.length > 0 && (
            <div className="space-y-2">
              {results.map((author) => (
                <div
                  key={author.olKey}
                  className="flex items-center justify-between gap-3 sm:gap-4 rounded-lg border border-border bg-surface p-3 sm:p-4"
                >
                  <div className="min-w-0">
                    <p className="font-medium text-zinc-100">{author.name}</p>
                    {author.sortName && author.sortName !== author.name && (
                      <p className="mt-0.5 text-sm text-muted">
                        {author.sortName}
                      </p>
                    )}
                  </div>
                  <button
                    onClick={() => addMutation.mutate(author)}
                    disabled={addMutation.isPending}
                    className="inline-flex shrink-0 items-center gap-1.5 rounded bg-brand px-3 py-1.5 text-sm font-medium text-white hover:bg-brand-hover disabled:opacity-50"
                  >
                    {addMutation.isPending ? (
                      <Loader2 size={14} className="animate-spin" />
                    ) : (
                      <Plus size={14} />
                    )}
                    Add
                  </button>
                </div>
              ))}
            </div>
          )}

          {!hasSearched && !isSearching && (
            <EmptyState
              icon={<UserPlus size={32} />}
              title="Search for an author"
              description="Results from Open Library will appear here."
            />
          )}
        </div>
      </PageContent>
    </>
  );
}
