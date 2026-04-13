import { useEffect, useState } from "react";
import { useParams, useNavigate } from "react-router";
import { getLibraryFile } from "@/api";
import type { LibraryItemResponse } from "@/types/api";
import { EpubReader } from "./EpubReader";
import { PdfReader } from "./PdfReader";

function getExtension(path: string): string {
  return path.split(".").pop()?.toLowerCase() ?? "";
}

export function ReaderPage() {
  const { id } = useParams<{ id: string }>();
  const navigate = useNavigate();
  const [item, setItem] = useState<LibraryItemResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!id) return;
    getLibraryFile(parseInt(id, 10))
      .then(setItem)
      .catch(() => setError("Failed to load file"));
  }, [id]);

  if (error) {
    return (
      <div className="flex h-screen items-center justify-center bg-zinc-900">
        <div className="text-center">
          <p className="text-red-400">{error}</p>
          <button
            onClick={() => navigate(-1)}
            className="mt-4 text-sm text-zinc-400 hover:text-zinc-100"
          >
            Go back
          </button>
        </div>
      </div>
    );
  }

  if (!item) {
    return (
      <div className="flex h-screen items-center justify-center bg-zinc-900 text-zinc-400">
        Loading...
      </div>
    );
  }

  const ext = getExtension(item.path);
  const itemId = parseInt(id!, 10);

  switch (ext) {
    case "epub":
      return <EpubReader libraryItemId={itemId} />;
    case "pdf":
      return <PdfReader libraryItemId={itemId} />;
    default:
      return (
        <div className="flex h-screen items-center justify-center bg-zinc-900">
          <div className="text-center">
            <p className="text-zinc-400">
              No reader available for .{ext} files
            </p>
            <button
              onClick={() => navigate(-1)}
              className="mt-4 text-sm text-zinc-400 hover:text-zinc-100"
            >
              Go back
            </button>
          </div>
        </div>
      );
  }
}

export default ReaderPage;
