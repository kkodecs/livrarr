import { useState } from "react";
import { Library, BookOpen } from "lucide-react";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";

// ── MOCK DATA ──

interface MockWork {
  id: number;
  title: string;
  authorName: string;
  mediaType: "ebook" | "audiobook";
  owned: boolean;
  libraryName?: string;
  libraryOwner?: string;
}

const MOCK_COMBINED_WORKS: MockWork[] = [
  // Own works
  { id: 1, title: "The Man from the Future", authorName: "Ananyo Bhattacharya", mediaType: "ebook", owned: true },
  { id: 2, title: "A Game of Thrones", authorName: "George R.R. Martin", mediaType: "ebook", owned: true },
  { id: 3, title: "Ender's Game", authorName: "Orson Scott Card", mediaType: "audiobook", owned: true },
  // Shared works (from other users' libraries)
  { id: 101, title: "Project Hail Mary", authorName: "Andy Weir", mediaType: "ebook", owned: false, libraryName: "Sarah's Sci-Fi", libraryOwner: "sarah" },
  { id: 102, title: "The Martian", authorName: "Andy Weir", mediaType: "ebook", owned: false, libraryName: "Sarah's Sci-Fi", libraryOwner: "sarah" },
  { id: 103, title: "Dune", authorName: "Frank Herbert", mediaType: "audiobook", owned: false, libraryName: "Sarah's Sci-Fi", libraryOwner: "sarah" },
  { id: 104, title: "Atomic Habits", authorName: "James Clear", mediaType: "ebook", owned: false, libraryName: "Mike's Non-Fiction", libraryOwner: "mike" },
  { id: 105, title: "Deep Work", authorName: "Cal Newport", mediaType: "ebook", owned: false, libraryName: "Mike's Non-Fiction", libraryOwner: "mike" },
];

// ── Filter state ──

type LibraryFilter = "all" | "mine" | string;

// ── Page ──

export default function CombinedWorksPreview() {
  const [filter, setFilter] = useState<LibraryFilter>("all");

  const libraries = [
    { key: "mine", label: "My Library", count: MOCK_COMBINED_WORKS.filter((w) => w.owned).length },
    ...Object.entries(
      MOCK_COMBINED_WORKS.filter((w) => !w.owned).reduce(
        (acc, w) => {
          const key = w.libraryName!;
          acc[key] = (acc[key] || 0) + 1;
          return acc;
        },
        {} as Record<string, number>,
      ),
    ).map(([name, count]) => ({ key: name, label: name, count })),
  ];

  const filteredWorks =
    filter === "all"
      ? MOCK_COMBINED_WORKS
      : filter === "mine"
        ? MOCK_COMBINED_WORKS.filter((w) => w.owned)
        : MOCK_COMBINED_WORKS.filter((w) => w.libraryName === filter);

  return (
    <>
      <PageToolbar>
        <h1 className="flex items-center gap-2 text-lg font-semibold text-zinc-100">
          <BookOpen size={20} />
          Library (Combined View)
        </h1>
        <span className="text-xs text-red-400 font-medium uppercase">
          Mock Preview — Red borders = fake data
        </span>
      </PageToolbar>

      <PageContent>
        {/* Library filter tabs */}
        <div className="mb-4 flex items-center gap-2 border-2 border-red-500 rounded p-2">
          <button
            onClick={() => setFilter("all")}
            className={`rounded px-3 py-1 text-sm ${filter === "all" ? "bg-brand text-white" : "text-muted hover:text-zinc-100"}`}
          >
            All ({MOCK_COMBINED_WORKS.length})
          </button>
          {libraries.map((lib) => (
            <button
              key={lib.key}
              onClick={() => setFilter(lib.key)}
              className={`rounded px-3 py-1 text-sm ${filter === lib.key ? "bg-brand text-white" : "text-muted hover:text-zinc-100"}`}
            >
              {lib.label} ({lib.count})
            </button>
          ))}
        </div>

        {/* Works table */}
        <div className="border-2 border-red-500 rounded overflow-hidden">
          <table className="w-full text-sm">
            <thead>
              <tr className="border-b border-border bg-zinc-800/50">
                <th className="px-3 py-2 text-left text-xs text-muted w-10"></th>
                <th className="px-3 py-2 text-left text-xs text-muted">Title</th>
                <th className="px-3 py-2 text-left text-xs text-muted">Author</th>
                <th className="px-3 py-2 text-left text-xs text-muted">Format</th>
                <th className="px-3 py-2 text-left text-xs text-muted">Source</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border">
              {filteredWorks.map((work) => (
                <tr
                  key={`${work.owned ? "own" : "shared"}-${work.id}`}
                  className="hover:bg-zinc-800/50"
                >
                  <td className="px-3 py-2">
                    <div className="h-8 w-6 rounded bg-zinc-700 flex items-center justify-center text-[8px] text-zinc-500">
                      ?
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <span className="font-medium text-zinc-100">
                      {work.title}
                    </span>
                  </td>
                  <td className="px-3 py-2 text-muted">{work.authorName}</td>
                  <td className="px-3 py-2">
                    <span
                      className={`inline-block rounded px-1.5 py-0.5 text-[10px] font-medium ${
                        work.mediaType === "ebook"
                          ? "bg-blue-500/20 text-blue-300"
                          : "bg-purple-500/20 text-purple-300"
                      }`}
                    >
                      {work.mediaType}
                    </span>
                  </td>
                  <td className="px-3 py-2">
                    {work.owned ? (
                      <span className="text-xs text-muted">Mine</span>
                    ) : (
                      <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-medium text-amber-300">
                        <Library size={10} />
                        {work.libraryName}
                      </span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>

        {/* Legend */}
        <div className="mt-4 flex items-center gap-4 text-xs text-muted border-2 border-red-500 rounded p-2">
          <span>Mine = works you own and can edit/delete</span>
          <span className="inline-flex items-center gap-1">
            <span className="inline-flex items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-medium text-amber-300">
              <Library size={10} />
              Library Name
            </span>
            = shared with you (read-only)
          </span>
        </div>
      </PageContent>
    </>
  );
}

