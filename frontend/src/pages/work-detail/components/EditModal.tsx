import { useForm } from "react-hook-form";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { updateWork } from "@/api";
import { FormModal } from "@/components/Page/FormModal";
import type { WorkDetailResponse, UpdateWorkRequest } from "@/types/api";
import { CoverSection } from "./CoverSection";

interface EditForm {
  title: string;
  authorName: string;
  seriesName: string;
  seriesPosition: string;
  monitorEbook: boolean;
  monitorAudiobook: boolean;
}

export function EditModal({
  work,
  open,
  onOpenChange,
  onCoverUploaded,
}: {
  work: WorkDetailResponse;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCoverUploaded?: () => void;
}) {
  const queryClient = useQueryClient();

  const {
    register,
    handleSubmit,
    formState: { isSubmitting },
  } = useForm<EditForm>({
    defaultValues: {
      title: work.title,
      authorName: work.authorName,
      seriesName: work.seriesName ?? "",
      seriesPosition:
        work.seriesPosition != null ? String(work.seriesPosition) : "",
      monitorEbook: work.monitorEbook,
      monitorAudiobook: work.monitorAudiobook,
    },
  });

  const updateMutation = useMutation({
    mutationFn: (req: UpdateWorkRequest) => updateWork(work.id, req),
    onSuccess: () => {
      toast.success("Work updated");
      queryClient.invalidateQueries({ queryKey: ["work", String(work.id)] });
      queryClient.invalidateQueries({ queryKey: ["works"] });
      onOpenChange(false);
    },
    onError: () => toast.error("Failed to update work"),
  });

  const onSubmit = (data: EditForm) => {
    const req: UpdateWorkRequest = {
      title: data.title || null,
      authorName: data.authorName || null,
      seriesName: data.seriesName || null,
      seriesPosition: data.seriesPosition ? Number(data.seriesPosition) : null,
      monitorEbook: data.monitorEbook,
      monitorAudiobook: data.monitorAudiobook,
    };
    updateMutation.mutate(req);
  };

  return (
    <FormModal open={open} onOpenChange={onOpenChange} title="Edit Work">
      <form onSubmit={handleSubmit(onSubmit)} className="space-y-4">
        <label className="block">
          <span className="mb-1 block text-sm font-medium text-zinc-300">
            Title
          </span>
          <input {...register("title")} className="input-field" />
        </label>
        <label className="block">
          <span className="mb-1 block text-sm font-medium text-zinc-300">
            Author
          </span>
          <input {...register("authorName")} className="input-field" />
        </label>
        <div className="grid grid-cols-2 gap-3">
          <label className="block">
            <span className="mb-1 block text-sm font-medium text-zinc-300">
              Series
            </span>
            <input {...register("seriesName")} className="input-field" />
          </label>
          <label className="block">
            <span className="mb-1 block text-sm font-medium text-zinc-300">
              Position
            </span>
            <input
              {...register("seriesPosition")}
              type="number"
              step="any"
              className="input-field"
            />
          </label>
        </div>

        <div className="flex gap-6">
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input
              type="checkbox"
              {...register("monitorEbook")}
              className="rounded border-border"
            />
            Monitor Ebook
          </label>
          <label className="flex items-center gap-2 text-sm text-zinc-200 cursor-pointer">
            <input
              type="checkbox"
              {...register("monitorAudiobook")}
              className="rounded border-border"
            />
            Monitor Audiobook
          </label>
        </div>

        <CoverSection
          work={work}
          onCoverUploaded={onCoverUploaded}
          onClose={() => onOpenChange(false)}
        />

        <div className="flex justify-end gap-3 pt-2">
          <button
            type="button"
            onClick={() => onOpenChange(false)}
            className="rounded px-4 py-2 text-sm text-muted hover:text-zinc-100"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={isSubmitting || updateMutation.isPending}
            className="btn-primary"
          >
            {updateMutation.isPending ? "Saving..." : "Save"}
          </button>
        </div>
      </form>
    </FormModal>
  );
}
