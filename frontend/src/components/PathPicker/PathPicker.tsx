import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Folder, ChevronUp, AlertTriangle } from "lucide-react";
import { browseFilesystem } from "@/api";

interface PathPickerProps {
  initialPath?: string;
  onSelect: (path: string) => void;
  onClose: () => void;
}

export function PathPicker({ initialPath = "/", onSelect, onClose }: PathPickerProps) {
  const [currentPath, setCurrentPath] = useState(initialPath);

  const { data, isLoading, isError } = useQuery({
    queryKey: ["filesystem", currentPath],
    queryFn: () => browseFilesystem(currentPath),
  });

  return (
    <div className="rounded border border-border bg-zinc-900 p-4">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-zinc-100">Select Directory</h3>
        <button
          onClick={onClose}
          className="text-xs text-muted hover:text-zinc-200"
        >
          Cancel
        </button>
      </div>

      <div className="mb-2 rounded bg-zinc-800 px-3 py-1.5 text-xs text-zinc-300 font-mono truncate">
        {currentPath}
      </div>

      <div className="max-h-64 overflow-y-auto space-y-0.5">
        {data?.parent && (
          <button
            onClick={() => data.parent && setCurrentPath(data.parent)}
            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-sm text-zinc-300 hover:bg-zinc-800"
          >
            <ChevronUp size={14} className="text-muted" />
            ..
          </button>
        )}

        {isLoading && (
          <p className="px-2 py-4 text-center text-xs text-muted">Loading...</p>
        )}

        {isError && (
          <div className="flex items-center gap-2 px-2 py-4 text-xs text-red-400">
            <AlertTriangle size={14} />
            Failed to browse this directory. Check permissions.
          </div>
        )}

        {!isLoading && !isError && data?.directories.map((dir) => (
          <button
            key={dir.path}
            onClick={() => setCurrentPath(dir.path)}
            className="flex w-full items-center gap-2 rounded px-2 py-1.5 text-sm text-zinc-300 hover:bg-zinc-800"
          >
            <Folder size={14} className="text-blue-400" />
            {dir.name}
          </button>
        ))}

        {!isLoading && !isError && data?.directories.length === 0 && (
          <p className="px-2 py-4 text-center text-xs text-muted">
            No subdirectories
          </p>
        )}
      </div>

      <div className="mt-3 flex justify-end gap-2">
        <button onClick={onClose} className="btn-secondary text-sm">
          Cancel
        </button>
        <button
          onClick={() => onSelect(currentPath)}
          className="btn-primary text-sm"
        >
          Select
        </button>
      </div>
    </div>
  );
}
