import { useEffect, useState } from "react";
import { useParams, useSearchParams, useNavigate } from "react-router";
import { getWork } from "@/api";
import type { WorkDetailResponse } from "@/types/api";
import { AudioPlayer } from "./AudioPlayer";

export function ListenPage() {
  const { id } = useParams<{ id: string }>();
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const [work, setWork] = useState<WorkDetailResponse | null>(null);
  const [error, setError] = useState<string | null>(null);

  const workId = parseInt(searchParams.get("workId") ?? "0", 10);

  useEffect(() => {
    if (!workId) {
      setError("Missing work context");
      return;
    }
    getWork(workId)
      .then(setWork)
      .catch(() => setError("Failed to load work"));
  }, [workId]);

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

  if (!work) {
    return (
      <div className="flex h-screen items-center justify-center bg-zinc-900 text-zinc-400">
        Loading...
      </div>
    );
  }

  return (
    <AudioPlayer
      libraryItemId={parseInt(id!, 10)}
      workTitle={work.title}
      authorName={work.authorName}
      workId={work.id}
    />
  );
}

export default ListenPage;
