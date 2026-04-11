import { useState } from "react";
import {
  Library,
  Plus,
  Trash2,
  Globe,
  Lock,
  FolderOpen,
} from "lucide-react";
import { PageContent } from "@/components/Page/PageContent";
import { PageToolbar } from "@/components/Page/PageToolbar";

// ── MOCK DATA (red borders indicate hardcoded data) ──

const MOCK_ROOT_FOLDERS = [
  { id: 1, path: "/books/ebooks", mediaType: "ebook" as const },
  { id: 2, path: "/books/audiobooks", mediaType: "audiobook" as const },
  { id: 3, path: "/books/nonfiction", mediaType: "ebook" as const },
];

interface MockLibrary {
  id: number;
  name: string;
  rootFolderId: number;
  rootFolderPath: string;
  mediaType: "ebook" | "audiobook";
  workCount: number;
  fileCount: number;
  shared: boolean;
}

const INITIAL_LIBRARIES: MockLibrary[] = [
  {
    id: 1,
    name: "Pete's Fiction",
    rootFolderId: 1,
    rootFolderPath: "/books/ebooks",
    mediaType: "ebook",
    workCount: 28,
    fileCount: 34,
    shared: true,
  },
  {
    id: 2,
    name: "Pete's Audiobooks",
    rootFolderId: 2,
    rootFolderPath: "/books/audiobooks",
    mediaType: "audiobook",
    workCount: 18,
    fileCount: 22,
    shared: true,
  },
  {
    id: 3,
    name: "Pete's Non-Fiction",
    rootFolderId: 3,
    rootFolderPath: "/books/nonfiction",
    mediaType: "ebook",
    workCount: 7,
    fileCount: 7,
    shared: false,
  },
];

// ── Page ──

export default function LibrariesPage() {
  const [libraries, setLibraries] = useState(INITIAL_LIBRARIES);
  const [showCreate, setShowCreate] = useState(false);

  const toggleShared = (id: number) => {
    setLibraries((prev) =>
      prev.map((lib) =>
        lib.id === id ? { ...lib, shared: !lib.shared } : lib,
      ),
    );
  };

  return (
    <>
      <PageToolbar>
        <h1 className="flex items-center gap-2 text-lg font-semibold text-zinc-100">
          <Library size={20} />
          Libraries
        </h1>
        <button
          onClick={() => setShowCreate(!showCreate)}
          className="btn-primary inline-flex items-center gap-1.5 text-sm"
        >
          <Plus size={14} />
          New Library
        </button>
      </PageToolbar>

      <PageContent>
        {/* Create form */}
        {showCreate && (
          <div className="mb-6 rounded border-2 border-red-500 bg-zinc-800/50 p-4 space-y-3">
            <p className="text-xs font-medium text-red-400 uppercase tracking-wide">
              Mock — New Library Form
            </p>
            <div className="grid grid-cols-2 gap-4">
              <div>
                <label className="block text-xs text-muted mb-1">
                  Library Name
                </label>
                <input
                  placeholder="e.g., My Ebooks"
                  className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none"
                />
              </div>
              <div>
                <label className="block text-xs text-muted mb-1">
                  Root Folder
                </label>
                <select className="w-full rounded border border-border bg-zinc-900 px-3 py-2 text-sm text-zinc-100 focus:border-brand focus:outline-none">
                  {MOCK_ROOT_FOLDERS.map((rf) => (
                    <option key={rf.id} value={rf.id}>
                      {rf.path} ({rf.mediaType})
                    </option>
                  ))}
                </select>
              </div>
            </div>
            <div className="flex gap-2">
              <button className="btn-primary text-sm">Create Library</button>
              <button
                onClick={() => setShowCreate(false)}
                className="btn-secondary text-sm"
              >
                Cancel
              </button>
            </div>
          </div>
        )}

        {/* Library cards */}
        <div className="space-y-4">
          {libraries.map((lib) => (
            <div
              key={lib.id}
              className="rounded border-2 border-red-500 bg-zinc-800/50 p-4"
            >
              <div className="flex items-start justify-between">
                <div>
                  <h3 className="text-lg font-medium text-zinc-100 flex items-center gap-2">
                    <Library size={18} className="text-brand" />
                    {lib.name}
                  </h3>
                  <div className="mt-1 flex items-center gap-4 text-sm text-muted">
                    <span className="flex items-center gap-1">
                      <FolderOpen size={14} />
                      {lib.rootFolderPath}
                    </span>
                    <span
                      className={`inline-block rounded px-1.5 py-0.5 text-[10px] font-medium ${
                        lib.mediaType === "ebook"
                          ? "bg-blue-500/20 text-blue-300"
                          : "bg-purple-500/20 text-purple-300"
                      }`}
                    >
                      {lib.mediaType}
                    </span>
                    <span>
                      {lib.workCount} works · {lib.fileCount} files
                    </span>
                  </div>
                </div>
                <div className="flex items-center gap-3">
                  {/* Shared toggle */}
                  <button
                    onClick={() => toggleShared(lib.id)}
                    className={`inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-xs font-medium transition-colors ${
                      lib.shared
                        ? "bg-green-500/20 text-green-300 hover:bg-green-500/30"
                        : "bg-zinc-700 text-zinc-400 hover:bg-zinc-600"
                    }`}
                  >
                    {lib.shared ? (
                      <>
                        <Globe size={12} />
                        Shared
                      </>
                    ) : (
                      <>
                        <Lock size={12} />
                        Private
                      </>
                    )}
                  </button>
                  <button className="btn-secondary inline-flex items-center gap-1.5 text-xs text-red-400 hover:text-red-300">
                    <Trash2 size={12} />
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>

        {/* Explanation */}
        <div className="mt-6 rounded border-2 border-red-500 bg-zinc-800/30 p-3 text-xs text-muted space-y-1">
          <p className="text-red-400 font-medium uppercase tracking-wide">Mock — Info</p>
          <p>
            <Globe size={10} className="inline text-green-300 mr-1" />
            <strong>Shared</strong> — all authenticated users on this instance can browse this library (read-only).
          </p>
          <p>
            <Lock size={10} className="inline text-zinc-400 mr-1" />
            <strong>Private</strong> — only you can see this library.
          </p>
        </div>
      </PageContent>
    </>
  );
}
